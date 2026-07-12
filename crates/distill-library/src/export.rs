//! Preview and publish recoverable `distill-session-jsonl-v1` exports.
//!
//! Preview derives a current-projection eligibility snapshot with no filesystem,
//! export-row, or Activity side effects. Publish shares that policy and writes a
//! Library-owned JSONL artifact through the durable `preparing` → `committed` →
//! rename → `published` lifecycle.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{LibraryError, LibraryResult};
use crate::storage::{set_file_mode_600, DistillPaths};
use crate::types::{
    ExportDataset, ExportOmission, ExportOmissionReason, ExportPreview, ExportProgress,
    ExportProgressControl, ExportResult, ExportStatus, SessionIdentity, EXPORT_FORMAT_ID,
};

const MANUAL_ORIGIN: &str = "manual";
const OBJECT_TYPE_SESSION: &str = "session";

/// One projected message loaded for JSONL emission.
#[derive(Clone, Debug)]
struct ExportMessageRow {
    ordinal: i64,
    role: String,
    text: String,
    created_at: Option<String>,
    message_kind: String,
    metadata_json: String,
}

/// Current-projection candidate carrying manual curation labels.
#[derive(Clone, Debug)]
struct DatasetCandidate {
    session_id: i64,
    identity: SessionIdentity,
    label_names: Vec<String>,
}

/// Shared eligibility snapshot used by preview and publish.
#[derive(Clone, Debug)]
struct EligibilitySnapshot {
    eligible: Vec<DatasetCandidate>,
    omitted: Vec<ExportOmission>,
}

/// Durable fields needed to reconcile one incomplete export row on open.
struct IncompleteExportRow {
    id: i64,
    dataset: String,
    temp_path: Option<String>,
    output_path: Option<String>,
    sha256: Option<String>,
    byte_size: Option<i64>,
    record_count: i64,
    created_at: String,
}

/// Wire shape for one deterministic JSONL session record.
#[derive(Serialize)]
struct ExportSessionLine {
    exported_at: String,
    source: String,
    external_session_id: String,
    title: Option<String>,
    project_path: Option<String>,
    updated_at: Option<String>,
    started_at: Option<String>,
    source_url: Option<String>,
    summary: Option<String>,
    metadata: Value,
    labels: Vec<String>,
    tags: Vec<String>,
    messages: Vec<ExportMessageLine>,
    turn_pairs: Vec<TurnPairLine>,
}

/// Wire shape for one projected export message.
#[derive(Serialize)]
struct ExportMessageLine {
    ordinal: i64,
    role: String,
    text: String,
    created_at: Option<String>,
    message_kind: String,
    metadata: Value,
}

/// Canonical turn-pair wire shape.
#[derive(Serialize)]
struct TurnPairLine {
    user: String,
    assistant: String,
}

/**
 * Preview dataset export eligibility without filesystem or Activity side effects.
 *
 * Parameters:
 * - `conn`: open Distill SQLite connection.
 * - `dataset`: approved `train` or `holdout` target.
 */
pub(crate) fn preview_export(
    conn: &Connection,
    dataset: ExportDataset,
) -> LibraryResult<ExportPreview> {
    let snapshot = build_eligibility_snapshot(conn, dataset)?;
    Ok(ExportPreview {
        dataset,
        format_id: EXPORT_FORMAT_ID.to_string(),
        eligible: snapshot
            .eligible
            .into_iter()
            .map(|row| row.identity)
            .collect(),
        omitted: snapshot.omitted,
    })
}

/**
 * Publish a recoverable Library-owned JSONL export for `dataset`.
 *
 * Parameters:
 * - `conn`: open Distill SQLite connection.
 * - `paths`: Distill home paths including the exports directory.
 * - `dataset`: approved `train` or `holdout` target.
 * - `on_progress`: typed progress observer that may request cancellation.
 */
pub(crate) fn publish_export<F>(
    conn: &mut Connection,
    paths: &DistillPaths,
    dataset: ExportDataset,
    mut on_progress: F,
) -> LibraryResult<ExportResult>
where
    F: FnMut(ExportProgress) -> ExportProgressControl,
{
    fs::create_dir_all(&paths.exports)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&paths.exports, fs::Permissions::from_mode(0o700))?;
    }

    let snapshot = build_eligibility_snapshot(conn, dataset)?;
    let exported_at = chrono::Utc::now().to_rfc3339();
    let eligibility_json = snapshot_json(&snapshot, dataset, &exported_at)?;
    let timestamp_stem = exported_at.replace([':', '.'], "-");
    let file_stem = format!("{}-sessions-{}", dataset.as_str(), timestamp_stem);
    let output_path = paths.exports.join(format!("{file_stem}.jsonl"));
    let temp_path = paths.exports.join(format!("{file_stem}.jsonl.tmp"));
    let output_display = path_to_string(&output_path)?;
    let temp_display = path_to_string(&temp_path)?;

    let export_id = insert_preparing_row(
        conn,
        dataset,
        &exported_at,
        &temp_display,
        &output_display,
        &eligibility_json,
        snapshot.eligible.len() as i64,
    )?;
    if wants_cancel(on_progress(ExportProgress::Preparing { export_id })) {
        return terminalize_cancelled(conn, export_id, dataset, &snapshot, Some(&temp_path));
    }

    if wants_cancel(on_progress(ExportProgress::Writing {
        export_id,
        records_written: 0,
        record_total: snapshot.eligible.len() as u64,
    })) {
        return terminalize_cancelled(conn, export_id, dataset, &snapshot, Some(&temp_path));
    }

    let write_outcome = write_temp_jsonl(
        conn,
        &temp_path,
        &snapshot.eligible,
        &exported_at,
        export_id,
        &mut on_progress,
    );

    let write_outcome = match write_outcome {
        Ok(outcome) => outcome,
        Err(err) => {
            let _ = terminalize_failed(
                conn,
                export_id,
                Some(&temp_path),
                "export_write_failed",
                &err,
            );
            return Err(err);
        }
    };

    if write_outcome.cancelled {
        return terminalize_cancelled(conn, export_id, dataset, &snapshot, Some(&temp_path));
    }

    #[cfg(feature = "test-faults")]
    crate::faults::check(crate::faults::FaultPoint::AfterExportTempWrite)?;

    mark_committed(
        conn,
        export_id,
        &write_outcome.sha256,
        write_outcome.byte_size,
        write_outcome.record_count,
    )?;
    on_progress(ExportProgress::Committed { export_id });

    if wants_cancel(on_progress(ExportProgress::Writing {
        export_id,
        records_written: write_outcome.record_count,
        record_total: snapshot.eligible.len() as u64,
    })) {
        return terminalize_cancelled(conn, export_id, dataset, &snapshot, Some(&temp_path));
    }

    #[cfg(feature = "test-faults")]
    crate::faults::check(crate::faults::FaultPoint::AfterExportCommittedBeforeRename)?;

    if let Err(err) = fs::rename(&temp_path, &output_path) {
        let library_err = LibraryError::Io(err);
        let _ = terminalize_failed(
            conn,
            export_id,
            Some(&temp_path),
            "export_rename_failed",
            &library_err,
        );
        return Err(library_err);
    }
    if let Err(err) = set_file_mode_600(&output_path) {
        let _ = mark_failed_after_rename(conn, export_id, &err);
        return Err(err);
    }
    on_progress(ExportProgress::Renamed { export_id });

    #[cfg(feature = "test-faults")]
    crate::faults::check(crate::faults::FaultPoint::AfterExportRenameBeforeFinalization)?;

    if let Err(err) = finalize_published(
        conn,
        PublishFinalization {
            export_id,
            dataset,
            output_path: &output_display,
            sha256: &write_outcome.sha256,
            byte_size: write_outcome.byte_size,
            record_count: write_outcome.record_count,
            exported_at: &exported_at,
        },
    ) {
        let _ = mark_failed_after_rename(conn, export_id, &err);
        return Err(err);
    }
    on_progress(ExportProgress::Published { export_id });

    Ok(ExportResult {
        export_id,
        dataset,
        format_id: EXPORT_FORMAT_ID.to_string(),
        status: ExportStatus::Published,
        output_path: Some(output_display),
        sha256: Some(write_outcome.sha256),
        byte_size: Some(write_outcome.byte_size),
        record_count: write_outcome.record_count,
        eligible: snapshot
            .eligible
            .into_iter()
            .map(|row| row.identity)
            .collect(),
        omitted: snapshot.omitted,
        error_class: None,
        error_message: None,
    })
}

/**
 * Classify incomplete export rows and remove disposable temp files on open.
 *
 * Never invents `export_written` or deletes final published artifacts.
 *
 * Parameters:
 * - `conn`: open Distill SQLite connection.
 * - `paths`: Distill home paths.
 */
pub(crate) fn reconcile_incomplete_exports(
    conn: &mut Connection,
    paths: &DistillPaths,
) -> LibraryResult<(u64, u64)> {
    let mut classified = 0_u64;
    let mut removed_temps = 0_u64;
    let now = chrono::Utc::now().to_rfc3339();

    let incomplete: Vec<IncompleteExportRow> = {
        let mut stmt = conn.prepare(
            "SELECT id, dataset, temp_path, output_path, sha256, byte_size,
                    record_count, created_at
             FROM exports
             WHERE status IN ('preparing', 'committed')
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(IncompleteExportRow {
                id: row.get(0)?,
                dataset: row.get(1)?,
                temp_path: row.get(2)?,
                output_path: row.get(3)?,
                sha256: row.get(4)?,
                byte_size: row.get(5)?,
                record_count: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out
    };

    for row in incomplete {
        let dataset = ExportDataset::parse(&row.dataset).map_err(LibraryError::InvalidArgument)?;
        if let (Some(output), Some(expected_sha), Some(expected_bytes)) = (
            row.output_path.as_deref(),
            row.sha256.as_deref(),
            row.byte_size,
        ) {
            if verify_export_file(Path::new(output), expected_sha, expected_bytes)? {
                finalize_recovered_export(conn, &row, dataset)?;
                if let Some(temp) = row.temp_path.as_deref() {
                    if remove_disposable_temp(Path::new(temp), paths)? {
                        removed_temps += 1;
                    }
                }
                classified += 1;
                continue;
            }
        }
        if let Some(temp) = row.temp_path.as_deref() {
            if remove_disposable_temp(Path::new(temp), paths)? {
                removed_temps += 1;
            }
        }
        conn.execute(
            "UPDATE exports
             SET status = 'failed_publish',
                 updated_at = ?1,
                 temp_path = NULL,
                 output_path = NULL,
                 error_class = COALESCE(error_class, 'incomplete_on_open'),
                 error_message = COALESCE(
                     error_message,
                     'incomplete export classified on open without inventing success'
                 )
             WHERE id = ?2 AND status IN ('preparing', 'committed')",
            params![now, row.id],
        )?;
        classified += 1;
    }

    removed_temps += sweep_orphan_export_temps(paths)?;
    Ok((classified, removed_temps))
}

fn verify_export_file(path: &Path, expected_sha: &str, expected_bytes: i64) -> LibraryResult<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        Ok(_) => return Ok(false),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    if expected_bytes < 0 || metadata.len() != expected_bytes as u64 {
        return Ok(false);
    }
    let bytes = fs::read(path)?;
    Ok(hex::encode(Sha256::digest(bytes)) == expected_sha)
}

fn finalize_recovered_export(
    conn: &mut Connection,
    row: &IncompleteExportRow,
    dataset: ExportDataset,
) -> LibraryResult<()> {
    let output_path = row.output_path.as_deref().ok_or_else(|| {
        LibraryError::InvalidArgument("recoverable export is missing output path".into())
    })?;
    let sha256 = row.sha256.as_deref().ok_or_else(|| {
        LibraryError::InvalidArgument("recoverable export is missing checksum".into())
    })?;
    let byte_size = row.byte_size.ok_or_else(|| {
        LibraryError::InvalidArgument("recoverable export is missing byte size".into())
    })?;
    let now = chrono::Utc::now().to_rfc3339();
    let tx = conn.transaction()?;
    let changed = tx.execute(
        "UPDATE exports
         SET status = 'published', updated_at = ?1, temp_path = NULL,
             output_path = ?2, error_class = NULL, error_message = NULL
         WHERE id = ?3 AND status = 'committed'",
        params![now, output_path, row.id],
    )?;
    if changed == 0 {
        tx.commit()?;
        return Ok(());
    }
    tx.execute(
        "INSERT INTO activity_events (event_type, occurred_at, payload_json)
         VALUES ('export_written', ?1, ?2)",
        params![
            now,
            json!({
                "export_id": row.id,
                "object_type": "export",
                "object_id": row.id,
                "dataset": dataset.as_str(),
                "format_id": EXPORT_FORMAT_ID,
                "output_path": output_path,
                "sha256": sha256,
                "byte_size": byte_size,
                "record_count": row.record_count,
                "exported_at": row.created_at,
                "recovered_on_open": true,
            })
            .to_string(),
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// Outcome of writing the temporary JSONL artifact.
struct TempWriteOutcome {
    sha256: String,
    byte_size: u64,
    record_count: u64,
    cancelled: bool,
}

/**
 * Build the shared eligibility snapshot for preview and publish.
 */
fn build_eligibility_snapshot(
    conn: &Connection,
    dataset: ExportDataset,
) -> LibraryResult<EligibilitySnapshot> {
    let candidates = load_dataset_candidates(conn, dataset)?;
    let mut eligible = Vec::new();
    let mut omitted = Vec::new();
    for candidate in candidates {
        match classify_candidate(&candidate.label_names, dataset) {
            CandidateClassification::Eligible => eligible.push(candidate),
            CandidateClassification::Omitted(reason) => omitted.push(ExportOmission {
                identity: candidate.identity,
                reason,
            }),
            CandidateClassification::NotTarget => {}
        }
    }
    Ok(EligibilitySnapshot { eligible, omitted })
}

/**
 * Load all current sessions so unreviewed and favorite-only sessions can be
 * reported with explicit omission reasons. Sessions assigned only to the
 * other dataset remain outside this target preview.
 */
fn load_dataset_candidates(
    conn: &Connection,
    _dataset: ExportDataset,
) -> LibraryResult<Vec<DatasetCandidate>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.source_kind, s.external_session_id
         FROM sessions s
         ORDER BY s.source_kind ASC, s.external_session_id ASC, s.id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut candidates = Vec::new();
    for row in rows {
        let (session_id, source_kind, external_session_id) = row?;
        let label_names = load_manual_label_names(conn, session_id)?;
        candidates.push(DatasetCandidate {
            session_id,
            identity: SessionIdentity {
                source_kind,
                external_session_id,
            },
            label_names,
        });
    }
    Ok(candidates)
}

enum CandidateClassification {
    Eligible,
    Omitted(ExportOmissionReason),
    NotTarget,
}

/** Apply the shared export safety policy for one current session. */
fn classify_candidate(label_names: &[String], dataset: ExportDataset) -> CandidateClassification {
    let has_exclude = label_names.iter().any(|name| name == "exclude");
    let has_sensitive = label_names.iter().any(|name| name == "sensitive");
    let has_train = label_names.iter().any(|name| name == "train");
    let has_holdout = label_names.iter().any(|name| name == "holdout");

    if has_exclude {
        return CandidateClassification::Omitted(ExportOmissionReason::Exclude);
    }
    if has_sensitive {
        return CandidateClassification::Omitted(ExportOmissionReason::Sensitive);
    }
    if has_train && has_holdout {
        return CandidateClassification::Omitted(ExportOmissionReason::ConflictingDatasetLabels);
    }
    let has_target = match dataset {
        ExportDataset::Train => has_train,
        ExportDataset::Holdout => has_holdout,
    };
    if has_target {
        return CandidateClassification::Eligible;
    }
    if has_train || has_holdout {
        return CandidateClassification::NotTarget;
    }
    if label_names.iter().any(|name| name == "favorite") {
        CandidateClassification::Omitted(ExportOmissionReason::FavoriteOnly)
    } else {
        CandidateClassification::Omitted(ExportOmissionReason::Unreviewed)
    }
}

/**
 * Write eligible sessions to a temporary JSONL file and hash the bytes.
 */
fn write_temp_jsonl<F>(
    conn: &Connection,
    temp_path: &Path,
    eligible: &[DatasetCandidate],
    exported_at: &str,
    export_id: i64,
    on_progress: &mut F,
) -> LibraryResult<TempWriteOutcome>
where
    F: FnMut(ExportProgress) -> ExportProgressControl,
{
    if temp_path.exists() {
        fs::remove_file(temp_path)?;
    }
    let file = File::create(temp_path)?;
    set_file_mode_600(temp_path)?;
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut byte_size = 0_u64;
    let mut record_count = 0_u64;
    let total = eligible.len() as u64;

    for candidate in eligible {
        if wants_cancel(on_progress(ExportProgress::Writing {
            export_id,
            records_written: record_count,
            record_total: total,
        })) {
            writer.flush()?;
            return Ok(TempWriteOutcome {
                sha256: hex::encode(hasher.finalize()),
                byte_size,
                record_count,
                cancelled: true,
            });
        }

        let line = build_session_line(conn, candidate, exported_at)?;
        let encoded = serde_json::to_string(&line)?;
        writer.write_all(encoded.as_bytes())?;
        writer.write_all(b"\n")?;
        hasher.update(encoded.as_bytes());
        hasher.update(b"\n");
        byte_size += encoded.len() as u64 + 1;
        record_count += 1;
    }

    writer.flush()?;
    writer.get_ref().sync_all()?;
    drop(writer);

    Ok(TempWriteOutcome {
        sha256: hex::encode(hasher.finalize()),
        byte_size,
        record_count,
        cancelled: false,
    })
}

/**
 * Build one deterministic JSONL object for an eligible session.
 */
fn build_session_line(
    conn: &Connection,
    candidate: &DatasetCandidate,
    exported_at: &str,
) -> LibraryResult<ExportSessionLine> {
    let (
        title,
        project_path,
        source_url,
        summary,
        started_at,
        updated_at,
        metadata_json,
        generation,
    ) = conn.query_row(
        "SELECT title, project_path, source_url, summary, started_at, updated_at,
                    metadata_json, successful_projection_generation
             FROM sessions WHERE id = ?1",
        [candidate.session_id],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
            ))
        },
    )?;

    let messages = load_projection_messages(conn, candidate.session_id, generation)?;
    let tags = load_manual_tag_names(conn, candidate.session_id)?;
    let turn_pairs = build_turn_pairs(&messages);
    let message_lines = messages
        .into_iter()
        .map(|message| ExportMessageLine {
            ordinal: message.ordinal,
            role: message.role,
            text: message.text,
            created_at: message.created_at,
            message_kind: message.message_kind,
            metadata: parse_json_object(&message.metadata_json),
        })
        .collect();

    Ok(ExportSessionLine {
        exported_at: exported_at.to_string(),
        source: candidate.identity.source_kind.clone(),
        external_session_id: candidate.identity.external_session_id.clone(),
        title,
        project_path,
        updated_at,
        started_at,
        source_url,
        summary,
        metadata: parse_json_object(&metadata_json),
        labels: candidate.label_names.clone(),
        tags,
        messages: message_lines,
        turn_pairs,
    })
}

/**
 * Load current-projection messages in ordinal order.
 */
fn load_projection_messages(
    conn: &Connection,
    session_id: i64,
    generation: i64,
) -> LibraryResult<Vec<ExportMessageRow>> {
    let mut stmt = conn.prepare(
        "SELECT ordinal, role, text, created_at, message_kind, metadata_json
         FROM projection_messages
         WHERE session_id = ?1 AND projection_generation = ?2
         ORDER BY ordinal ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![session_id, generation], |row| {
        Ok(ExportMessageRow {
            ordinal: row.get(0)?,
            role: row.get(1)?,
            text: row.get(2)?,
            created_at: row.get(3)?,
            message_kind: row.get(4)?,
            metadata_json: row.get(5)?,
        })
    })?;
    let mut messages = Vec::new();
    for row in rows {
        messages.push(row?);
    }
    Ok(messages)
}

/**
 * Derive canonical turn pairs from projected messages.
 */
fn build_turn_pairs(messages: &[ExportMessageRow]) -> Vec<TurnPairLine> {
    let mut pairs = Vec::new();
    let mut pending_user: Option<String> = None;
    for message in messages {
        if message.role == "user" {
            pending_user = Some(message.text.clone());
            continue;
        }
        if message.role == "assistant" && message.message_kind == "meta" {
            continue;
        }
        if message.role == "assistant" {
            if let Some(user) = pending_user.take() {
                pairs.push(TurnPairLine {
                    user,
                    assistant: message.text.clone(),
                });
            }
        }
    }
    pairs
}

fn load_manual_label_names(conn: &Connection, session_id: i64) -> LibraryResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT l.name
         FROM label_assignments la
         JOIN labels l ON l.id = la.label_id
         WHERE la.object_type = ?1 AND la.object_id = ?2 AND la.origin = ?3
         ORDER BY l.name ASC",
    )?;
    let rows = stmt.query_map(
        params![OBJECT_TYPE_SESSION, session_id, MANUAL_ORIGIN],
        |row| row.get::<_, String>(0),
    )?;
    let mut names = Vec::new();
    for row in rows {
        names.push(row?);
    }
    Ok(names)
}

fn load_manual_tag_names(conn: &Connection, session_id: i64) -> LibraryResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT t.name
         FROM tag_assignments ta
         JOIN tags t ON t.id = ta.tag_id
         WHERE ta.object_type = ?1 AND ta.object_id = ?2 AND ta.origin = ?3
         ORDER BY t.name ASC",
    )?;
    let rows = stmt.query_map(
        params![OBJECT_TYPE_SESSION, session_id, MANUAL_ORIGIN],
        |row| row.get::<_, String>(0),
    )?;
    let mut names = Vec::new();
    for row in rows {
        names.push(row?);
    }
    Ok(names)
}

fn parse_json_object(raw: &str) -> Value {
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(map)) => Value::Object(map),
        _ => json!({}),
    }
}

fn snapshot_json(
    snapshot: &EligibilitySnapshot,
    dataset: ExportDataset,
    exported_at: &str,
) -> LibraryResult<String> {
    Ok(serde_json::to_string(&json!({
        "format_id": EXPORT_FORMAT_ID,
        "dataset": dataset.as_str(),
        "exported_at": exported_at,
        "eligible": snapshot.eligible.iter().map(|row| json!({
            "source_kind": row.identity.source_kind,
            "external_session_id": row.identity.external_session_id,
        })).collect::<Vec<_>>(),
        "omitted": snapshot.omitted.iter().map(|row| json!({
            "source_kind": row.identity.source_kind,
            "external_session_id": row.identity.external_session_id,
            "reason": row.reason.as_str(),
        })).collect::<Vec<_>>(),
    }))?)
}

fn insert_preparing_row(
    conn: &Connection,
    dataset: ExportDataset,
    exported_at: &str,
    temp_path: &str,
    output_path: &str,
    eligibility_json: &str,
    record_count: i64,
) -> LibraryResult<i64> {
    conn.execute(
        "INSERT INTO exports (
            format_id, dataset, status, created_at, updated_at, temp_path,
            output_path, record_count, eligibility_snapshot_json
         ) VALUES (?1, ?2, 'preparing', ?3, ?3, ?4, ?5, ?6, ?7)",
        params![
            EXPORT_FORMAT_ID,
            dataset.as_str(),
            exported_at,
            temp_path,
            output_path,
            record_count,
            eligibility_json,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn mark_committed(
    conn: &Connection,
    export_id: i64,
    sha256: &str,
    byte_size: u64,
    record_count: u64,
) -> LibraryResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE exports
         SET status = 'committed',
             updated_at = ?1,
             sha256 = ?2,
             byte_size = ?3,
             record_count = ?4
         WHERE id = ?5 AND status = 'preparing'",
        params![
            now,
            sha256,
            byte_size as i64,
            record_count as i64,
            export_id
        ],
    )?;
    Ok(())
}

/// Inputs required to finalize a committed export as `published`.
struct PublishFinalization<'a> {
    export_id: i64,
    dataset: ExportDataset,
    output_path: &'a str,
    sha256: &'a str,
    byte_size: u64,
    record_count: u64,
    exported_at: &'a str,
}

fn finalize_published(conn: &mut Connection, args: PublishFinalization<'_>) -> LibraryResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE exports
         SET status = 'published',
             updated_at = ?1,
             temp_path = NULL,
             output_path = ?2,
             sha256 = ?3,
             byte_size = ?4,
             record_count = ?5,
             error_class = NULL,
             error_message = NULL
         WHERE id = ?6 AND status = 'committed'",
        params![
            now,
            args.output_path,
            args.sha256,
            args.byte_size as i64,
            args.record_count as i64,
            args.export_id
        ],
    )?;
    tx.execute(
        "INSERT INTO activity_events (
            event_type, occurred_at, source_kind, session_id, capture_id, attempt_id, payload_json
         ) VALUES ('export_written', ?1, NULL, NULL, NULL, NULL, ?2)",
        params![
            now,
            json!({
                "export_id": args.export_id,
                "object_type": "export",
                "object_id": args.export_id,
                "dataset": args.dataset.as_str(),
                "format_id": EXPORT_FORMAT_ID,
                "output_path": args.output_path,
                "sha256": args.sha256,
                "byte_size": args.byte_size,
                "record_count": args.record_count,
                "exported_at": args.exported_at,
            })
            .to_string(),
        ],
    )?;
    #[cfg(feature = "test-faults")]
    crate::faults::check(crate::faults::FaultPoint::DuringExportFinalizationBeforeCommit)?;
    tx.commit()?;
    Ok(())
}

fn terminalize_cancelled(
    conn: &Connection,
    export_id: i64,
    dataset: ExportDataset,
    snapshot: &EligibilitySnapshot,
    temp_path: Option<&Path>,
) -> LibraryResult<ExportResult> {
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(path) = temp_path {
        let _ = fs::remove_file(path);
    }
    conn.execute(
        "UPDATE exports
         SET status = 'cancelled',
             updated_at = ?1,
             temp_path = NULL,
             output_path = NULL,
             error_class = 'cancelled',
             error_message = 'export cancelled at a safe checkpoint'
         WHERE id = ?2 AND status IN ('preparing', 'committed')",
        params![now, export_id],
    )?;
    Ok(ExportResult {
        export_id,
        dataset,
        format_id: EXPORT_FORMAT_ID.to_string(),
        status: ExportStatus::Cancelled,
        output_path: None,
        sha256: None,
        byte_size: None,
        record_count: 0,
        eligible: snapshot
            .eligible
            .iter()
            .map(|row| row.identity.clone())
            .collect(),
        omitted: snapshot.omitted.clone(),
        error_class: Some("cancelled".into()),
        error_message: Some("export cancelled at a safe checkpoint".into()),
    })
}

fn terminalize_failed(
    conn: &Connection,
    export_id: i64,
    temp_path: Option<&Path>,
    error_class: &str,
    err: &LibraryError,
) -> LibraryResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(path) = temp_path {
        let _ = fs::remove_file(path);
    }
    conn.execute(
        "UPDATE exports
         SET status = 'failed_publish',
             updated_at = ?1,
             temp_path = NULL,
             output_path = NULL,
             error_class = ?2,
             error_message = ?3
         WHERE id = ?4 AND status IN ('preparing', 'committed')",
        params![now, error_class, err.to_string(), export_id],
    )?;
    Ok(())
}

fn mark_failed_after_rename(
    conn: &Connection,
    export_id: i64,
    err: &LibraryError,
) -> LibraryResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE exports
         SET updated_at = ?1,
             temp_path = NULL,
             error_class = 'export_finalize_failed',
             error_message = ?2
         WHERE id = ?3 AND status = 'committed'",
        params![now, err.to_string(), export_id],
    )?;
    Ok(())
}

fn wants_cancel(control: ExportProgressControl) -> bool {
    matches!(control, ExportProgressControl::Cancel)
}

fn path_to_string(path: &Path) -> LibraryResult<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| LibraryError::InvalidArgument("export path is not valid UTF-8".into()))
}

fn remove_disposable_temp(path: &Path, paths: &DistillPaths) -> LibraryResult<bool> {
    if !is_disposable_export_temp(path, paths) {
        return Ok(false);
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => {
            fs::remove_file(path)?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.into()),
    }
}

fn sweep_orphan_export_temps(paths: &DistillPaths) -> LibraryResult<u64> {
    let safe_dir = match fs::symlink_metadata(&paths.exports) {
        Ok(meta) => !meta.file_type().is_symlink() && meta.is_dir(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err.into()),
    };
    if !safe_dir {
        return Ok(0);
    }
    let mut removed = 0_u64;
    for entry in fs::read_dir(&paths.exports)? {
        let entry = entry?;
        let path = entry.path();
        if is_disposable_export_temp(&path, paths) {
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            if meta.file_type().is_file() {
                fs::remove_file(&path)?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

fn is_disposable_export_temp(path: &Path, paths: &DistillPaths) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !name.ends_with(".jsonl.tmp") {
        return false;
    }
    path.parent()
        .is_some_and(|parent| parent == paths.exports.as_path())
}
