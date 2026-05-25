use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

pub fn open_read_write(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| {
        format!(
            "failed to open writable SQLite database at {}",
            path.display()
        )
    })?;
    configure_connection(&connection)?;
    Ok(connection)
}

pub fn open_read_only(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| {
        format!(
            "failed to open read-only SQLite database at {}",
            path.display()
        )
    })?;
    configure_connection(&connection)?;
    Ok(connection)
}

fn configure_connection(connection: &Connection) -> Result<()> {
    connection.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}
