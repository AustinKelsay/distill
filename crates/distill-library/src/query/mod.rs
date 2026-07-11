//! Public query, replay, and health helpers over Library storage.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::error::{LibraryError, LibraryResult};
use crate::storage::{read_capture_bytes, verify_migration_checksums, ContentRef};
use crate::types::{
    ActivityEventSummary, HealthReport, ProjectedArtifact, ProjectedMessage, SearchHit,
    SessionDetail, SessionSummary,
};

/**
 * Load one bounded Session Projection slice by identity.
 */
pub fn get_session(
    conn: &Connection,
    source_kind: &str,
    external_session_id: &str,
    message_limit: u32,
    artifact_limit: u32,
) -> LibraryResult<Option<SessionDetail>> {
    let summary = conn
        .query_row(
            "SELECT id, source_kind, external_session_id, title,
                    accepted_capture_count, normalization_attempt_count,
                    successful_projection_generation, metadata_json
             FROM sessions
             WHERE source_kind = ?1 AND external_session_id = ?2",
            params![source_kind, external_session_id],
            |row| {
                Ok((
                    SessionSummary {
                        id: row.get(0)?,
                        source_kind: row.get(1)?,
                        external_session_id: row.get(2)?,
                        title: row.get(3)?,
                        accepted_capture_count: row.get(4)?,
                        normalization_attempt_count: row.get(5)?,
                        successful_projection_generation: row.get(6)?,
                    },
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?;

    let Some((summary, metadata_json)) = summary else {
        return Ok(None);
    };

    let mut message_stmt = conn.prepare(
        "SELECT id, ordinal, role, message_kind, text
         FROM projection_messages
         WHERE session_id = ?1 AND projection_generation = ?2
         ORDER BY ordinal ASC
         LIMIT ?3",
    )?;
    let message_rows = message_stmt.query_map(
        params![
            summary.id,
            summary.successful_projection_generation,
            i64::from(message_limit)
        ],
        |row| {
            Ok(ProjectedMessage {
                id: row.get(0)?,
                ordinal: row.get(1)?,
                role: row.get(2)?,
                message_kind: row.get(3)?,
                text: row.get(4)?,
            })
        },
    )?;
    let mut messages = Vec::new();
    for row in message_rows {
        messages.push(row?);
    }

    let mut artifact_stmt = conn.prepare(
        "SELECT id, artifact_type, message_id, capture_fact_id, text_preview
         FROM projection_artifacts
         WHERE session_id = ?1 AND projection_generation = ?2
         ORDER BY id ASC
         LIMIT ?3",
    )?;
    let artifact_rows = artifact_stmt.query_map(
        params![
            summary.id,
            summary.successful_projection_generation,
            i64::from(artifact_limit)
        ],
        |row| {
            Ok(ProjectedArtifact {
                id: row.get(0)?,
                artifact_type: row.get(1)?,
                message_id: row.get(2)?,
                capture_fact_id: row.get(3)?,
                text_preview: row.get(4)?,
            })
        },
    )?;
    let mut artifacts = Vec::new();
    for row in artifact_rows {
        artifacts.push(row?);
    }

    Ok(Some(SessionDetail {
        summary,
        messages,
        artifacts,
        metadata_json,
    }))
}

/**
 * Search current projection text via FTS5.
 *
 * Punctuation-only queries return no hits.
 */
pub fn search(conn: &Connection, query: &str, limit: u32) -> LibraryResult<Vec<SearchHit>> {
    let tokens: Vec<&str> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let match_query = tokens
        .iter()
        .map(|token| format!("\"{}\"", token.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" AND ");

    let mut stmt = conn.prepare(
        "SELECT f.session_id, f.message_id, s.source_kind, s.external_session_id, f.role, f.text
         FROM projection_fts f
         JOIN sessions s ON s.id = f.session_id
         WHERE projection_fts MATCH ?1
         ORDER BY f.session_id DESC, f.message_id ASC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![match_query, i64::from(limit)], |row| {
        Ok(SearchHit {
            session_id: row.get(0)?,
            message_id: row.get(1)?,
            source_kind: row.get(2)?,
            external_session_id: row.get(3)?,
            role: row.get(4)?,
            text: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/**
 * Replay Distill-owned Capture bytes after verifying checksum.
 */
pub fn replay_capture(conn: &Connection, home: &Path, capture_id: i64) -> LibraryResult<Vec<u8>> {
    let content = load_content_ref(conn, capture_id)?;
    let bytes = read_capture_bytes(home, &content, capture_id)?;
    Ok(bytes)
}

/**
 * Load a ContentRef for an accepted Capture.
 */
fn load_content_ref(conn: &Connection, capture_id: i64) -> LibraryResult<ContentRef> {
    let row = conn
        .query_row(
            "SELECT content_kind, media_type, sha256, byte_size, inline_text, blob_path
             FROM captures WHERE id = ?1",
            [capture_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| LibraryError::NotFound(format!("capture {capture_id}")))?;

    let (kind, media_type, sha256, byte_size, inline_text, blob_path) = row;
    match kind.as_str() {
        "inline" => Ok(ContentRef::Inline {
            text: inline_text.ok_or_else(|| LibraryError::ContentIntegrity {
                capture_id,
                detail: "missing inline_text".into(),
            })?,
            sha256,
            byte_size: byte_size as u64,
            media_type,
        }),
        "blob" => Ok(ContentRef::Blob {
            relative_path: blob_path.ok_or_else(|| LibraryError::ContentIntegrity {
                capture_id,
                detail: "missing blob_path".into(),
            })?,
            sha256,
            byte_size: byte_size as u64,
            media_type,
        }),
        other => Err(LibraryError::ContentIntegrity {
            capture_id,
            detail: format!("unknown content_kind {other}"),
        }),
    }
}

/**
 * List recent Activity Events for contract assertions.
 */
pub fn list_activity(conn: &Connection, limit: u32) -> LibraryResult<Vec<ActivityEventSummary>> {
    let mut stmt = conn.prepare(
        "SELECT event_type, capture_id, session_id
         FROM activity_events
         ORDER BY id ASC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([i64::from(limit)], |row| {
        Ok(ActivityEventSummary {
            event_type: row.get(0)?,
            capture_id: row.get(1)?,
            session_id: row.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/**
 * Build a Library health report covering migrations, blobs, and FTS presence.
 */
pub fn health(conn: &Connection, home: &Path) -> LibraryResult<HealthReport> {
    let mut issues = Vec::new();
    let schema_status = match verify_migration_checksums(conn) {
        Ok(()) => "ok".to_string(),
        Err(err) => {
            issues.push(err.to_string());
            "failed".to_string()
        }
    };

    let mut content_status = "ok".to_string();
    let mut stmt = conn.prepare(
        "SELECT id, content_kind, sha256, byte_size, inline_text, blob_path FROM captures",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;
    for row in rows {
        let (id, kind, sha256, byte_size, inline_text, blob_path) = row?;
        match kind.as_str() {
            "inline" => {
                let text = inline_text.unwrap_or_default();
                let actual = hex::encode(Sha256::digest(text.as_bytes()));
                if actual != sha256 || text.len() as i64 != byte_size {
                    issues.push(format!("inline capture {id} failed checksum or size check"));
                    content_status = "failed".to_string();
                }
            }
            "blob" => {
                let relative = blob_path.unwrap_or_default();
                let absolute = home.join(&relative);
                if !absolute.is_file() {
                    issues.push(format!("blob capture {id} missing file {relative}"));
                    content_status = "failed".to_string();
                    continue;
                }
                let bytes = std::fs::read(&absolute)?;
                let actual = hex::encode(Sha256::digest(&bytes));
                if actual != sha256 || bytes.len() as i64 != byte_size {
                    issues.push(format!("blob capture {id} failed checksum or size check"));
                    content_status = "failed".to_string();
                }
            }
            other => {
                issues.push(format!("capture {id} has unknown content_kind {other}"));
                content_status = "failed".to_string();
            }
        }
    }

    let fts_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM projection_fts", [], |row| row.get(0))?;
    let message_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM projection_messages", [], |row| {
            row.get(0)
        })?;
    let fts_status = if fts_count == message_count {
        "ok".to_string()
    } else {
        issues.push(format!(
            "fts/projection mismatch: fts={fts_count} messages={message_count}"
        ));
        "failed".to_string()
    };

    Ok(HealthReport {
        ok: issues.is_empty(),
        schema_status,
        content_status,
        fts_status,
        issues,
    })
}
