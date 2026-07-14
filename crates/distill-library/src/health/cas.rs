//! Safe CAS path validation and symlink-aware blob tree scanning.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::LibraryResult;

/**
 * True when every byte is an ASCII lowercase hex digit.
 */
pub(super) fn is_lowercase_hex(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/**
 * Canonical disposable staging partial: exactly 64 lowercase hex chars + `.partial`.
 */
pub(super) fn is_canonical_staging_partial_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".partial") else {
        return false;
    };
    stem.len() == 64 && is_lowercase_hex(stem)
}

/**
 * True when `relative` matches `blobs/<2 hex>/<62 hex>` with only lowercase hex.
 */
pub(super) fn is_canonical_blob_relative(relative: &str) -> bool {
    if relative.contains('\\') || relative.contains('\0') {
        return false;
    }
    let path = Path::new(relative);
    let components: Vec<_> = path.components().collect();
    if components.len() != 3 {
        return false;
    }
    match (&components[0], &components[1], &components[2]) {
        (Component::Normal(blobs), Component::Normal(prefix), Component::Normal(rest)) => {
            let blobs = blobs.to_str().unwrap_or_default();
            let prefix = prefix.to_str().unwrap_or_default();
            let rest = rest.to_str().unwrap_or_default();
            blobs == "blobs"
                && prefix.len() == 2
                && is_lowercase_hex(prefix)
                && rest.len() == 62
                && is_lowercase_hex(rest)
        }
        _ => false,
    }
}

/**
 * Outcome of safely resolving a Capture-referenced CAS path.
 */
pub(super) enum SafeCasOpen {
    /// Regular in-home canonical blob file.
    Regular(PathBuf),
    /// Path does not exist.
    Missing,
    /// Absolute, traversal, or non-canonical relative path.
    InvalidPath,
    /// Symlink or non-regular filesystem entry.
    SymlinkOrSpecial,
}

/**
 * Resolve a blob relative path for read/delete only when it is canonical and in-home.
 */
pub(super) fn open_safe_cas_file(home: &Path, relative: &str) -> SafeCasOpen {
    if !is_canonical_blob_relative(relative) {
        return SafeCasOpen::InvalidPath;
    }
    let absolute = home.join(relative);
    if !path_is_inside_home(home, &absolute) {
        return SafeCasOpen::InvalidPath;
    }
    // `symlink_metadata` on the final entry does not reveal symlinked parent
    // directories. Validate both fixed CAS ancestors before any read/delete.
    let relative_path = Path::new(relative);
    let mut components = relative_path.components();
    let Some(Component::Normal(blobs)) = components.next() else {
        return SafeCasOpen::InvalidPath;
    };
    let Some(Component::Normal(prefix)) = components.next() else {
        return SafeCasOpen::InvalidPath;
    };
    for ancestor in [home.join(blobs), home.join(blobs).join(prefix)] {
        let meta = match fs::symlink_metadata(&ancestor) {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return SafeCasOpen::Missing,
            Err(_) => return SafeCasOpen::SymlinkOrSpecial,
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return SafeCasOpen::SymlinkOrSpecial;
        }
    }
    let meta = match fs::symlink_metadata(&absolute) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return SafeCasOpen::Missing,
        Err(_) => return SafeCasOpen::SymlinkOrSpecial,
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return SafeCasOpen::SymlinkOrSpecial;
    }
    SafeCasOpen::Regular(absolute)
}

/**
 * Result of a CAS tree scan that never follows symlinks.
 */
pub(super) struct CasScan {
    /// Home-relative canonical regular files safe to treat as CAS blobs.
    pub regular_canonical: Vec<String>,
    /// Symlinks or malformed entries that block healthy CAS state.
    pub blocking_entries: u64,
}

/**
 * Walk the CAS tree using symlink metadata only; never follow links.
 */
pub(super) fn scan_cas_tree(home: &Path, blobs_dir: &Path) -> LibraryResult<CasScan> {
    let mut scan = CasScan {
        regular_canonical: Vec::new(),
        blocking_entries: 0,
    };
    if !blobs_dir.exists() {
        return Ok(scan);
    }
    let meta = fs::symlink_metadata(blobs_dir)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        scan.blocking_entries += 1;
        return Ok(scan);
    }
    collect_cas_entries(home, blobs_dir, &mut scan)?;
    scan.regular_canonical.sort();
    Ok(scan)
}

/**
 * Recursively collect CAS entries without following directory or file symlinks.
 */
fn collect_cas_entries(home: &Path, current: &Path, scan: &mut CasScan) -> LibraryResult<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => {
                scan.blocking_entries += 1;
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            scan.blocking_entries += 1;
            continue;
        }
        if meta.is_dir() {
            collect_cas_entries(home, &path, scan)?;
            continue;
        }
        if !meta.is_file() {
            scan.blocking_entries += 1;
            continue;
        }
        let Ok(relative) = path.strip_prefix(home) else {
            scan.blocking_entries += 1;
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if !is_canonical_blob_relative(&relative) || !path_is_inside_home(home, &path) {
            scan.blocking_entries += 1;
            continue;
        }
        scan.regular_canonical.push(relative);
    }
    Ok(())
}

/**
 * True when `path` resolves under `home` without leaving via `..` components.
 *
 * Uses lexical containment only — callers must already reject symlinks.
 */
fn path_is_inside_home(home: &Path, path: &Path) -> bool {
    let home = normalize_lexical(home);
    let path = normalize_lexical(path);
    path.starts_with(&home)
}

/**
 * Lexically normalize a path by dropping `.` and rejecting unresolved `..` escapes.
 */
fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}
