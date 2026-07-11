//! Distill home layout and restrictive Unix permissions.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use rusqlite::Connection;

use super::configure_connection;
use crate::error::LibraryResult;

/// Well-known paths under a Distill home.
#[derive(Clone, Debug)]
pub struct DistillPaths {
    /// Root Distill home directory.
    pub home: PathBuf,
    /// SQLite database path.
    pub database: PathBuf,
    /// Content-addressed blob directory.
    pub blobs: PathBuf,
    /// Staging directory for atomic blob writes.
    pub staging: PathBuf,
}

impl DistillPaths {
    /// Resolve well-known paths for a Distill home.
    pub fn new(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        Self {
            database: home.join("distill.db"),
            blobs: home.join("blobs"),
            staging: home.join("staging"),
            home,
        }
    }
}

/**
 * Create the Distill home layout with restrictive Unix modes and open SQLite.
 *
 * Directories use mode `0o700`. The database file uses mode `0o600`.
 */
pub fn ensure_home_layout(home: &Path) -> LibraryResult<DistillPaths> {
    let paths = DistillPaths::new(home);
    create_dir_secure(&paths.home)?;
    create_dir_secure(&paths.blobs)?;
    create_dir_secure(&paths.staging)?;
    ensure_db_file(&paths.database)?;
    Ok(paths)
}

/**
 * Open a configured SQLite connection to the Distill database.
 */
pub fn open_connection(paths: &DistillPaths) -> LibraryResult<Connection> {
    let conn = Connection::open(&paths.database)?;
    configure_connection(&conn)?;
    Ok(conn)
}

/**
 * Create a directory with mode `0o700` on Unix.
 */
fn create_dir_secure(path: &Path) -> LibraryResult<()> {
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    #[cfg(unix)]
    {
        let perms = fs::Permissions::from_mode(0o700);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

/**
 * Ensure the database file exists with mode `0o600` on Unix.
 */
fn ensure_db_file(path: &Path) -> LibraryResult<()> {
    if !path.exists() {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        opts.mode(0o600);
        let _file = opts.open(path)?;
    }
    #[cfg(unix)]
    {
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

/**
 * Set mode `0o600` on a newly written file (Unix).
 */
#[cfg(unix)]
pub fn set_file_mode_600(path: &Path) -> LibraryResult<()> {
    let perms = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
pub fn set_file_mode_600(_path: &Path) -> LibraryResult<()> {
    Ok(())
}
