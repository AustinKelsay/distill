//! Path relationship checks for legacy Electron import.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use tempfile::TempDir;

use crate::error::{LibraryError, LibraryResult};

/**
 * Canonicalize and reject missing, same, aliased, or ancestor/descendant homes.
 *
 * Destination must already exist as a native Distill home directory. Source must
 * exist and contain `distill.db`. Neither path may be a symlink itself after
 * canonicalize; same-path, device/inode aliasing, and ancestor relationships fail.
 *
 * Parameters:
 * - `source_home`: legacy Electron Distill home
 * - `destination_home`: native Library Distill home
 */
pub fn validate_legacy_import_paths(
    source_home: &Path,
    destination_home: &Path,
) -> LibraryResult<(PathBuf, PathBuf)> {
    let source = canonicalize_existing_dir(source_home, "legacy source home")?;
    let destination = canonicalize_existing_dir(destination_home, "destination Distill home")?;

    if source == destination {
        return Err(LibraryError::InvalidArgument(
            "legacy source home and destination Distill home must be different paths".into(),
        ));
    }

    if is_path_alias(&source, &destination)? {
        return Err(LibraryError::InvalidArgument(
            "legacy source home and destination Distill home resolve to the same filesystem identity"
                .into(),
        ));
    }

    if destination.starts_with(&source) || source.starts_with(&destination) {
        return Err(LibraryError::InvalidArgument(
            "legacy source home and destination Distill home must not be ancestor or descendant"
                .into(),
        ));
    }

    let source_db = source.join("distill.db");
    match fs::symlink_metadata(&source_db) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(LibraryError::InvalidArgument(
                "legacy distill.db is missing or is not a regular file".into(),
            ));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(LibraryError::NotFound(
                "legacy distill.db was not found under the source home".into(),
            ));
        }
        Err(err) => return Err(err.into()),
    }

    Ok((source, destination))
}

/**
 * Open the legacy SQLite database with read-only flags and query_only.
 *
 * Never enables WAL or other write PRAGMAs against the source.
 */
pub fn open_legacy_readonly(source_home: &Path) -> LibraryResult<Connection> {
    let db_path = source_home.join("distill.db");
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(db_path, flags)?;
    conn.busy_timeout(std::time::Duration::from_millis(500))?;
    conn.execute_batch(
        "PRAGMA query_only = ON;
         PRAGMA foreign_keys = ON;",
    )?;
    Ok(conn)
}

/**
 * Copy the legacy SQLite database and any WAL sidecars into a private snapshot.
 *
 * The live Electron home is never opened by SQLite. This matters for WAL homes:
 * even a query-only connection can need to create or update a `-shm` file. The
 * snapshot lives under the destination staging directory and is removed when
 * the returned guard is dropped.
 */
pub fn snapshot_legacy_database(source_home: &Path, staging: &Path) -> LibraryResult<TempDir> {
    let snapshot = TempDir::new_in(staging)?;
    for name in ["distill.db", "distill.db-wal", "distill.db-shm"] {
        let source = source_home.join(name);
        let metadata = match fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(LibraryError::InvalidArgument(format!(
                "legacy {name} must be a regular file"
            )));
        }
        fs::copy(&source, snapshot.path().join(name))?;
    }
    Ok(snapshot)
}

/**
 * Canonicalize an existing directory, rejecting missing or non-directory paths.
 */
fn canonicalize_existing_dir(path: &Path, label: &str) -> LibraryResult<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(LibraryError::InvalidArgument(format!(
            "{label} path must not be empty"
        )));
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(LibraryError::InvalidArgument(format!(
                "{label} must not be a symlink"
            )));
        }
        Ok(meta) if !meta.is_dir() => {
            return Err(LibraryError::InvalidArgument(format!(
                "{label} must be a directory"
            )));
        }
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(LibraryError::NotFound(format!("{label} does not exist")));
        }
        Err(err) => return Err(err.into()),
    }
    Ok(fs::canonicalize(path)?)
}

/**
 * Detect same-device/inode aliasing between two canonical paths.
 */
fn is_path_alias(left: &Path, right: &Path) -> LibraryResult<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let left_meta = fs::metadata(left)?;
        let right_meta = fs::metadata(right)?;
        Ok(left_meta.dev() == right_meta.dev() && left_meta.ino() == right_meta.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = (left, right);
        Ok(false)
    }
}
