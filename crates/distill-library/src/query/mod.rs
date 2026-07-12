//! Public query and replay helpers over Library storage.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{LibraryError, LibraryResult};
use crate::storage::{read_capture_bytes, ContentRef};
use crate::types::{
    derive_workflow_state, matches_workflow_lane, ActivityEventSummary, AttemptSummary,
    ProjectedArtifact, ProjectedMessage, SearchHit, SessionDetail, SessionDetailRequest,
    SessionLabel, SessionListItem, SessionListPage, SessionListRequest, SessionSummary, SessionTag,
    WorkflowLane,
};

/**
 * Load one bounded Session Projection slice by identity.
 *
 * Compatibility wrapper around [`session_detail`] without message/artifact cursors.
 */
pub fn get_session(
    conn: &Connection,
    source_kind: &str,
    external_session_id: &str,
    message_limit: u32,
    artifact_limit: u32,
) -> LibraryResult<Option<SessionDetail>> {
    session_detail(
        conn,
        &SessionDetailRequest {
            source_kind: source_kind.to_string(),
            external_session_id: external_session_id.to_string(),
            message_limit,
            artifact_limit,
            message_cursor: None,
            artifact_cursor: None,
        },
    )
}

/**
 * Load a bounded Session Projection detail page with optional continuation cursors.
 */
pub fn session_detail(
    conn: &Connection,
    request: &SessionDetailRequest,
) -> LibraryResult<Option<SessionDetail>> {
    let message_cursor = parse_message_cursor(request.message_cursor.as_deref())?;
    let artifact_cursor = parse_artifact_cursor(request.artifact_cursor.as_deref())?;

    let row = conn
        .query_row(
            "SELECT id, source_kind, external_session_id, title, project_path, source_url,
                    summary, started_at, updated_at, metadata_json,
                    accepted_capture_count, normalization_attempt_count,
                    successful_projection_generation
             FROM sessions
             WHERE source_kind = ?1 AND external_session_id = ?2",
            params![request.source_kind, request.external_session_id],
            |row| {
                Ok((
                    SessionSummary {
                        id: row.get(0)?,
                        source_kind: row.get(1)?,
                        external_session_id: row.get(2)?,
                        title: row.get(3)?,
                        accepted_capture_count: row.get(10)?,
                        normalization_attempt_count: row.get(11)?,
                        successful_projection_generation: row.get(12)?,
                    },
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .optional()?;

    let Some((
        summary,
        project_path,
        source_url,
        projection_summary,
        started_at,
        updated_at,
        raw_metadata,
    )) = row
    else {
        return Ok(None);
    };

    let metadata_json = sanitize_metadata_json(&raw_metadata);
    let labels = load_manual_labels(conn, &[summary.id])?
        .remove(&summary.id)
        .unwrap_or_default();
    let tags = load_manual_tags(conn, &[summary.id])?
        .remove(&summary.id)
        .unwrap_or_default();
    let workflow_state = derive_workflow_state(labels.iter().map(|label| label.name.as_str()));

    let (messages, next_message_cursor) = load_message_page(
        conn,
        summary.id,
        summary.successful_projection_generation,
        request.message_limit,
        message_cursor,
    )?;
    let (artifacts, next_artifact_cursor) = load_artifact_page(
        conn,
        summary.id,
        summary.successful_projection_generation,
        request.artifact_limit,
        artifact_cursor,
    )?;

    Ok(Some(SessionDetail {
        raw_capture_count: summary.accepted_capture_count,
        summary,
        messages,
        artifacts,
        metadata_json,
        project_path,
        source_url,
        projection_summary,
        started_at,
        updated_at,
        tags,
        labels,
        workflow_state,
        next_message_cursor,
        next_artifact_cursor,
    }))
}

/**
 * List sessions with optional FTS search, lane filter, and keyset cursor paging.
 */
pub fn list_sessions(
    conn: &Connection,
    request: &SessionListRequest,
) -> LibraryResult<SessionListPage> {
    let cursor = parse_list_cursor(request.cursor.as_deref())?;
    let normalized_query = match request.query.as_deref() {
        None => None,
        Some(text) if text.trim().is_empty() => None,
        Some(text) => {
            let match_query = normalize_search_query(text);
            if match_query.is_none() {
                return Ok(SessionListPage {
                    items: Vec::new(),
                    next_cursor: None,
                });
            }
            match_query
        }
    };

    let fetch_limit = i64::from(request.limit) + 1;
    let mut sql = String::from(
        "SELECT s.id, s.source_kind, s.external_session_id, s.title, s.project_path, s.updated_at,
                s.accepted_capture_count, s.normalization_attempt_count,
                s.successful_projection_generation,
                (
                  SELECT COUNT(*) FROM projection_messages pm
                  WHERE pm.session_id = s.id
                    AND pm.projection_generation = s.successful_projection_generation
                ) AS message_count,
                (
                  SELECT pm.text FROM projection_messages pm
                  WHERE pm.session_id = s.id
                    AND pm.projection_generation = s.successful_projection_generation
                    AND pm.role = 'user' AND pm.message_kind = 'text'
                  ORDER BY pm.ordinal ASC, pm.id ASC LIMIT 1
                ) AS first_user_text,
                (
                  SELECT pm.text FROM projection_messages pm
                  WHERE pm.session_id = s.id
                    AND pm.projection_generation = s.successful_projection_generation
                    AND pm.role = 'assistant' AND pm.message_kind = 'text'
                  ORDER BY pm.ordinal ASC, pm.id ASC LIMIT 1
                ) AS first_assistant_text
         FROM sessions s
         WHERE s.successful_projection_generation > 0",
    );
    let mut bind: Vec<rusqlite::types::Value> = Vec::new();

    if let Some(match_query) = &normalized_query {
        sql.push_str(
            " AND s.id IN (
                SELECT DISTINCT f.session_id
                FROM projection_fts f
                WHERE projection_fts MATCH ?1
              )",
        );
        bind.push(match_query.clone().into());
    }

    append_lane_sql(request.lane, &mut sql);

    if let Some((updated_at, id)) = &cursor {
        let idx = bind.len() + 1;
        sql.push_str(&format!(
            " AND (
                COALESCE(s.updated_at, '') < ?{idx}
                OR (COALESCE(s.updated_at, '') = ?{idx} AND s.id < ?{})
              )",
            idx + 1
        ));
        bind.push(updated_at.clone().into());
        bind.push((*id).into());
    }

    sql.push_str(" ORDER BY COALESCE(s.updated_at, '') DESC, s.id DESC");
    let limit_idx = bind.len() + 1;
    sql.push_str(&format!(" LIMIT ?{limit_idx}"));
    bind.push(fetch_limit.into());

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(bind), |row| {
        Ok(ListRow {
            id: row.get(0)?,
            source_kind: row.get(1)?,
            external_session_id: row.get(2)?,
            title: row.get(3)?,
            project_path: row.get(4)?,
            updated_at: row.get(5)?,
            accepted_capture_count: row.get(6)?,
            normalization_attempt_count: row.get(7)?,
            successful_projection_generation: row.get(8)?,
            message_count: row.get(9)?,
            first_user_text: row.get(10)?,
            first_assistant_text: row.get(11)?,
        })
    })?;

    let mut candidates = Vec::new();
    for row in rows {
        candidates.push(row?);
    }

    let session_ids: Vec<i64> = candidates.iter().map(|row| row.id).collect();
    let mut labels_by_id = load_manual_labels(conn, &session_ids)?;
    let mut tags_by_id = load_manual_tags(conn, &session_ids)?;

    let mut items = Vec::new();
    for row in candidates {
        let labels = labels_by_id.remove(&row.id).unwrap_or_default();
        let tags = tags_by_id.remove(&row.id).unwrap_or_default();
        if !matches_workflow_lane(request.lane, labels.iter().map(|label| label.name.as_str())) {
            continue;
        }
        let workflow_state = derive_workflow_state(labels.iter().map(|label| label.name.as_str()));
        items.push(SessionListItem {
            id: row.id,
            source_kind: row.source_kind,
            external_session_id: row.external_session_id,
            title: derive_title(row.title.as_deref(), row.first_user_text.as_deref()),
            project_path: row.project_path,
            updated_at: row.updated_at,
            preview: derive_preview(
                row.first_assistant_text.as_deref(),
                row.first_user_text.as_deref(),
            ),
            message_count: row.message_count,
            accepted_capture_count: row.accepted_capture_count,
            normalization_attempt_count: row.normalization_attempt_count,
            successful_projection_generation: row.successful_projection_generation,
            labels,
            tags,
            workflow_state,
        });
        if items.len() as i64 >= fetch_limit {
            break;
        }
    }

    let next_cursor = if items.len() as i64 > i64::from(request.limit) {
        items.pop();
        items
            .last()
            .map(|item| encode_list_cursor(item.updated_at.as_deref().unwrap_or(""), item.id))
    } else {
        None
    };

    Ok(SessionListPage { items, next_cursor })
}

/**
 * List immutable Attempt summaries for one Capture, oldest first.
 */
pub fn list_capture_attempts(
    conn: &Connection,
    capture_id: i64,
) -> LibraryResult<Vec<AttemptSummary>> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM captures WHERE id = ?1",
            [capture_id],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(LibraryError::NotFound(format!("capture {capture_id}")));
    }

    let mut stmt = conn.prepare(
        "SELECT na.id, na.capture_id, na.parser_id, na.parser_version, na.outcome,
                na.error_class, na.error_message, na.projection_generation,
                (SELECT COUNT(*) FROM capture_facts cf WHERE cf.attempt_id = na.id)
         FROM normalization_attempts na
         WHERE na.capture_id = ?1
         ORDER BY na.id ASC",
    )?;
    let rows = stmt.query_map([capture_id], |row| {
        Ok(AttemptSummary {
            id: row.get(0)?,
            capture_id: row.get(1)?,
            parser_id: row.get(2)?,
            parser_version: row.get(3)?,
            outcome: row.get(4)?,
            error_class: row.get(5)?,
            error_message: row.get(6)?,
            projection_generation: row.get(7)?,
            fact_count: row.get(8)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/**
 * Search current projection text via FTS5.
 *
 * Uses canonical Unicode token extraction and quoted-AND normalization.
 * Zero-token queries return no hits.
 */
pub fn search(conn: &Connection, query: &str, limit: u32) -> LibraryResult<Vec<SearchHit>> {
    let Some(match_query) = normalize_search_query(query) else {
        return Ok(Vec::new());
    };

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
 * Normalize free-text search into a safe FTS quoted-AND query.
 *
 * Returns `None` when zero Unicode tokens are extracted.
 */
pub fn normalize_search_query(query: &str) -> Option<String> {
    let tokens = extract_search_tokens(query);
    if tokens.is_empty() {
        return None;
    }
    Some(
        tokens
            .iter()
            .map(|token| format!("\"{token}\""))
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

/**
 * Extract tokens using the canonical Unicode pattern `[\p{L}\p{N}_-]+`.
 */
fn extract_search_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in query.chars() {
        if ch.is_alphabetic() || ch.is_numeric() || ch == '_' || ch == '-' {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/**
 * Append SQL predicates that encode lane membership using manual label assignments only.
 */
fn append_lane_sql(lane: WorkflowLane, sql: &mut String) {
    let manual_labels = "SELECT l.name
         FROM label_assignments la
         JOIN labels l ON l.id = la.label_id
         WHERE la.object_type = 'session'
           AND la.origin = 'manual'
           AND la.object_id = s.id";
    match lane {
        WorkflowLane::All => {}
        WorkflowLane::Favorites => {
            sql.push_str(&format!(
                " AND EXISTS (
                    SELECT 1 FROM ({manual_labels}) names WHERE names.name = 'favorite'
                  )"
            ));
        }
        WorkflowLane::NeedsReview => {
            sql.push_str(&format!(
                " AND (
                    EXISTS (SELECT 1 FROM ({manual_labels}) names WHERE names.name IN ('exclude', 'sensitive'))
                    OR (
                      EXISTS (SELECT 1 FROM ({manual_labels}) names WHERE names.name = 'train')
                      AND EXISTS (SELECT 1 FROM ({manual_labels}) names WHERE names.name = 'holdout')
                    )
                  )"
            ));
        }
        WorkflowLane::TrainReady => {
            sql.push_str(&format!(
                " AND EXISTS (SELECT 1 FROM ({manual_labels}) names WHERE names.name = 'train')
                  AND NOT EXISTS (
                    SELECT 1 FROM ({manual_labels}) names
                    WHERE names.name IN ('exclude', 'sensitive', 'holdout')
                  )"
            ));
        }
        WorkflowLane::HoldoutReady => {
            sql.push_str(&format!(
                " AND EXISTS (SELECT 1 FROM ({manual_labels}) names WHERE names.name = 'holdout')
                  AND NOT EXISTS (
                    SELECT 1 FROM ({manual_labels}) names
                    WHERE names.name IN ('exclude', 'sensitive', 'train')
                  )"
            ));
        }
    }
}

struct ListRow {
    id: i64,
    source_kind: String,
    external_session_id: String,
    title: Option<String>,
    project_path: Option<String>,
    updated_at: Option<String>,
    accepted_capture_count: i64,
    normalization_attempt_count: i64,
    successful_projection_generation: i64,
    message_count: i64,
    first_user_text: Option<String>,
    first_assistant_text: Option<String>,
}

/**
 * Load a message page and optional next cursor.
 */
fn load_message_page(
    conn: &Connection,
    session_id: i64,
    generation: i64,
    limit: u32,
    cursor: Option<(i64, i64)>,
) -> LibraryResult<(Vec<ProjectedMessage>, Option<String>)> {
    let fetch = i64::from(limit) + 1;
    let mut sql = String::from(
        "SELECT id, ordinal, role, message_kind, text
         FROM projection_messages
         WHERE session_id = ?1 AND projection_generation = ?2",
    );
    let mut bind: Vec<rusqlite::types::Value> = vec![session_id.into(), generation.into()];
    if let Some((ordinal, id)) = cursor {
        sql.push_str(" AND (ordinal > ?3 OR (ordinal = ?3 AND id > ?4))");
        bind.push(ordinal.into());
        bind.push(id.into());
        sql.push_str(" ORDER BY ordinal ASC, id ASC LIMIT ?5");
        bind.push(fetch.into());
    } else {
        sql.push_str(" ORDER BY ordinal ASC, id ASC LIMIT ?3");
        bind.push(fetch.into());
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(bind), |row| {
        Ok(ProjectedMessage {
            id: row.get(0)?,
            ordinal: row.get(1)?,
            role: row.get(2)?,
            message_kind: row.get(3)?,
            text: row.get(4)?,
        })
    })?;
    let mut messages = Vec::new();
    for row in rows {
        messages.push(row?);
    }
    let next_cursor = if messages.len() as i64 > i64::from(limit) {
        messages.pop();
        messages
            .last()
            .map(|message| encode_message_cursor(message.ordinal, message.id))
    } else {
        None
    };
    Ok((messages, next_cursor))
}

/**
 * Load an artifact page and optional next cursor.
 */
fn load_artifact_page(
    conn: &Connection,
    session_id: i64,
    generation: i64,
    limit: u32,
    cursor: Option<i64>,
) -> LibraryResult<(Vec<ProjectedArtifact>, Option<String>)> {
    let fetch = i64::from(limit) + 1;
    let mut sql = String::from(
        "SELECT id, artifact_type, message_id, capture_fact_id, text_preview
         FROM projection_artifacts
         WHERE session_id = ?1 AND projection_generation = ?2",
    );
    let mut bind: Vec<rusqlite::types::Value> = vec![session_id.into(), generation.into()];
    if let Some(id) = cursor {
        sql.push_str(" AND id > ?3");
        bind.push(id.into());
        sql.push_str(" ORDER BY id ASC LIMIT ?4");
        bind.push(fetch.into());
    } else {
        sql.push_str(" ORDER BY id ASC LIMIT ?3");
        bind.push(fetch.into());
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(bind), |row| {
        Ok(ProjectedArtifact {
            id: row.get(0)?,
            artifact_type: row.get(1)?,
            message_id: row.get(2)?,
            capture_fact_id: row.get(3)?,
            text_preview: row.get(4)?,
        })
    })?;
    let mut artifacts = Vec::new();
    for row in rows {
        artifacts.push(row?);
    }
    let next_cursor = if artifacts.len() as i64 > i64::from(limit) {
        artifacts.pop();
        artifacts
            .last()
            .map(|artifact| encode_artifact_cursor(artifact.id))
    } else {
        None
    };
    Ok((artifacts, next_cursor))
}

/**
 * Load manual-origin labels for the given session ids, ordered by name.
 */
fn load_manual_labels(
    conn: &Connection,
    session_ids: &[i64],
) -> LibraryResult<std::collections::HashMap<i64, Vec<SessionLabel>>> {
    let mut map = std::collections::HashMap::new();
    for id in session_ids {
        map.insert(*id, Vec::new());
    }
    if session_ids.is_empty() {
        return Ok(map);
    }
    let placeholders = session_ids
        .iter()
        .enumerate()
        .map(|(idx, _)| format!("?{}", idx + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT la.object_id, l.id, l.name, l.scope, la.origin
         FROM label_assignments la
         JOIN labels l ON l.id = la.label_id
         WHERE la.object_type = 'session'
           AND la.origin = 'manual'
           AND la.object_id IN ({placeholders})
         ORDER BY la.object_id ASC, l.name ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(session_ids.iter().copied()),
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                SessionLabel {
                    id: row.get(1)?,
                    name: row.get(2)?,
                    scope: row.get(3)?,
                    origin: row.get(4)?,
                },
            ))
        },
    )?;
    for row in rows {
        let (session_id, label) = row?;
        if let Some(bucket) = map.get_mut(&session_id) {
            bucket.push(label);
        }
    }
    Ok(map)
}

/**
 * Load manual-origin tags for the given session ids, ordered by name.
 */
fn load_manual_tags(
    conn: &Connection,
    session_ids: &[i64],
) -> LibraryResult<std::collections::HashMap<i64, Vec<SessionTag>>> {
    let mut map = std::collections::HashMap::new();
    for id in session_ids {
        map.insert(*id, Vec::new());
    }
    if session_ids.is_empty() {
        return Ok(map);
    }
    let placeholders = session_ids
        .iter()
        .enumerate()
        .map(|(idx, _)| format!("?{}", idx + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT ta.object_id, t.id, t.name, t.kind, ta.origin
         FROM tag_assignments ta
         JOIN tags t ON t.id = ta.tag_id
         WHERE ta.object_type = 'session'
           AND ta.origin = 'manual'
           AND ta.object_id IN ({placeholders})
         ORDER BY ta.object_id ASC, t.name ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(session_ids.iter().copied()),
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                SessionTag {
                    id: row.get(1)?,
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    origin: row.get(4)?,
                },
            ))
        },
    )?;
    for row in rows {
        let (session_id, tag) = row?;
        if let Some(bucket) = map.get_mut(&session_id) {
            bucket.push(tag);
        }
    }
    Ok(map)
}

fn derive_title(title: Option<&str>, first_user_text: Option<&str>) -> String {
    if let Some(direct) = title.map(str::trim).filter(|value| !value.is_empty()) {
        return direct.to_string();
    }
    if let Some(preview) = clean_excerpt(first_user_text, 160) {
        return preview;
    }
    "Untitled session".to_string()
}

fn derive_preview(
    first_assistant_text: Option<&str>,
    first_user_text: Option<&str>,
) -> Option<String> {
    clean_excerpt(first_assistant_text, 280).or_else(|| clean_excerpt(first_user_text, 280))
}

fn clean_excerpt(text: Option<&str>, max_length: usize) -> Option<String> {
    let cleaned = text?.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        return None;
    }
    if cleaned.chars().count() > max_length {
        let truncated: String = cleaned.chars().take(max_length.saturating_sub(1)).collect();
        Some(format!("{}…", truncated.trim_end()))
    } else {
        Some(cleaned)
    }
}

fn sanitize_metadata_json(raw: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) if value.is_object() => value.to_string(),
        _ => "{}".to_string(),
    }
}

fn encode_list_cursor(updated_at: &str, id: i64) -> String {
    format!("v1\u{1f}{updated_at}\u{1f}{id}")
}

fn parse_list_cursor(raw: Option<&str>) -> LibraryResult<Option<(String, i64)>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut parts = raw.split('\u{1f}');
    let version = parts.next();
    let updated_at = parts.next();
    let id_text = parts.next();
    if version != Some("v1") || updated_at.is_none() || id_text.is_none() || parts.next().is_some()
    {
        return Err(LibraryError::InvalidArgument(
            "session list cursor must use the v1 format".into(),
        ));
    }
    let id: i64 = id_text
        .unwrap()
        .parse()
        .map_err(|_| LibraryError::InvalidArgument("session list cursor id is invalid".into()))?;
    Ok(Some((updated_at.unwrap().to_string(), id)))
}

fn encode_message_cursor(ordinal: i64, id: i64) -> String {
    format!("v1\u{1f}{ordinal}\u{1f}{id}")
}

fn parse_message_cursor(raw: Option<&str>) -> LibraryResult<Option<(i64, i64)>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut parts = raw.split('\u{1f}');
    let version = parts.next();
    let ordinal_text = parts.next();
    let id_text = parts.next();
    if version != Some("v1")
        || ordinal_text.is_none()
        || id_text.is_none()
        || parts.next().is_some()
    {
        return Err(LibraryError::InvalidArgument(
            "message cursor must use the v1 format".into(),
        ));
    }
    let ordinal: i64 = ordinal_text
        .unwrap()
        .parse()
        .map_err(|_| LibraryError::InvalidArgument("message cursor ordinal is invalid".into()))?;
    let id: i64 = id_text
        .unwrap()
        .parse()
        .map_err(|_| LibraryError::InvalidArgument("message cursor id is invalid".into()))?;
    Ok(Some((ordinal, id)))
}

fn encode_artifact_cursor(id: i64) -> String {
    format!("v1\u{1f}{id}")
}

fn parse_artifact_cursor(raw: Option<&str>) -> LibraryResult<Option<i64>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut parts = raw.split('\u{1f}');
    let version = parts.next();
    let id_text = parts.next();
    if version != Some("v1") || id_text.is_none() || parts.next().is_some() {
        return Err(LibraryError::InvalidArgument(
            "artifact cursor must use the v1 format".into(),
        ));
    }
    let id: i64 = id_text
        .unwrap()
        .parse()
        .map_err(|_| LibraryError::InvalidArgument("artifact cursor id is invalid".into()))?;
    Ok(Some(id))
}
