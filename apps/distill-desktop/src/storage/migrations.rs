use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use super::schema::{ELECTRON_SCHEMA_SQL, LABEL_SEEDS, SCHEMA_MIGRATIONS_SQL, SOURCE_SEEDS};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchemaVersion(pub i64);

pub const PHASE1_SCHEMA_VERSION: SchemaVersion = SchemaVersion(1);

pub fn migrate_to_latest(connection: &mut Connection) -> Result<SchemaVersion> {
    connection.execute_batch(SCHEMA_MIGRATIONS_SQL)?;

    let already_applied = connection
        .query_row(
            "SELECT version FROM schema_migrations WHERE version = ?1",
            [PHASE1_SCHEMA_VERSION.0],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;

    if already_applied.is_some() {
        return Ok(PHASE1_SCHEMA_VERSION);
    }

    let transaction = connection.transaction()?;
    transaction.execute_batch(ELECTRON_SCHEMA_SQL)?;

    for (kind, display_name) in SOURCE_SEEDS {
        transaction.execute(
            "INSERT OR IGNORE INTO sources (kind, display_name, install_status) VALUES (?1, ?2, 'unknown')",
            (kind, display_name),
        )?;
    }

    for label in LABEL_SEEDS {
        transaction.execute(
            "INSERT OR IGNORE INTO labels (name, scope) VALUES (?1, 'session')",
            [label],
        )?;
    }

    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version) VALUES (?1)",
        [PHASE1_SCHEMA_VERSION.0],
    )?;
    transaction.commit()?;

    Ok(PHASE1_SCHEMA_VERSION)
}
