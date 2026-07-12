//! Ordered checksummed schema migrations for a fresh Distill home.

use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::error::{LibraryError, LibraryResult};

/// Embedded migration scripts in apply order.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../../migrations/0001_initial.sql")),
    (
        2,
        include_str!("../../migrations/0002_source_prefs_sync_runs.sql"),
    ),
    (
        3,
        include_str!("../../migrations/0003_curation_read_models.sql"),
    ),
];

/**
 * Apply all pending checksummed migrations in order.
 *
 * Returns the latest applied migration version.
 */
pub fn migrate_to_latest(conn: &mut Connection) -> LibraryResult<i64> {
    ensure_migrations_table(conn)?;
    let mut latest = 0_i64;
    for &(version, sql) in MIGRATIONS {
        latest = version;
        let checksum = hex::encode(Sha256::digest(sql.as_bytes()));
        if let Some(existing) = applied_checksum(conn, version)? {
            if existing != checksum {
                return Err(LibraryError::MigrationChecksumMismatch { version });
            }
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        // schema_migrations row is created by 0001 itself for version bookkeeping of later
        // migrations; for v1 we insert explicitly after applying the DDL that creates the table.
        tx.execute(
            "INSERT INTO schema_migrations (version, checksum, applied_at) VALUES (?1, ?2, ?3)",
            (version, checksum, chrono::Utc::now().to_rfc3339()),
        )?;
        tx.commit()?;
    }
    Ok(latest)
}

/**
 * Ensure the migrations table exists before the first migration runs.
 *
 * The migrator bootstraps `schema_migrations` so checksum verification works before
 * and after applying versioned DDL scripts.
 */
fn ensure_migrations_table(conn: &Connection) -> LibraryResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            checksum TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/**
 * Read the stored checksum for an applied migration version.
 */
fn applied_checksum(conn: &Connection, version: i64) -> LibraryResult<Option<String>> {
    let value = conn
        .query_row(
            "SELECT checksum FROM schema_migrations WHERE version = ?1",
            [version],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(value)
}

/**
 * Verify every applied migration checksum still matches the embedded SQL.
 */
pub fn verify_migration_checksums(conn: &Connection) -> LibraryResult<()> {
    for &(version, sql) in MIGRATIONS {
        let expected = hex::encode(Sha256::digest(sql.as_bytes()));
        match applied_checksum(conn, version)? {
            Some(actual) if actual == expected => {}
            Some(_) => return Err(LibraryError::MigrationChecksumMismatch { version }),
            None => {
                return Err(LibraryError::InvalidArgument(format!(
                    "migration {version} is not applied"
                )));
            }
        }
    }
    Ok(())
}
