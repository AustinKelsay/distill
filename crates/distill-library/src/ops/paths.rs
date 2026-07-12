//! Canonical configured-root path policy for Source preferences and detection.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::{LibraryError, LibraryResult};

/**
 * Canonicalize a configured Source root.
 *
 * Rules:
 * - empty roots are rejected
 * - the path must exist and be a directory after symlink resolution
 * - lexical `..` escapes before existence are rejected
 * - the stored/returned path is the canonical absolute directory
 *
 * Symlinks are followed by platform canonicalize. Escaping the intended tree via
 * `..` components in a non-existent path is rejected before snapshot work.
 */
pub fn canonicalize_configured_root(root: &Path) -> LibraryResult<PathBuf> {
    if root.as_os_str().is_empty() {
        return Err(LibraryError::InvalidConfiguredRoot {
            detail: "configured root must not be empty".into(),
        });
    }
    if has_parent_escape(root) && !root.exists() {
        return Err(LibraryError::InvalidConfiguredRoot {
            detail: "configured root escapes via parent segments".into(),
        });
    }
    let meta = fs::symlink_metadata(root).map_err(|_| LibraryError::InvalidConfiguredRoot {
        detail: "configured root does not exist".into(),
    })?;
    if meta.file_type().is_symlink() {
        // Symlink roots are allowed when they resolve to an existing directory.
        // Callers store the canonical target, never the unresolved symlink path.
    }
    let canonical = fs::canonicalize(root).map_err(|_| LibraryError::InvalidConfiguredRoot {
        detail: "configured root could not be canonicalized".into(),
    })?;
    let canonical_meta =
        fs::metadata(&canonical).map_err(|_| LibraryError::InvalidConfiguredRoot {
            detail: "configured root metadata unavailable after canonicalize".into(),
        })?;
    if !canonical_meta.is_dir() {
        return Err(LibraryError::InvalidConfiguredRoot {
            detail: "configured root must be a directory".into(),
        });
    }
    Ok(canonical)
}

/**
 * True when a path lexically contains a parent-dir escape that could leave a base.
 */
fn has_parent_escape(path: &Path) -> bool {
    let mut depth = 0_i32;
    for component in path.components() {
        match component {
            Component::RootDir | Component::Prefix(_) | Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
        }
    }
    false
}
