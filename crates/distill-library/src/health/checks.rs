//! Health classification checks over schema, FTS, staging, CAS, and incomplete state.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use rusqlite::Connection;

use super::cas::{is_canonical_staging_partial_name, scan_cas_tree};
use crate::error::LibraryResult;
use crate::storage::verify_migration_checksums;
use crate::types::HealthIssue;

/**
 * Schema health: migration checksums plus SQLite quick/integrity/foreign-key checks.
 *
 * Failures use stable generic redacted summaries — never raw DB/path/SQL text.
 */
pub(super) fn check_schema_integrity(
    conn: &Connection,
    issues: &mut Vec<HealthIssue>,
) -> LibraryResult<String> {
    let mut status = "ok".to_string();

    if verify_migration_checksums(conn).is_err() {
        push_schema_issue(
            issues,
            &mut status,
            "migration_integrity",
            "migration checksum verification failed",
        );
    }

    let quick: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick != "ok" {
        push_schema_issue(
            issues,
            &mut status,
            "sqlite_quick_check",
            "sqlite quick check failed",
        );
    }

    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        push_schema_issue(
            issues,
            &mut status,
            "sqlite_integrity_check",
            "sqlite integrity check failed",
        );
    }

    let mut fk_stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let fk_rows = fk_stmt.query_map([], |_| Ok(()))?;
    let mut fk_violations = 0_u64;
    for row in fk_rows {
        row?;
        fk_violations += 1;
    }
    if fk_violations > 0 {
        push_schema_issue(
            issues,
            &mut status,
            "sqlite_foreign_key_check",
            "sqlite foreign key check failed",
        );
    }

    Ok(status)
}

/**
 * Require exact projection_messages ↔ FTS identity and searchable-field agreement.
 *
 * Every searchable persisted field is compared: session_id, message_id, title,
 * project_path, role, and text. Count-only equality is insufficient.
 */
pub(super) fn check_fts_projection_agreement(
    conn: &Connection,
    issues: &mut Vec<HealthIssue>,
) -> LibraryResult<String> {
    let mut status = "ok".to_string();
    let mut message_stmt = conn.prepare(
        "SELECT pm.id, pm.session_id,
                COALESCE(s.title, ''), COALESCE(s.project_path, ''),
                pm.role, pm.text
         FROM projection_messages pm
         JOIN sessions s ON s.id = pm.session_id
         WHERE pm.projection_generation = s.successful_projection_generation
         ORDER BY pm.id ASC",
    )?;
    let message_rows = message_stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    let mut expected = Vec::new();
    for row in message_rows {
        expected.push(row?);
    }

    let mut fts_stmt = conn.prepare(
        "SELECT message_id, session_id, title, project_path, role, text
         FROM projection_fts
         ORDER BY message_id ASC",
    )?;
    let fts_rows = fts_stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    let mut actual = Vec::new();
    for row in fts_rows {
        actual.push(row?);
    }

    if expected != actual {
        status = "failed".to_string();
        issues.push(HealthIssue {
            code: "fts_projection_mismatch".into(),
            severity: "repairable".into(),
            category: "fts".into(),
            summary: format!(
                "projection and FTS disagree: {} projection message(s), {} FTS row(s)",
                expected.len(),
                actual.len()
            ),
        });
    }
    Ok(status)
}

/**
 * Report remaining canonical staging partials and unrecognized staging entries.
 */
pub(super) fn check_staging_partials(
    staging: &Path,
    issues: &mut Vec<HealthIssue>,
) -> LibraryResult<String> {
    let mut canonical = 0_u64;
    let mut unrecognized = 0_u64;
    let safe_staging_dir = match fs::symlink_metadata(staging) {
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_dir() => true,
        Ok(_) => {
            issues.push(HealthIssue {
                code: "unsafe_staging_root".into(),
                severity: "blocking".into(),
                category: "staging".into(),
                summary: "staging root is a symlink or non-directory entry".into(),
            });
            false
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => return Err(err.into()),
    };
    if safe_staging_dir {
        for entry in fs::read_dir(staging)? {
            let entry = entry?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                unrecognized += 1;
                continue;
            };
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(_) => {
                    unrecognized += 1;
                    continue;
                }
            };
            if meta.file_type().is_symlink() || meta.file_type().is_dir() {
                unrecognized += 1;
                continue;
            }
            if !meta.file_type().is_file() {
                unrecognized += 1;
                continue;
            }
            if is_canonical_staging_partial_name(name) {
                canonical += 1;
            } else {
                unrecognized += 1;
            }
        }
    }

    let mut status = "ok".to_string();
    if canonical > 0 {
        status = "failed".to_string();
        issues.push(HealthIssue {
            code: "staging_partial".into(),
            severity: "repairable".into(),
            category: "staging".into(),
            summary: format!("{canonical} disposable staging partial(s) present"),
        });
    }
    if unrecognized > 0 {
        status = "failed".to_string();
        issues.push(HealthIssue {
            code: "unrecognized_staging_entry".into(),
            severity: "blocking".into(),
            category: "staging".into(),
            summary: format!("{unrecognized} unrecognized staging entr(y/ies) present"),
        });
    }
    Ok(status)
}

/**
 * Report CAS issues: unreferenced regular blobs, plus blocking symlink/malformed entries.
 */
pub(super) fn check_orphan_blobs(
    conn: &Connection,
    home: &Path,
    blobs_dir: &Path,
    issues: &mut Vec<HealthIssue>,
) -> LibraryResult<String> {
    let referenced = referenced_blob_paths(conn)?;
    let scan = scan_cas_tree(home, blobs_dir)?;
    let mut status = "ok".to_string();

    if scan.blocking_entries > 0 {
        status = "failed".to_string();
        issues.push(HealthIssue {
            code: "cas_unrecognized_entry".into(),
            severity: "blocking".into(),
            category: "orphan".into(),
            summary: format!(
                "{} unrecognized or unsafe CAS entr(y/ies) present",
                scan.blocking_entries
            ),
        });
    }

    let orphans = scan
        .regular_canonical
        .into_iter()
        .filter(|path| !referenced.contains(path))
        .count();
    if orphans > 0 {
        status = "failed".to_string();
        issues.push(HealthIssue {
            code: "orphan_blob".into(),
            severity: "repairable".into(),
            category: "orphan".into(),
            summary: format!("{orphans} unreferenced CAS blob(s) present"),
        });
    }
    Ok(status)
}

/**
 * Report incomplete Captures, pending Attempts, projection linkage faults, and
 * mismatched Session counters.
 *
 * Empty successful projections are valid when `current_attempt_id` points at a
 * succeeded Attempt for the same `projection_generation`.
 */
pub(super) fn check_incomplete_state(
    conn: &Connection,
    issues: &mut Vec<HealthIssue>,
) -> LibraryResult<String> {
    let mut status = "ok".to_string();

    let unresolved_captures: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captures c
         WHERE NOT EXISTS (
           SELECT 1 FROM normalization_attempts na WHERE na.capture_id = c.id
         )
         AND NOT EXISTS (
           SELECT 1 FROM activity_events ae
           WHERE ae.capture_id = c.id AND ae.event_type = 'capture_failed'
         )",
        [],
        |row| row.get(0),
    )?;
    if unresolved_captures > 0 {
        status = "failed".to_string();
        issues.push(HealthIssue {
            code: "incomplete_capture".into(),
            severity: "repairable".into(),
            category: "incomplete".into(),
            summary: format!(
                "{unresolved_captures} Capture(s) lack Attempts and capture_failed recovery"
            ),
        });
    }

    let pending_attempts: i64 = conn.query_row(
        "SELECT COUNT(*) FROM normalization_attempts WHERE outcome = 'pending'",
        [],
        |row| row.get(0),
    )?;
    if pending_attempts > 0 {
        status = "failed".to_string();
        issues.push(HealthIssue {
            code: "pending_attempt".into(),
            severity: "repairable".into(),
            category: "incomplete".into(),
            summary: format!("{pending_attempts} pending Normalization Attempt(s)"),
        });
    }

    let broken_projections: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sessions s
         WHERE s.successful_projection_generation > 0
           AND (
             s.current_attempt_id IS NULL
             OR NOT EXISTS (
               SELECT 1 FROM normalization_attempts na
               WHERE na.id = s.current_attempt_id
                 AND na.outcome = 'succeeded'
                 AND na.projection_generation = s.successful_projection_generation
             )
           )",
        [],
        |row| row.get(0),
    )?;
    if broken_projections > 0 {
        status = "failed".to_string();
        issues.push(HealthIssue {
            code: "incomplete_projection".into(),
            severity: "blocking".into(),
            category: "incomplete".into(),
            summary: format!(
                "{broken_projections} Session(s) have invalid current Attempt/projection linkage"
            ),
        });
    }

    let counter_mismatches = count_session_counter_mismatches(conn)?;
    if counter_mismatches > 0 {
        status = "failed".to_string();
        issues.push(HealthIssue {
            code: "session_counter_mismatch".into(),
            severity: "repairable".into(),
            category: "incomplete".into(),
            summary: format!(
                "{counter_mismatches} Session(s) have mismatched materialized counters"
            ),
        });
    }

    Ok(status)
}

/**
 * Collect Capture-referenced blob paths relative to the Distill home.
 */
pub(super) fn referenced_blob_paths(conn: &Connection) -> LibraryResult<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT blob_path FROM captures WHERE content_kind = 'blob'")?;
    let rows = stmt.query_map([], |row| row.get::<_, Option<String>>(0))?;
    let mut out = HashSet::new();
    for row in rows {
        if let Some(path) = row? {
            out.insert(path);
        }
    }
    Ok(out)
}

/**
 * Count Sessions whose stored Capture/Attempt counters disagree with reality.
 */
fn count_session_counter_mismatches(conn: &Connection) -> LibraryResult<u64> {
    let mut stmt = conn.prepare(
        "SELECT source_kind, external_session_id,
                accepted_capture_count, normalization_attempt_count
         FROM sessions",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut mismatches = 0_u64;
    for row in rows {
        let (source_kind, external_session_id, stored_captures, stored_attempts) = row?;
        let accepted_capture_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captures WHERE source_kind = ?1 AND external_session_id = ?2",
            rusqlite::params![source_kind, external_session_id],
            |row| row.get(0),
        )?;
        let normalization_attempt_count: i64 = conn.query_row(
            "SELECT COUNT(*)
             FROM normalization_attempts na
             JOIN captures c ON c.id = na.capture_id
             WHERE c.source_kind = ?1 AND c.external_session_id = ?2",
            rusqlite::params![source_kind, external_session_id],
            |row| row.get(0),
        )?;
        if accepted_capture_count != stored_captures
            || normalization_attempt_count != stored_attempts
        {
            mismatches += 1;
        }
    }
    Ok(mismatches)
}

/**
 * Push a schema-category health issue with a stable generic summary.
 */
fn push_schema_issue(
    issues: &mut Vec<HealthIssue>,
    status: &mut String,
    code: &str,
    summary: &str,
) {
    *status = "failed".to_string();
    issues.push(HealthIssue {
        code: code.into(),
        severity: "blocking".into(),
        category: "schema".into(),
        summary: summary.into(),
    });
}
