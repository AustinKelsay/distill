//! Manual session tag and label curation mutations.
//!
//! Targets Session Identity `(source_kind, external_session_id)`. Every changed
//! assignment and its Activity Event commit in one SQLite transaction. Blank,
//! unknown, missing-session, duplicate-add, and missing-remove paths are typed
//! no-ops (`changed: false`) with no Activity side effects.

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};

use crate::error::LibraryResult;
use crate::types::{
    derive_workflow_state, CurationMutationResult, SessionCurationRequest, SessionIdentity,
    SessionLabel, SessionTag, WorkflowState,
};

const MANUAL_ORIGIN: &str = "manual";
const TAG_KIND_MANUAL: &str = "manual";
const OBJECT_TYPE_SESSION: &str = "session";
const DATASET_LABELS: &[&str] = &["exclude", "holdout", "train"];

/**
 * Add a manual tag to the session addressed by `request`.
 *
 * Parameters:
 * - `conn`: open Distill SQLite connection.
 * - `request`: session identity plus tag name (trimmed, Unicode-lowercased).
 */
pub(crate) fn add_session_tag(
    conn: &mut Connection,
    request: SessionCurationRequest,
) -> LibraryResult<CurationMutationResult> {
    let identity = identity_from_request(&request);
    let Some(name) = normalize_mutation_name(&request.name) else {
        return unchanged_result(conn, identity);
    };
    let Some(session_id) = find_session_id(conn, &identity)? else {
        return Ok(empty_unchanged(identity));
    };

    let tx = conn.transaction()?;
    let now = chrono::Utc::now().to_rfc3339();
    let tag_id = ensure_manual_tag(&tx, &name, &now)?;
    let inserted = tx.execute(
        "INSERT INTO tag_assignments (object_type, object_id, tag_id, origin, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(object_type, object_id, tag_id, origin) DO NOTHING",
        params![OBJECT_TYPE_SESSION, session_id, tag_id, MANUAL_ORIGIN, now],
    )?;
    if inserted == 0 {
        let result = load_mutation_result(&tx, identity, session_id, false)?;
        tx.commit()?;
        return Ok(result);
    }

    emit_curation_activity(
        &tx,
        "tag_added",
        &identity.source_kind,
        session_id,
        json!({
            "object_type": OBJECT_TYPE_SESSION,
            "tag_id": tag_id,
            "tag_name": name,
            "origin": MANUAL_ORIGIN,
        }),
    )?;
    let result = load_mutation_result(&tx, identity, session_id, true)?;
    tx.commit()?;
    Ok(result)
}

/**
 * Remove a manual tag from the session addressed by `request`.
 *
 * Parameters:
 * - `conn`: open Distill SQLite connection.
 * - `request`: session identity plus tag name (trimmed, Unicode-lowercased).
 */
pub(crate) fn remove_session_tag(
    conn: &mut Connection,
    request: SessionCurationRequest,
) -> LibraryResult<CurationMutationResult> {
    let identity = identity_from_request(&request);
    let Some(name) = normalize_mutation_name(&request.name) else {
        return unchanged_result(conn, identity);
    };
    let Some(session_id) = find_session_id(conn, &identity)? else {
        return Ok(empty_unchanged(identity));
    };

    let tx = conn.transaction()?;
    let tag_id: Option<i64> = tx
        .query_row("SELECT id FROM tags WHERE name = ?1", [&name], |row| {
            row.get(0)
        })
        .optional()?;
    let Some(tag_id) = tag_id else {
        let result = load_mutation_result(&tx, identity, session_id, false)?;
        tx.commit()?;
        return Ok(result);
    };

    let deleted = tx.execute(
        "DELETE FROM tag_assignments
         WHERE object_type = ?1
           AND object_id = ?2
           AND tag_id = ?3
           AND origin = ?4",
        params![OBJECT_TYPE_SESSION, session_id, tag_id, MANUAL_ORIGIN],
    )?;
    if deleted == 0 {
        let result = load_mutation_result(&tx, identity, session_id, false)?;
        tx.commit()?;
        return Ok(result);
    }

    emit_curation_activity(
        &tx,
        "tag_removed",
        &identity.source_kind,
        session_id,
        json!({
            "object_type": OBJECT_TYPE_SESSION,
            "tag_id": tag_id,
            "tag_name": name,
            "origin": MANUAL_ORIGIN,
        }),
    )?;
    let result = load_mutation_result(&tx, identity, session_id, true)?;
    tx.commit()?;
    Ok(result)
}

/**
 * Toggle a seeded catalog label on the session addressed by `request`.
 *
 * Enabling a dataset label removes other dataset labels in the same transaction
 * and emits `label_toggled` for each removal plus the enable. Orthogonal
 * `sensitive` / `favorite` modifiers are preserved.
 *
 * Parameters:
 * - `conn`: open Distill SQLite connection.
 * - `request`: session identity plus label name (trimmed, Unicode-lowercased).
 */
pub(crate) fn toggle_session_label(
    conn: &mut Connection,
    request: SessionCurationRequest,
) -> LibraryResult<CurationMutationResult> {
    let identity = identity_from_request(&request);
    let Some(name) = normalize_mutation_name(&request.name) else {
        return unchanged_result(conn, identity);
    };
    let Some(session_id) = find_session_id(conn, &identity)? else {
        return Ok(empty_unchanged(identity));
    };

    let tx = conn.transaction()?;
    let label = tx
        .query_row(
            "SELECT id, name FROM labels WHERE name = ?1",
            [&name],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((label_id, label_name)) = label else {
        let result = load_mutation_result(&tx, identity, session_id, false)?;
        tx.commit()?;
        return Ok(result);
    };

    let existing_assignment: Option<i64> = tx
        .query_row(
            "SELECT id FROM label_assignments
             WHERE object_type = ?1
               AND object_id = ?2
               AND label_id = ?3
               AND origin = ?4",
            params![OBJECT_TYPE_SESSION, session_id, label_id, MANUAL_ORIGIN],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(assignment_id) = existing_assignment {
        let deleted = tx.execute(
            "DELETE FROM label_assignments WHERE id = ?1",
            [assignment_id],
        )?;
        if deleted == 0 {
            let result = load_mutation_result(&tx, identity, session_id, false)?;
            tx.commit()?;
            return Ok(result);
        }
        emit_label_toggled(
            &tx,
            &identity.source_kind,
            session_id,
            label_id,
            &label_name,
            false,
        )?;
        let result = load_mutation_result(&tx, identity, session_id, true)?;
        tx.commit()?;
        return Ok(result);
    }

    // A derived/non-manual assignment occupies the same catalog slot. Treat it
    // as an idempotent no-op rather than deleting another manual dataset label
    // and then failing the insert on the schema's uniqueness constraint.
    let non_manual_assignment: Option<String> = tx
        .query_row(
            "SELECT origin FROM label_assignments
             WHERE object_type = ?1
               AND object_id = ?2
               AND label_id = ?3
               AND origin != ?4
             LIMIT 1",
            params![OBJECT_TYPE_SESSION, session_id, label_id, MANUAL_ORIGIN],
            |row| row.get(0),
        )
        .optional()?;
    if non_manual_assignment.is_some() {
        let result = load_mutation_result(&tx, identity, session_id, false)?;
        tx.commit()?;
        return Ok(result);
    }

    if is_dataset_label(&label_name) {
        remove_conflicting_dataset_labels(&tx, &identity.source_kind, session_id, &label_name)?;
    }

    let now = chrono::Utc::now().to_rfc3339();
    let inserted = tx.execute(
        "INSERT INTO label_assignments (object_type, object_id, label_id, origin, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(object_type, object_id, label_id) DO NOTHING",
        params![
            OBJECT_TYPE_SESSION,
            session_id,
            label_id,
            MANUAL_ORIGIN,
            now
        ],
    )?;
    if inserted == 0 {
        let result = load_mutation_result(&tx, identity, session_id, false)?;
        tx.commit()?;
        return Ok(result);
    }

    emit_label_toggled(
        &tx,
        &identity.source_kind,
        session_id,
        label_id,
        &label_name,
        true,
    )?;
    let result = load_mutation_result(&tx, identity, session_id, true)?;
    tx.commit()?;
    Ok(result)
}

/**
 * Trim and Unicode-lowercase a mutation name. Blank results become `None`.
 */
fn normalize_mutation_name(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn identity_from_request(request: &SessionCurationRequest) -> SessionIdentity {
    SessionIdentity {
        source_kind: request.source_kind.clone(),
        external_session_id: request.external_session_id.clone(),
    }
}

fn empty_unchanged(identity: SessionIdentity) -> CurationMutationResult {
    CurationMutationResult {
        changed: false,
        identity,
        tags: Vec::new(),
        labels: Vec::new(),
        workflow_state: WorkflowState::Neutral,
    }
}

/**
 * Return current curation state without writing when the session may exist.
 */
fn unchanged_result(
    conn: &Connection,
    identity: SessionIdentity,
) -> LibraryResult<CurationMutationResult> {
    let Some(session_id) = find_session_id(conn, &identity)? else {
        return Ok(empty_unchanged(identity));
    };
    load_mutation_result(conn, identity, session_id, false)
}

fn find_session_id(conn: &Connection, identity: &SessionIdentity) -> LibraryResult<Option<i64>> {
    let id = conn
        .query_row(
            "SELECT id FROM sessions
             WHERE source_kind = ?1 AND external_session_id = ?2",
            params![identity.source_kind, identity.external_session_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(id)
}

/**
 * Insert a manual-kind tag catalog row when missing; return its id.
 */
fn ensure_manual_tag(tx: &Transaction<'_>, name: &str, now: &str) -> LibraryResult<i64> {
    tx.execute(
        "INSERT INTO tags (name, kind, created_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(name) DO NOTHING",
        params![name, TAG_KIND_MANUAL, now],
    )?;
    let tag_id = tx.query_row("SELECT id FROM tags WHERE name = ?1", [name], |row| {
        row.get(0)
    })?;
    Ok(tag_id)
}

fn is_dataset_label(name: &str) -> bool {
    DATASET_LABELS.contains(&name)
}

/**
 * Remove other dataset labels for this session and audit each disable.
 */
fn remove_conflicting_dataset_labels(
    tx: &Transaction<'_>,
    source_kind: &str,
    session_id: i64,
    keeping: &str,
) -> LibraryResult<()> {
    let conflicting: Vec<(i64, i64, String)> = {
        let mut stmt = tx.prepare(
            "SELECT la.id, l.id, l.name
             FROM label_assignments la
             JOIN labels l ON l.id = la.label_id
             WHERE la.object_type = ?1
               AND la.object_id = ?2
               AND la.origin = ?3
               AND l.name IN ('train', 'holdout', 'exclude')
               AND l.name != ?4
             ORDER BY l.name ASC",
        )?;
        let rows = stmt.query_map(
            params![OBJECT_TYPE_SESSION, session_id, MANUAL_ORIGIN, keeping],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out
    };

    for (assignment_id, label_id, label_name) in conflicting {
        let deleted = tx.execute(
            "DELETE FROM label_assignments WHERE id = ?1",
            [assignment_id],
        )?;
        if deleted == 0 {
            continue;
        }
        emit_label_toggled(tx, source_kind, session_id, label_id, &label_name, false)?;
    }
    Ok(())
}

fn emit_label_toggled(
    tx: &Transaction<'_>,
    source_kind: &str,
    session_id: i64,
    label_id: i64,
    label_name: &str,
    enabled: bool,
) -> LibraryResult<()> {
    emit_curation_activity(
        tx,
        "label_toggled",
        source_kind,
        session_id,
        json!({
            "object_type": OBJECT_TYPE_SESSION,
            "label_id": label_id,
            "label_name": label_name,
            "origin": MANUAL_ORIGIN,
            "enabled": enabled,
        }),
    )
}

fn emit_curation_activity(
    tx: &Transaction<'_>,
    event_type: &str,
    source_kind: &str,
    session_id: i64,
    payload: Value,
) -> LibraryResult<()> {
    tx.execute(
        "INSERT INTO activity_events (
            event_type, occurred_at, source_kind, session_id, capture_id, attempt_id, payload_json
         ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5)",
        params![
            event_type,
            chrono::Utc::now().to_rfc3339(),
            source_kind,
            session_id,
            payload.to_string(),
        ],
    )?;
    Ok(())
}

fn load_mutation_result(
    conn: &Connection,
    identity: SessionIdentity,
    session_id: i64,
    changed: bool,
) -> LibraryResult<CurationMutationResult> {
    let tags = load_manual_tags(conn, session_id)?;
    let labels = load_manual_labels(conn, session_id)?;
    let workflow_state = derive_workflow_state(labels.iter().map(|label| label.name.as_str()));
    Ok(CurationMutationResult {
        changed,
        identity,
        tags,
        labels,
        workflow_state,
    })
}

fn load_manual_tags(conn: &Connection, session_id: i64) -> LibraryResult<Vec<SessionTag>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, t.kind, ta.origin
         FROM tag_assignments ta
         JOIN tags t ON t.id = ta.tag_id
         WHERE ta.object_type = ?1
           AND ta.object_id = ?2
           AND ta.origin = ?3
         ORDER BY t.name ASC",
    )?;
    let rows = stmt.query_map(
        params![OBJECT_TYPE_SESSION, session_id, MANUAL_ORIGIN],
        |row| {
            Ok(SessionTag {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: row.get(2)?,
                origin: row.get(3)?,
            })
        },
    )?;
    let mut tags = Vec::new();
    for row in rows {
        tags.push(row?);
    }
    Ok(tags)
}

fn load_manual_labels(conn: &Connection, session_id: i64) -> LibraryResult<Vec<SessionLabel>> {
    let mut stmt = conn.prepare(
        "SELECT l.id, l.name, l.scope, la.origin
         FROM label_assignments la
         JOIN labels l ON l.id = la.label_id
         WHERE la.object_type = ?1
           AND la.object_id = ?2
           AND la.origin = ?3
         ORDER BY l.name ASC",
    )?;
    let rows = stmt.query_map(
        params![OBJECT_TYPE_SESSION, session_id, MANUAL_ORIGIN],
        |row| {
            Ok(SessionLabel {
                id: row.get(0)?,
                name: row.get(1)?,
                scope: row.get(2)?,
                origin: row.get(3)?,
            })
        },
    )?;
    let mut labels = Vec::new();
    for row in rows {
        labels.push(row?);
    }
    Ok(labels)
}
