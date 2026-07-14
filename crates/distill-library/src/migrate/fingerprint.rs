//! Source DB and in-home content fingerprints for idempotent legacy import.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::LibraryResult;

/// Fingerprints identifying one legacy Electron home snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFingerprints {
    /// Combined fingerprint stored as the idempotency key.
    pub source_fingerprint: String,
    /// SHA-256 of `distill.db` file bytes.
    pub source_db_sha256: String,
    /// Fingerprint of regular in-home blob/export content files.
    pub content_fingerprint: String,
}

/**
 * Compute durable fingerprints over the legacy database and safe content files.
 *
 * Content fingerprint covers only regular files under `blobs/` and `exports/`,
 * never following symlinks and never leaving the source home.
 */
pub fn compute_source_fingerprints(source_home: &Path) -> LibraryResult<SourceFingerprints> {
    let db_path = source_home.join("distill.db");
    let source_db_sha256 = hex::encode(Sha256::digest(fs::read(&db_path)?));
    let content_fingerprint = fingerprint_content_tree(source_home)?;
    let journal_fingerprint = fingerprint_journal_sidecars(source_home)?;
    let mut combined = Sha256::new();
    combined.update(source_db_sha256.as_bytes());
    combined.update(b":");
    combined.update(journal_fingerprint.as_bytes());
    combined.update(b":");
    combined.update(content_fingerprint.as_bytes());
    Ok(SourceFingerprints {
        source_fingerprint: hex::encode(combined.finalize()),
        source_db_sha256,
        content_fingerprint,
    })
}

/** Hash the legacy database's WAL/SHM sidecars without opening them. */
fn fingerprint_journal_sidecars(source_home: &Path) -> LibraryResult<String> {
    let mut hasher = Sha256::new();
    for name in ["distill.db-wal", "distill.db-shm"] {
        hasher.update(name.as_bytes());
        hasher.update(b"\0");
        let path = source_home.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(crate::error::LibraryError::InvalidArgument(format!(
                    "legacy {name} must be a regular file"
                )));
            }
            Ok(_) => hasher.update(read_file_bytes(&path)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => hasher.update(b"missing"),
            Err(err) => return Err(err.into()),
        }
        hasher.update(b"\n");
    }
    Ok(hex::encode(hasher.finalize()))
}

/**
 * Walk `blobs/` and `exports/` for regular files and hash path+digest pairs.
 */
fn fingerprint_content_tree(source_home: &Path) -> LibraryResult<String> {
    let mut entries: Vec<(String, String)> = Vec::new();
    for dir_name in ["blobs", "exports"] {
        let root = source_home.join(dir_name);
        if !root.exists() {
            continue;
        }
        collect_regular_files(source_home, &root, &mut entries)?;
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, digest) in entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(digest.as_bytes());
        hasher.update(b"\n");
    }
    Ok(hex::encode(hasher.finalize()))
}

/**
 * Recursively collect relative path + sha256 for regular in-root files.
 */
fn collect_regular_files(
    source_home: &Path,
    dir: &Path,
    out: &mut Vec<(String, String)>,
) -> LibraryResult<()> {
    let read = match fs::read_dir(dir) {
        Ok(read) => read,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    for entry in read {
        let entry = entry?;
        let path = entry.path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect_regular_files(source_home, &path, out)?;
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        if !is_within_root(source_home, &path) {
            continue;
        }
        let rel = normalize_rel(source_home, &path);
        let digest = hex::encode(Sha256::digest(read_file_bytes(&path)?));
        out.push((rel, digest));
    }
    Ok(())
}

/**
 * Read file bytes for fingerprinting.
 */
fn read_file_bytes(path: &Path) -> LibraryResult<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/**
 * Return true when `path` stays under `root` without escaping.
 */
fn is_within_root(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

/**
 * Build a stable relative path string from the source home.
 */
fn normalize_rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/**
 * Resolve a legacy blob path to an absolute regular file under the source home.
 *
 * Rejects absolute escapes, parent traversal, symlinks, and non-files. Paths are
 * interpreted relative to `source_home/blobs` when not already under the home.
 */
pub fn resolve_in_root_regular_file(
    source_home: &Path,
    relative_or_home_path: &str,
    under_blobs: bool,
) -> LibraryResult<Option<PathBuf>> {
    let trimmed = relative_or_home_path.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.contains('\0') {
        return Ok(None);
    }
    let candidate = PathBuf::from(trimmed);
    if candidate
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Ok(None);
    }
    let absolute = if candidate.is_absolute() {
        // Absolute paths are only accepted when they canonicalize under source_home.
        candidate
    } else if under_blobs {
        source_home.join("blobs").join(&candidate)
    } else {
        source_home.join(&candidate)
    };

    let meta = match fs::symlink_metadata(&absolute) {
        Ok(meta) => meta,
        Err(_) => return Ok(None),
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Ok(None);
    }
    let canonical = match fs::canonicalize(&absolute) {
        Ok(path) => path,
        Err(_) => return Ok(None),
    };
    if !canonical.starts_with(source_home) {
        return Ok(None);
    }
    Ok(Some(canonical))
}
