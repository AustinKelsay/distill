//! Library-owned storage: Distill home, migrations, content store, and SQLite helpers.

mod content;
mod home;
mod migrations;

pub use content::{read_capture_bytes, store_capture_bytes, ContentRef};
pub use home::{ensure_home_layout, open_connection, DistillPaths};
pub use migrations::{migrate_to_latest, verify_migration_checksums};

use rusqlite::Connection;

use crate::error::LibraryResult;

/**
 * Enable SQLite foreign keys and WAL for a Library connection.
 */
pub fn configure_connection(conn: &Connection) -> LibraryResult<()> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;",
    )?;
    Ok(())
}
