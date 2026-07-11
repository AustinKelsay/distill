//! Ingest pipeline: SourceAdapter -> verified Capture -> Attempt -> Projection.

use std::fs;
use std::path::{Component, Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::adapter::{CaptureCandidate, CaptureSnapshot, ParsedCapture, SourceAdapter, SourceKind};
use crate::error::{LibraryError, LibraryResult};
use crate::storage::{store_capture_bytes, ContentRef, DistillPaths};
use crate::types::IngestReport;

/**
 * Run a SourceAdapter through the production ingest seam into the Library store.
 */
pub fn ingest_adapter(
    conn: &mut Connection,
    paths: &DistillPaths,
    adapter: &dyn SourceAdapter,
    max_capture_bytes: u64,
) -> LibraryResult<IngestReport> {
    let source = adapter.detect()?;
    let source_id = upsert_source(conn, &source.kind, &source.display_name, &source.data_root)?;
    let candidates = adapter.discover(&source)?;
    let mut report = IngestReport::default();

    for candidate in candidates {
        enforce_configured_root(&source.data_root, &candidate)?;
        let snapshot = adapter.snapshot(&candidate)?;
        verify_snapshot_metadata(&snapshot)?;
        if let Some(existing_id) = find_duplicate(
            conn,
            candidate.source_kind.as_str(),
            &candidate.source_path,
            &snapshot.sha256,
        )? {
            report.skipped_duplicates += 1;
            report.capture_ids.push(existing_id);
            continue;
        }

        let content = store_capture_bytes(
            &paths.home,
            &paths.staging,
            &paths.blobs,
            &snapshot.bytes,
            &snapshot.media_type,
            max_capture_bytes,
        )?;

        let capture_id = {
            let tx = conn.transaction()?;
            let capture_id = insert_capture(
                &tx,
                source_id,
                &candidate,
                &content,
                snapshot.source_modified_at.as_deref(),
            )?;
            emit_activity(
                &tx,
                "capture_recorded",
                Some(candidate.source_kind.as_str()),
                None,
                Some(capture_id),
                None,
                json!({ "source_path": candidate.source_path, "sha256": snapshot.sha256 }),
            )?;
            tx.commit()?;
            capture_id
        };
        report.accepted_captures += 1;
        report.capture_ids.push(capture_id);

        match adapter.parse(&candidate, &snapshot) {
            Ok(parsed) => {
                let attempt_id = insert_attempt(
                    conn,
                    capture_id,
                    &source.parser.id,
                    &source.parser.version,
                    "pending",
                    None,
                    None,
                )?;
                match publish_projection(conn, capture_id, attempt_id, &candidate, &parsed) {
                    Ok(()) => {
                        report.successful_attempts += 1;
                    }
                    Err(err) => {
                        fail_attempt(conn, attempt_id, "projection_failed", &err.to_string())?;
                        report.failed_attempts += 1;
                    }
                }
            }
            Err(err) => {
                let attempt_id = insert_attempt(
                    conn,
                    capture_id,
                    &source.parser.id,
                    &source.parser.version,
                    "failed",
                    Some("parse_failed"),
                    Some(&err.to_string()),
                )?;
                let _ = attempt_id;
                report.failed_attempts += 1;
            }
        }
    }

    Ok(report)
}

/**
 * Verify adapter-reported snapshot metadata before dedupe or persistence.
 */
fn verify_snapshot_metadata(snapshot: &CaptureSnapshot) -> LibraryResult<()> {
    let actual_size = snapshot.bytes.len() as u64;
    let actual_sha256 = hex::encode(Sha256::digest(&snapshot.bytes));
    if snapshot.byte_size != actual_size || snapshot.sha256 != actual_sha256 {
        return Err(LibraryError::SourceAdapter(
            crate::adapter::SourceStageError::Snapshot(
                "adapter snapshot checksum or byte size did not match its bytes".to_string(),
            ),
        ));
    }
    Ok(())
}

/**
 * Reject file-backed candidates whose absolute path escapes the configured root.
 */
fn enforce_configured_root(root: &Path, candidate: &CaptureCandidate) -> LibraryResult<()> {
    let Some(absolute) = candidate.absolute_path.as_ref() else {
        return Ok(());
    };
    let canonical_root = fs::canonicalize(root)?;
    let canonical_path = if absolute.exists() {
        fs::canonicalize(absolute)?
    } else {
        // Still reject obvious escapes before snapshot for missing hostile paths.
        normalize_join(&canonical_root, absolute)?
    };
    if !canonical_path.starts_with(&canonical_root) {
        return Err(LibraryError::PathOutsideConfiguredRoot {
            path: canonical_path,
            root: canonical_root,
        });
    }
    Ok(())
}

/**
 * Best-effort join/normalization used when the candidate path does not yet exist.
 */
fn normalize_join(root: &Path, path: &Path) -> LibraryResult<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(LibraryError::PathOutsideConfiguredRoot {
                        path: joined,
                        root: root.to_path_buf(),
                    });
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Ok(normalized)
}

/**
 * Upsert a Source row and return its id.
 */
fn upsert_source(
    conn: &Connection,
    kind: &SourceKind,
    display_name: &str,
    data_root: &Path,
) -> LibraryResult<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO sources (kind, display_name, data_root, metadata_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, '{}', ?4, ?4)
         ON CONFLICT(kind) DO UPDATE SET
           display_name = excluded.display_name,
           data_root = excluded.data_root,
           updated_at = excluded.updated_at",
        params![
            kind.as_str(),
            display_name,
            data_root.display().to_string(),
            now
        ],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM sources WHERE kind = ?1",
        [kind.as_str()],
        |row| row.get(0),
    )?;
    Ok(id)
}

/**
 * Look up an exact-duplicate Capture by dedupe key.
 */
fn find_duplicate(
    conn: &Connection,
    source_kind: &str,
    source_path: &str,
    sha256: &str,
) -> LibraryResult<Option<i64>> {
    let id = conn
        .query_row(
            "SELECT id FROM captures WHERE source_kind = ?1 AND source_path = ?2 AND sha256 = ?3",
            params![source_kind, source_path, sha256],
            |row| row.get(0),
        )
        .optional()?;
    Ok(id)
}

/**
 * Insert an accepted Capture after Distill owns recoverable content.
 */
fn insert_capture(
    conn: &Connection,
    source_id: i64,
    candidate: &CaptureCandidate,
    content: &ContentRef,
    source_modified_at: Option<&str>,
) -> LibraryResult<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    let (content_kind, inline_text, blob_path, sha256, byte_size, media_type) = match content {
        ContentRef::Inline {
            text,
            sha256,
            byte_size,
            media_type,
        } => (
            "inline",
            Some(text.as_str()),
            None,
            sha256.as_str(),
            *byte_size,
            media_type.as_str(),
        ),
        ContentRef::Blob {
            relative_path,
            sha256,
            byte_size,
            media_type,
        } => (
            "blob",
            None,
            Some(relative_path.as_str()),
            sha256.as_str(),
            *byte_size,
            media_type.as_str(),
        ),
    };

    conn.execute(
        "INSERT INTO captures (
            source_id, source_kind, source_path, external_session_id,
            content_kind, media_type, sha256, byte_size, inline_text, blob_path,
            source_modified_at, accepted_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            source_id,
            candidate.source_kind.as_str(),
            candidate.source_path,
            candidate.external_session_id,
            content_kind,
            media_type,
            sha256,
            byte_size as i64,
            inline_text,
            blob_path,
            source_modified_at,
            now,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/**
 * Insert a Normalization Attempt row.
 */
fn insert_attempt(
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
fn fail_attempt(
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
 * Atomically publish Capture Facts, Session Projection, FTS, and Activity.
 */
fn publish_projection(
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

/**
 * Append an Activity Event outside a larger projection transaction when needed.
 */
fn emit_activity(
    conn: &Connection,
    event_type: &str,
    source_kind: Option<&str>,
    session_id: Option<i64>,
    capture_id: Option<i64>,
    attempt_id: Option<i64>,
    payload: serde_json::Value,
) -> LibraryResult<()> {
    conn.execute(
        "INSERT INTO activity_events (
            event_type, occurred_at, source_kind, session_id, capture_id, attempt_id, payload_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event_type,
            chrono::Utc::now().to_rfc3339(),
            source_kind,
            session_id,
            capture_id,
            attempt_id,
            payload.to_string(),
        ],
    )?;
    Ok(())
}
