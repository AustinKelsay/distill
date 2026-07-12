//! Explicit repair mutations for documented Library recovery actions.

use std::fs;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use super::cas::{open_safe_cas_file, scan_cas_tree, SafeCasOpen};
use super::checks::referenced_blob_paths;
use crate::error::LibraryResult;

/**
 * Counts produced by incomplete-state resolution inside one transaction.
 */
pub(super) struct IncompleteResolution {
    pub failed_pending_attempts: u64,
    pub appended_capture_failed_recoveries: u64,
    pub recomputed_session_counters: u64,
}

/**
 * Fail pending Attempts, append capture_failed for Attempt-less Captures, and
 * recompute Session counters — all in one idempotent transaction.
 */
pub(super) fn resolve_incomplete_state(
    conn: &mut Connection,
) -> LibraryResult<IncompleteResolution> {
    let tx = conn.transaction()?;
    let now = chrono::Utc::now().to_rfc3339();

    let failed_pending_attempts = tx.execute(
        "UPDATE normalization_attempts
         SET outcome = 'failed',
             finished_at = ?1,
             error_class = 'interrupted',
             error_message = 'ingest interrupted before projection publication'
         WHERE outcome = 'pending'",
        params![now],
    )? as u64;

    let mut capture_stmt = tx.prepare(
        "SELECT c.id FROM captures c
         WHERE NOT EXISTS (
           SELECT 1 FROM normalization_attempts na WHERE na.capture_id = c.id
         )
         AND NOT EXISTS (
           SELECT 1 FROM activity_events ae
           WHERE ae.capture_id = c.id AND ae.event_type = 'capture_failed'
         )
         ORDER BY c.id ASC",
    )?;
    let capture_ids = capture_stmt
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(capture_stmt);

    let mut appended_capture_failed_recoveries = 0_u64;
    for capture_id in capture_ids {
        let source_kind: Option<String> = tx
            .query_row(
                "SELECT source_kind FROM captures WHERE id = ?1",
                [capture_id],
                |row| row.get(0),
            )
            .optional()?;
        tx.execute(
            "INSERT INTO activity_events (
                event_type, occurred_at, source_kind, session_id, capture_id, attempt_id, payload_json
             ) VALUES ('capture_failed', ?1, ?2, NULL, ?3, NULL, ?4)",
            params![
                now,
                source_kind,
                capture_id,
                serde_json::json!({
                    "reason": "interrupted_before_attempt",
                    "recovery": "repair"
                })
                .to_string(),
            ],
        )?;
        appended_capture_failed_recoveries += 1;
    }

    let recomputed_session_counters = recompute_all_session_counters(&tx, &now)?;
    tx.commit()?;

    Ok(IncompleteResolution {
        failed_pending_attempts,
        appended_capture_failed_recoveries,
        recomputed_session_counters,
    })
}

/**
 * Recompute accepted_capture_count and normalization_attempt_count for every Session.
 *
 * Returns how many Session rows had at least one counter corrected.
 */
fn recompute_all_session_counters(conn: &Connection, now: &str) -> LibraryResult<u64> {
    let mut stmt = conn.prepare(
        "SELECT id, source_kind, external_session_id,
                accepted_capture_count, normalization_attempt_count
         FROM sessions
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    let mut corrected = 0_u64;
    for row in rows {
        let (session_id, source_kind, external_session_id, stored_captures, stored_attempts) = row?;
        let accepted_capture_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captures WHERE source_kind = ?1 AND external_session_id = ?2",
            params![source_kind, external_session_id],
            |row| row.get(0),
        )?;
        let normalization_attempt_count: i64 = conn.query_row(
            "SELECT COUNT(*)
             FROM normalization_attempts na
             JOIN captures c ON c.id = na.capture_id
             WHERE c.source_kind = ?1 AND c.external_session_id = ?2",
            params![source_kind, external_session_id],
            |row| row.get(0),
        )?;
        if accepted_capture_count != stored_captures
            || normalization_attempt_count != stored_attempts
        {
            conn.execute(
                "UPDATE sessions SET
                    accepted_capture_count = ?1,
                    normalization_attempt_count = ?2,
                    updated_at = ?3
                 WHERE id = ?4",
                params![
                    accepted_capture_count,
                    normalization_attempt_count,
                    now,
                    session_id
                ],
            )?;
            corrected += 1;
        }
    }
    Ok(corrected)
}

/**
 * Delete unreferenced in-root regular canonical CAS files only.
 *
 * Symlinks, malformed tree entries, and paths outside the Distill home are never
 * deletion candidates.
 */
pub(super) fn remove_orphan_blobs(
    conn: &Connection,
    home: &Path,
    blobs_dir: &Path,
) -> LibraryResult<u64> {
    let referenced = referenced_blob_paths(conn)?;
    let scan = scan_cas_tree(home, blobs_dir)?;
    let mut removed = 0_u64;
    for relative in scan.regular_canonical {
        if referenced.contains(&relative) {
            continue;
        }
        if let SafeCasOpen::Regular(absolute) = open_safe_cas_file(home, &relative) {
            fs::remove_file(&absolute)?;
            removed += 1;
        }
    }
    Ok(removed)
}

/**
 * Replace all FTS rows from current projection messages and Session fields.
 *
 * Restores `title` and `project_path` from the Session row — never hardcodes empty path.
 */
pub(super) fn rebuild_fts_from_projection(conn: &mut Connection) -> LibraryResult<u64> {
    let tx = conn.transaction()?;
    let rowids: Vec<i64> = {
        let mut stmt = tx.prepare("SELECT rowid FROM projection_fts")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }
        ids
    };
    for rowid in rowids {
        tx.execute("DELETE FROM projection_fts WHERE rowid = ?1", [rowid])?;
    }

    let mut stmt = tx.prepare(
        "SELECT pm.id, pm.session_id, pm.role, pm.text,
                COALESCE(s.title, ''), COALESCE(s.project_path, '')
         FROM projection_messages pm
         JOIN sessions s ON s.id = pm.session_id
         WHERE pm.projection_generation = s.successful_projection_generation
         ORDER BY pm.id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    let mut messages = Vec::new();
    for row in rows {
        messages.push(row?);
    }
    drop(stmt);

    let mut written = 0_u64;
    for (message_id, session_id, role, text, title, project_path) in messages {
        tx.execute(
            "INSERT INTO projection_fts (session_id, message_id, title, project_path, role, text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![session_id, message_id, title, project_path, role, text],
        )?;
        written += 1;
    }
    tx.commit()?;
    Ok(written)
}
