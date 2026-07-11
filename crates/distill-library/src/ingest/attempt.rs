//! Normalization Attempt bookkeeping and atomic Session Projection publication.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

use crate::adapter::{CaptureCandidate, ParsedCapture};
use crate::error::LibraryResult;

/**
 * Insert a Normalization Attempt row.
 */
pub(super) fn insert_attempt(
    conn: &Connection,
    capture_id: i64,
    parser_id: &str,
    parser_version: &str,
    outcome: &str,
    error_class: Option<&str>,
    error_message: Option<&str>,
) -> LibraryResult<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    let finished = if outcome == "pending" {
        None
    } else {
        Some(now.as_str())
    };
    conn.execute(
        "INSERT INTO normalization_attempts (
            capture_id, parser_id, parser_version, started_at, finished_at,
            outcome, error_class, error_message, metrics_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '{}')",
        params![
            capture_id,
            parser_id,
            parser_version,
            now,
            finished,
            outcome,
            error_class,
            error_message,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/**
 * Mark an Attempt failed without mutating the current Session Projection.
 */
pub(super) fn fail_attempt(
    conn: &Connection,
    attempt_id: i64,
    error_class: &str,
    error_message: &str,
) -> LibraryResult<()> {
    conn.execute(
        "UPDATE normalization_attempts
         SET outcome = 'failed', finished_at = ?1, error_class = ?2, error_message = ?3
         WHERE id = ?4",
        params![
            chrono::Utc::now().to_rfc3339(),
            error_class,
            error_message,
            attempt_id
        ],
    )?;
    Ok(())
}

/**
 * Produce a caller-safe diagnostic without SQL or filesystem path leakage.
 */
pub(super) fn safe_failure_message(err: &impl std::fmt::Display) -> String {
    let raw = err.to_string();
    let trimmed = raw.chars().take(240).collect::<String>();
    if trimmed.contains("CHECK") || trimmed.contains("constraint") {
        "projection constraints rejected the Attempt output".to_string()
    } else {
        trimmed
    }
}

/**
 * Refresh Session Capture/Attempt counters after a failed Attempt without touching projection.
 */
pub(super) fn refresh_session_counters_after_failed_attempt(
    conn: &Connection,
    source_kind: &str,
    external_session_id: Option<&str>,
) -> LibraryResult<()> {
    let Some(external_session_id) = external_session_id else {
        return Ok(());
    };
    let session_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM sessions WHERE source_kind = ?1 AND external_session_id = ?2",
            params![source_kind, external_session_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(session_id) = session_id else {
        return Ok(());
    };

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
    conn.execute(
        "UPDATE sessions SET
            accepted_capture_count = ?1,
            normalization_attempt_count = ?2,
            updated_at = ?3
         WHERE id = ?4",
        params![
            accepted_capture_count,
            normalization_attempt_count,
            chrono::Utc::now().to_rfc3339(),
            session_id
        ],
    )?;
    Ok(())
}

/**
 * Atomically publish Capture Facts, Session Projection, FTS, and Activity.
 */
pub(super) fn publish_projection(
    conn: &mut Connection,
    capture_id: i64,
    attempt_id: i64,
    candidate: &CaptureCandidate,
    parsed: &ParsedCapture,
) -> LibraryResult<()> {
    let tx = conn.transaction()?;
    let now = chrono::Utc::now().to_rfc3339();

    let mut fact_ids = Vec::with_capacity(parsed.facts.len());
    for (ordinal, fact) in parsed.facts.iter().enumerate() {
        tx.execute(
            "INSERT INTO capture_facts (
                attempt_id, ordinal, record_type, role, is_meta, content_text, content_json, metadata_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}')",
            params![
                attempt_id,
                ordinal as i64,
                fact.record_type,
                fact.role,
                if fact.is_meta { 1 } else { 0 },
                fact.content_text,
                fact.content_json.to_string(),
            ],
        )?;
        fact_ids.push(tx.last_insert_rowid());
    }

    let source_kind = candidate.source_kind.as_str();
    let accepted_capture_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM captures WHERE source_kind = ?1 AND external_session_id = ?2",
        params![source_kind, parsed.external_session_id],
        |row| row.get(0),
    )?;
    let normalization_attempt_count: i64 = tx.query_row(
        "SELECT COUNT(*)
         FROM normalization_attempts na
         JOIN captures c ON c.id = na.capture_id
         WHERE c.source_kind = ?1 AND c.external_session_id = ?2",
        params![source_kind, parsed.external_session_id],
        |row| row.get(0),
    )?;

    let existing: Option<(i64, i64)> = tx
        .query_row(
            "SELECT id, successful_projection_generation
             FROM sessions WHERE source_kind = ?1 AND external_session_id = ?2",
            params![source_kind, parsed.external_session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    let (session_id, generation) = if let Some((id, generation)) = existing {
        let next_generation = generation + 1;
        tx.execute(
            "UPDATE sessions SET
                title = ?1,
                summary = ?2,
                metadata_json = ?3,
                updated_at = ?4,
                accepted_capture_count = ?5,
                normalization_attempt_count = ?6,
                successful_projection_generation = ?7,
                current_attempt_id = ?8
             WHERE id = ?9",
            params![
                parsed.title,
                parsed.summary,
                parsed.metadata.to_string(),
                now,
                accepted_capture_count,
                normalization_attempt_count,
                next_generation,
                attempt_id,
                id,
            ],
        )?;
        // Replace projection slices for the new generation by deleting prior rows.
        tx.execute(
            "DELETE FROM projection_artifacts WHERE session_id = ?1",
            [id],
        )?;
        let fts_rowids: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT rowid FROM projection_fts WHERE session_id = ?1")?;
            let rows = stmt.query_map([id], |row| row.get(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };
        for rowid in fts_rowids {
            tx.execute("DELETE FROM projection_fts WHERE rowid = ?1", [rowid])?;
        }
        tx.execute(
            "DELETE FROM projection_messages WHERE session_id = ?1",
            [id],
        )?;
        (id, next_generation)
    } else {
        tx.execute(
            "INSERT INTO sessions (
                source_kind, external_session_id, title, summary, started_at, updated_at,
                metadata_json, accepted_capture_count, normalization_attempt_count,
                successful_projection_generation, current_attempt_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8, 1, ?9)",
            params![
                source_kind,
                parsed.external_session_id,
                parsed.title,
                parsed.summary,
                now,
                parsed.metadata.to_string(),
                accepted_capture_count,
                normalization_attempt_count,
                attempt_id,
            ],
        )?;
        (tx.last_insert_rowid(), 1_i64)
    };

    let mut message_ids = Vec::with_capacity(parsed.messages.len());
    for (ordinal, message) in parsed.messages.iter().enumerate() {
        tx.execute(
            "INSERT INTO projection_messages (
                session_id, projection_generation, ordinal, role, message_kind, text,
                external_message_id, created_at, metadata_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '{}')",
            params![
                session_id,
                generation,
                ordinal as i64,
                message.role,
                message.message_kind,
                message.text,
                message.external_message_id,
                now,
            ],
        )?;
        let message_id = tx.last_insert_rowid();
        message_ids.push(message_id);
        tx.execute(
            "INSERT INTO projection_fts (session_id, message_id, title, project_path, role, text)
             VALUES (?1, ?2, ?3, '', ?4, ?5)",
            params![
                session_id,
                message_id,
                parsed.title.clone().unwrap_or_default(),
                message.role,
                message.text,
            ],
        )?;
    }

    for artifact in &parsed.artifacts {
        let message_id = artifact
            .message_ordinal
            .and_then(|idx| message_ids.get(idx).copied());
        let capture_fact_id = artifact
            .fact_ordinal
            .and_then(|idx| fact_ids.get(idx).copied());
        tx.execute(
            "INSERT INTO projection_artifacts (
                session_id, projection_generation, message_id, capture_fact_id,
                artifact_type, text_preview, content_json, metadata_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}')",
            params![
                session_id,
                generation,
                message_id,
                capture_fact_id,
                artifact.artifact_type,
                artifact.text_preview,
                artifact.content_json.to_string(),
            ],
        )?;
    }

    tx.execute(
        "UPDATE normalization_attempts
         SET outcome = 'succeeded', finished_at = ?1, projection_generation = ?2
         WHERE id = ?3",
        params![now, generation, attempt_id],
    )?;

    tx.execute(
        "INSERT INTO activity_events (
            event_type, occurred_at, source_kind, session_id, capture_id, attempt_id, payload_json
         ) VALUES ('projection_replaced', ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            now,
            source_kind,
            session_id,
            capture_id,
            attempt_id,
            json!({ "projection_generation": generation }).to_string(),
        ],
    )?;

    tx.commit()?;
    Ok(())
}
