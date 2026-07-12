//! Library-owned storage: Distill home, migrations, content store, and SQLite helpers.

mod content;
mod home;
mod migrations;

pub use content::{read_capture_bytes, store_capture_bytes, ContentRef};
pub use home::{ensure_home_layout, open_connection, set_file_mode_600, DistillPaths};
pub use migrations::{migrate_to_latest, verify_migration_checksums};

use rusqlite::Connection;
use std::time::Duration;

use crate::error::LibraryResult;

/**
 * Enable SQLite foreign keys and WAL for a Library connection.
 */
pub fn configure_connection(conn: &Connection) -> LibraryResult<()> {
    // Keep background lease refreshes bounded while another connection owns a
    // short SQLite write transaction. The heartbeat retries after contention.
    conn.busy_timeout(Duration::from_millis(500))?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;",
    )?;
    Ok(())
}
