use std::collections::HashMap;
use std::fs;

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;
use serde_json::{Value, json};
use sha1::{Digest as Sha1Digest, Sha1};

use crate::connectors::{
    DiscoveredCapture, DiscoveredSource, ImportFailureEntry, ImportSourceSummary,
    NormalizedArtifact, NormalizedMessage, NormalizedSession, ParsedCapture, ParsedCaptureRecord,
    SourceConnector, SourceKind, configured_rust_connectors,
};

use super::{RustStore, connection};

const INLINE_CAPTURE_MAX_BYTES: usize = 64 * 1024;
const PARSER_VERSION: &str = "v0";

#[derive(Clone, Debug, Default, Serialize)]
pub struct SyncReport {
    pub reason: String,
    pub started_at: String,
    pub finished_at: String,
    pub summary: String,
    pub outcome: String,
    pub discovered_captures: usize,
    pub imported_captures: usize,
    pub skipped_captures: usize,
    pub failed_captures: usize,
    pub source_summaries: Vec<ImportSourceSummary>,
    pub failed_entries: Vec<ImportFailureEntry>,
    pub job_id: i64,
}

#[derive(Clone, Debug, Default)]
struct SyncAccumulator {
    discovered_captures: usize,
    imported_captures: usize,
    skipped_captures: usize,
    failed_captures: usize,
    source_summaries: Vec<ImportSourceSummary>,
    failed_entries: Vec<ImportFailureEntry>,
}

#[derive(Clone, Debug)]
struct ExistingCapture {
    id: i64,
    status: String,
}

#[derive(Clone, Debug)]
struct PersistedCaptureContent {
    raw_blob_path: Option<String>,
    raw_payload_json: String,
}

#[derive(Clone, Debug, Default, Serialize)]
struct SyncJobPayload {
    reason: String,
    summary: String,
    #[serde(rename = "startedAt")]
    started_at: String,
    #[serde(rename = "finishedAt", skip_serializing_if = "Option::is_none")]
    finished_at: Option<String>,
    #[serde(rename = "discoveredCaptures")]
    discovered_captures: usize,
    #[serde(rename = "importedCaptures")]
    imported_captures: usize,
    #[serde(rename = "skippedCaptures")]
    skipped_captures: usize,
    #[serde(rename = "failedCaptures")]
    failed_captures: usize,
    outcome: String,
    #[serde(
        rename = "sourceSummaries",
        skip_serializing_if = "Vec::is_empty",
        default
    )]
    source_summaries: Vec<ImportSourceSummary>,
    #[serde(
        rename = "failedEntries",
        skip_serializing_if = "Vec::is_empty",
        default
    )]
    failed_entries: Vec<ImportFailureEntry>,
}

impl RustStore {
    pub fn sync_now(&self, reason: &str) -> Result<SyncReport> {
        self.sync_with_connectors(configured_rust_connectors()?, reason)
    }

    #[cfg(test)]
    pub(crate) fn sync_codex_from_home(
        &self,
        codex_home: std::path::PathBuf,
        reason: &str,
    ) -> Result<SyncReport> {
        self.sync_with_connectors(
            vec![Box::new(crate::connectors::CodexConnector::new(codex_home))],
            reason,
        )
    }

    #[cfg(test)]
    pub(crate) fn sync_codex_and_claude_from_homes(
        &self,
        codex_home: std::path::PathBuf,
        claude_home: std::path::PathBuf,
        reason: &str,
    ) -> Result<SyncReport> {
        self.sync_with_connectors(
            vec![
                Box::new(crate::connectors::CodexConnector::new(codex_home)),
                Box::new(crate::connectors::ClaudeCodeConnector::new(claude_home)),
            ],
            reason,
        )
    }

    fn sync_with_connectors(
        &self,
        connectors: Vec<Box<dyn SourceConnector>>,
        reason: &str,
    ) -> Result<SyncReport> {
        let mut connection = connection::open_read_write(self.database_path())?;
        let started_at = now_iso();
        let job_id = insert_sync_job(&connection, reason, &started_at)?;
        insert_activity_event(
            &connection,
            "sync_started",
            "sync_job",
            Some(job_id),
            None,
            json!({
                "reason": reason,
                "jobType": "sync_sources"
            }),
        )?;

        let outcome = self.run_sync(&mut connection, &connectors);
        match outcome {
            Ok(mut accumulator) => {
                let finished_at = now_iso();
                let status = classify_sync_status(&accumulator);
                let summary = summarize_sync(&accumulator);
                let payload = SyncJobPayload {
                    reason: reason.to_string(),
                    summary: summary.clone(),
                    started_at: started_at.clone(),
                    finished_at: Some(finished_at.clone()),
                    discovered_captures: accumulator.discovered_captures,
                    imported_captures: accumulator.imported_captures,
                    skipped_captures: accumulator.skipped_captures,
                    failed_captures: accumulator.failed_captures,
                    outcome: status.to_string(),
                    source_summaries: std::mem::take(&mut accumulator.source_summaries),
                    failed_entries: std::mem::take(&mut accumulator.failed_entries),
                };
                update_sync_job(&connection, job_id, status, &payload, None)?;
                insert_activity_event(
                    &connection,
                    if status == "failed" {
                        "sync_failed"
                    } else {
                        "sync_completed"
                    },
                    "sync_job",
                    Some(job_id),
                    None,
                    json!({
                        "reason": reason,
                        "jobType": "sync_sources",
                        "outcome": status,
                        "discoveredCaptures": payload.discovered_captures,
                        "importedCaptures": payload.imported_captures,
                        "skippedCaptures": payload.skipped_captures,
                        "failedCaptures": payload.failed_captures
                    }),
                )?;

                Ok(SyncReport {
                    reason: reason.to_string(),
                    started_at,
                    finished_at,
                    summary,
                    outcome: status.to_string(),
                    discovered_captures: payload.discovered_captures,
                    imported_captures: payload.imported_captures,
                    skipped_captures: payload.skipped_captures,
                    failed_captures: payload.failed_captures,
                    source_summaries: payload.source_summaries,
                    failed_entries: payload.failed_entries,
                    job_id,
                })
            }
            Err(error) => {
                let finished_at = now_iso();
                let error_text = error.to_string();
                let payload = SyncJobPayload {
                    reason: reason.to_string(),
                    summary: format!("Codex sync failed: {error_text}"),
                    started_at: started_at.clone(),
                    finished_at: Some(finished_at.clone()),
                    discovered_captures: 0,
                    imported_captures: 0,
                    skipped_captures: 0,
                    failed_captures: 1,
                    outcome: "failed".to_string(),
                    source_summaries: Vec::new(),
                    failed_entries: vec![ImportFailureEntry {
                        source_kind: "sync_sources".to_string(),
                        source_path: "sync_sources".to_string(),
                        error_text: error_text.clone(),
                    }],
                };
                update_sync_job(&connection, job_id, "failed", &payload, Some(&error_text))?;
                insert_activity_event(
                    &connection,
                    "sync_failed",
                    "sync_job",
                    Some(job_id),
                    None,
                    json!({
                        "reason": reason,
                        "jobType": "sync_sources",
                        "errorText": error_text,
                        "scope": "job",
                        "fatal": true
                    }),
                )?;
                Err(error)
            }
        }
    }

    fn run_sync(
        &self,
        connection: &mut Connection,
        connectors: &[Box<dyn SourceConnector>],
    ) -> Result<SyncAccumulator> {
        let mut accumulator = SyncAccumulator::default();

        for connector in connectors {
            let source = match connector.detect() {
                Ok(source) => source,
                Err(error) => {
                    let error_text = error.to_string();
                    accumulator.failed_entries.push(ImportFailureEntry {
                        source_kind: connector.kind().as_str().to_string(),
                        source_path: connector.kind().as_str().to_string(),
                        error_text: error_text.clone(),
                    });
                    accumulator.source_summaries.push(ImportSourceSummary {
                        kind: connector.kind().as_str().to_string(),
                        discovered_captures: 0,
                        imported_captures: 0,
                        skipped_captures: 0,
                        failed_captures: 0,
                    });
                    insert_sync_source_failure_activity(
                        connection,
                        connector.kind().as_str(),
                        "detect",
                        connector.kind().as_str(),
                        &error_text,
                    )?;
                    continue;
                }
            };

            let source_id = upsert_source(connection, &source)?;
            let captures = match connector.discover_captures() {
                Ok(captures) => captures,
                Err(error) => {
                    let error_text = error.to_string();
                    accumulator.failed_entries.push(ImportFailureEntry {
                        source_kind: source.kind.as_str().to_string(),
                        source_path: source
                            .data_root
                            .as_ref()
                            .map(|value| value.display().to_string())
                            .unwrap_or_else(|| source.kind.as_str().to_string()),
                        error_text: error_text.clone(),
                    });
                    accumulator.source_summaries.push(ImportSourceSummary {
                        kind: source.kind.as_str().to_string(),
                        discovered_captures: 0,
                        imported_captures: 0,
                        skipped_captures: 0,
                        failed_captures: 0,
                    });
                    insert_sync_source_failure_activity(
                        connection,
                        source.kind.as_str(),
                        "discover",
                        source
                            .data_root
                            .as_ref()
                            .map(|value| value.display().to_string())
                            .unwrap_or_else(|| source.kind.as_str().to_string())
                            .as_str(),
                        &error_text,
                    )?;
                    continue;
                }
            };

            let mut source_summary = ImportSourceSummary {
                kind: source.kind.as_str().to_string(),
                discovered_captures: captures.len(),
                imported_captures: 0,
                skipped_captures: 0,
                failed_captures: 0,
            };
            accumulator.discovered_captures += captures.len();

            for capture in captures {
                match self.import_capture(
                    connection,
                    connector.as_ref(),
                    source_id,
                    &source,
                    &capture,
                ) {
                    Ok(ImportCaptureStatus::Imported) => {
                        accumulator.imported_captures += 1;
                        source_summary.imported_captures += 1;
                    }
                    Ok(ImportCaptureStatus::Skipped) => {
                        accumulator.skipped_captures += 1;
                        source_summary.skipped_captures += 1;
                    }
                    Ok(ImportCaptureStatus::Failed(error_text)) => {
                        accumulator.failed_captures += 1;
                        source_summary.failed_captures += 1;
                        accumulator.failed_entries.push(ImportFailureEntry {
                            source_kind: source.kind.as_str().to_string(),
                            source_path: capture.source_path.clone(),
                            error_text,
                        });
                    }
                    Err(error) => {
                        accumulator.failed_captures += 1;
                        source_summary.failed_captures += 1;
                        accumulator.failed_entries.push(ImportFailureEntry {
                            source_kind: source.kind.as_str().to_string(),
                            source_path: capture.source_path.clone(),
                            error_text: error.to_string(),
                        });
                    }
                }
            }

            accumulator.source_summaries.push(source_summary);
        }

        Ok(accumulator)
    }

    fn import_capture(
        &self,
        connection: &mut Connection,
        connector: &dyn SourceConnector,
        source_id: i64,
        source: &DiscoveredSource,
        capture: &DiscoveredCapture,
    ) -> Result<ImportCaptureStatus> {
        let snapshot = match connector.snapshot_capture(capture) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                insert_activity_event(
                    connection,
                    "capture_failed",
                    "capture",
                    None,
                    None,
                    json!({
                        "sourceKind": source.kind.as_str(),
                        "sourcePath": capture.source_path,
                        "externalSessionId": capture.external_session_id,
                        "stage": "snapshot",
                        "errorText": error.to_string()
                    }),
                )?;
                return Ok(ImportCaptureStatus::Failed(error.to_string()));
            }
        };

        let existing = find_capture(
            connection,
            source_id,
            &capture.source_path,
            &snapshot.raw_sha256,
        )?;
        if let Some(existing) = existing.as_ref() {
            if matches!(existing.status.as_str(), "normalized" | "failed_parse") {
                return Ok(ImportCaptureStatus::Skipped);
            }
        }

        let persisted = match persist_capture_content(&self.app_paths.blobs_dir, capture, &snapshot)
        {
            Ok(persisted) => persisted,
            Err(error) => {
                insert_activity_event(
                    connection,
                    "capture_failed",
                    "capture",
                    None,
                    None,
                    json!({
                        "sourceKind": source.kind.as_str(),
                        "sourcePath": capture.source_path,
                        "externalSessionId": capture.external_session_id,
                        "stage": "persistence",
                        "errorText": error.to_string()
                    }),
                )?;
                return Ok(ImportCaptureStatus::Failed(error.to_string()));
            }
        };

        let capture_id = if let Some(existing) = existing {
            update_capture_payload(connection, existing.id, capture, &snapshot, &persisted)?;
            existing.id
        } else {
            insert_capture(connection, source_id, capture, &snapshot, &persisted)?
        };

        let parsed = match connector.parse_capture(capture, &snapshot) {
            Ok(parsed) => parsed,
            Err(error) => {
                update_capture_failure(connection, capture_id, &error.to_string())?;
                insert_activity_event(
                    connection,
                    "capture_failed",
                    "capture",
                    Some(capture_id),
                    None,
                    json!({
                        "sourceKind": source.kind.as_str(),
                        "sourcePath": capture.source_path,
                        "externalSessionId": capture.external_session_id,
                        "stage": "parse",
                        "errorText": error.to_string()
                    }),
                )?;
                return Ok(ImportCaptureStatus::Failed(error.to_string()));
            }
        };

        replace_session_projection(connection, source_id, capture_id, source, capture, &parsed)?;
        Ok(ImportCaptureStatus::Imported)
    }
}

enum ImportCaptureStatus {
    Imported,
    Skipped,
    Failed(String),
}

fn insert_sync_job(connection: &Connection, reason: &str, started_at: &str) -> Result<i64> {
    let payload = SyncJobPayload {
        reason: reason.to_string(),
        summary: format!("Sync running: {reason}"),
        started_at: started_at.to_string(),
        finished_at: None,
        discovered_captures: 0,
        imported_captures: 0,
        skipped_captures: 0,
        failed_captures: 0,
        outcome: "running".to_string(),
        source_summaries: Vec::new(),
        failed_entries: Vec::new(),
    };
    connection.execute(
        r#"
        INSERT INTO jobs (
          job_type,
          object_type,
          object_id,
          status,
          run_after,
          payload_json,
          updated_at
        ) VALUES ('sync_sources', 'system', 1, 'running', CURRENT_TIMESTAMP, ?1, CURRENT_TIMESTAMP)
        "#,
        [serde_json::to_string(&payload)?],
    )?;
    Ok(connection.last_insert_rowid())
}

fn insert_sync_source_failure_activity(
    connection: &Connection,
    source_kind: &str,
    stage: &str,
    source_path: &str,
    error_text: &str,
) -> Result<()> {
    insert_activity_event(
        connection,
        "sync_failed",
        "sync_job",
        None,
        None,
        json!({
            "sourceKind": source_kind,
            "stage": stage,
            "sourcePath": source_path,
            "errorText": error_text,
            "fatal": false,
            "scope": "source"
        }),
    )
}

fn update_sync_job(
    connection: &Connection,
    job_id: i64,
    status: &str,
    payload: &SyncJobPayload,
    last_error: Option<&str>,
) -> Result<()> {
    connection.execute(
        r#"
        UPDATE jobs
        SET status = ?1,
            last_error = ?2,
            payload_json = ?3,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = ?4
        "#,
        params![status, last_error, serde_json::to_string(payload)?, job_id],
    )?;
    Ok(())
}

fn upsert_source(connection: &Connection, source: &DiscoveredSource) -> Result<i64> {
    let mut metadata = source.metadata.clone();
    metadata.insert("checks".to_string(), serde_json::to_value(&source.checks)?);
    connection.execute(
        r#"
        INSERT INTO sources (
          kind,
          display_name,
          executable_path,
          data_root,
          install_status,
          detected_at,
          metadata_json,
          updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)
        ON CONFLICT(kind) DO UPDATE SET
          display_name = excluded.display_name,
          executable_path = excluded.executable_path,
          data_root = excluded.data_root,
          install_status = excluded.install_status,
          detected_at = excluded.detected_at,
          metadata_json = excluded.metadata_json,
          updated_at = CURRENT_TIMESTAMP
        "#,
        params![
            source.kind.as_str(),
            source.display_name,
            source
                .executable_path
                .as_ref()
                .map(|value| value.display().to_string()),
            source
                .data_root
                .as_ref()
                .map(|value| value.display().to_string()),
            source.install_status.as_str(),
            now_iso(),
            serde_json::to_string(&metadata)?,
        ],
    )?;
    let source_id = connection.query_row(
        "SELECT id FROM sources WHERE kind = ?1",
        [source.kind.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(source_id)
}

fn find_capture(
    connection: &Connection,
    source_id: i64,
    source_path: &str,
    raw_sha256: &str,
) -> Result<Option<ExistingCapture>> {
    connection
        .query_row(
            r#"
            SELECT id, status
            FROM captures
            WHERE source_id = ?1 AND source_path = ?2 AND raw_sha256 = ?3
            LIMIT 1
            "#,
            params![source_id, source_path, raw_sha256],
            |row| {
                Ok(ExistingCapture {
                    id: row.get(0)?,
                    status: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn insert_capture(
    connection: &Connection,
    source_id: i64,
    capture: &DiscoveredCapture,
    snapshot: &crate::connectors::CaptureSnapshot,
    persisted: &PersistedCaptureContent,
) -> Result<i64> {
    connection.execute(
        r#"
        INSERT INTO captures (
          source_id,
          capture_kind,
          external_session_id,
          source_path,
          source_modified_at,
          source_size_bytes,
          raw_sha256,
          raw_blob_path,
          raw_payload_json,
          parser_version,
          status,
          captured_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'captured', ?11)
        "#,
        params![
            source_id,
            capture.capture_kind,
            capture.external_session_id,
            capture.source_path,
            snapshot
                .source_modified_at
                .clone()
                .or_else(|| capture.source_modified_at.clone()),
            snapshot
                .source_size_bytes
                .map(|value| value as i64)
                .or_else(|| capture.source_size_bytes.map(|value| value as i64)),
            snapshot.raw_sha256,
            persisted.raw_blob_path,
            persisted.raw_payload_json,
            PARSER_VERSION,
            now_iso(),
        ],
    )?;
    let capture_id = connection.last_insert_rowid();
    insert_activity_event(
        connection,
        "capture_recorded",
        "capture",
        Some(capture_id),
        None,
        json!({
            "sourceKind": capture.source_kind.as_str(),
            "sourcePath": capture.source_path,
            "externalSessionId": capture.external_session_id
        }),
    )?;
    Ok(capture_id)
}

fn update_capture_payload(
    connection: &Connection,
    capture_id: i64,
    capture: &DiscoveredCapture,
    snapshot: &crate::connectors::CaptureSnapshot,
    persisted: &PersistedCaptureContent,
) -> Result<()> {
    connection.execute(
        r#"
        UPDATE captures
        SET source_modified_at = ?1,
            source_size_bytes = ?2,
            raw_blob_path = ?3,
            raw_payload_json = ?4,
            error_text = NULL,
            parser_version = ?5
        WHERE id = ?6
        "#,
        params![
            snapshot
                .source_modified_at
                .clone()
                .or_else(|| capture.source_modified_at.clone()),
            snapshot
                .source_size_bytes
                .map(|value| value as i64)
                .or_else(|| capture.source_size_bytes.map(|value| value as i64)),
            persisted.raw_blob_path,
            persisted.raw_payload_json,
            PARSER_VERSION,
            capture_id,
        ],
    )?;
    Ok(())
}

fn update_capture_status(
    connection: &Transaction<'_>,
    capture_id: i64,
    status: &str,
) -> Result<()> {
    connection.execute(
        "UPDATE captures SET status = ?1, error_text = NULL WHERE id = ?2",
        params![status, capture_id],
    )?;
    Ok(())
}

fn update_capture_failure(
    connection: &Connection,
    capture_id: i64,
    error_text: &str,
) -> Result<()> {
    connection.execute(
        "UPDATE captures SET status = 'failed_parse', error_text = ?1 WHERE id = ?2",
        params![error_text, capture_id],
    )?;
    Ok(())
}

fn persist_capture_content(
    blobs_dir: &std::path::Path,
    capture: &DiscoveredCapture,
    snapshot: &crate::connectors::CaptureSnapshot,
) -> Result<PersistedCaptureContent> {
    let byte_size = snapshot
        .source_size_bytes
        .unwrap_or_else(|| u64::try_from(snapshot.raw_text.len()).unwrap_or(u64::MAX));
    let media_type = capture_media_type(capture.source_kind);
    let content_ref = if byte_size <= INLINE_CAPTURE_MAX_BYTES as u64 {
        json!({
            "kind": "inline",
            "mediaType": media_type,
            "text": snapshot.raw_text,
            "sha256": snapshot.raw_sha256,
            "byteSize": byte_size
        })
    } else {
        let extension = capture_extension(media_type);
        let blob_path = format!(
            "captures/{}/{}{}",
            &snapshot.raw_sha256[..2],
            snapshot.raw_sha256,
            extension
        );
        let absolute_path = blobs_dir.join(&blob_path);
        if let Some(parent) = absolute_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        if !absolute_path.exists() {
            fs::write(&absolute_path, snapshot.raw_text.as_bytes())
                .with_context(|| format!("failed to write {}", absolute_path.display()))?;
        }
        json!({
            "kind": "blob",
            "mediaType": media_type,
            "blobPath": blob_path,
            "sha256": snapshot.raw_sha256,
            "byteSize": byte_size
        })
    };

    Ok(PersistedCaptureContent {
        raw_blob_path: content_ref
            .get("blobPath")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        raw_payload_json: serde_json::to_string(&json!({
            "sourceKind": capture.source_kind.as_str(),
            "metadata": capture.metadata,
            "contentRef": content_ref
        }))?,
    })
}

fn capture_media_type(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::OpenCode => "application/json; charset=utf-8",
        _ => "application/x-ndjson; charset=utf-8",
    }
}

fn capture_extension(media_type: &str) -> &'static str {
    if media_type.starts_with("application/json") {
        ".json"
    } else if media_type.starts_with("application/x-ndjson") {
        ".jsonl"
    } else {
        ".txt"
    }
}

fn replace_session_projection(
    connection: &mut Connection,
    source_id: i64,
    capture_id: i64,
    source: &DiscoveredSource,
    capture: &DiscoveredCapture,
    parsed: &ParsedCapture,
) -> Result<()> {
    let tx = connection.transaction()?;
    let capture_record_ids = insert_capture_records(&tx, capture_id, &parsed.raw_records)?;
    let session_id = upsert_session(&tx, source_id, &parsed.session, parsed.messages.len())?;
    let message_links =
        replace_session_messages(&tx, session_id, &parsed.messages, &capture_record_ids)?;
    replace_session_artifacts(
        &tx,
        session_id,
        &parsed.artifacts,
        &capture_record_ids,
        &message_links,
    )?;
    update_capture_status(&tx, capture_id, "normalized")?;
    insert_activity_event_tx(
        &tx,
        "projection_replaced",
        "session",
        Some(session_id),
        Some(session_id),
        json!({
            "captureId": capture_id,
            "sourceKind": source.kind.as_str(),
            "sourcePath": capture.source_path,
            "externalSessionId": capture.external_session_id,
            "messageCount": parsed.messages.len(),
            "artifactCount": parsed.artifacts.len()
        }),
    )?;
    tx.commit()?;
    Ok(())
}

fn insert_capture_records(
    tx: &Transaction<'_>,
    capture_id: i64,
    records: &[ParsedCaptureRecord],
) -> Result<HashMap<usize, i64>> {
    tx.execute(
        "DELETE FROM capture_records WHERE capture_id = ?1",
        [capture_id],
    )?;
    let mut line_to_record = HashMap::new();
    for record in records {
        tx.execute(
            r#"
            INSERT INTO capture_records (
              capture_id,
              line_no,
              record_type,
              record_timestamp,
              provider_message_id,
              parent_provider_message_id,
              role,
              is_meta,
              content_text,
              content_json,
              metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                capture_id,
                record.line_no as i64,
                record.record_type,
                record.record_timestamp,
                record.provider_message_id,
                record.parent_provider_message_id,
                record.role,
                i64::from(record.is_meta),
                record.content_text,
                serde_json::to_string(&record.content_json)?,
                serde_json::to_string(&record.metadata)?,
            ],
        )?;
        line_to_record.insert(record.line_no, tx.last_insert_rowid());
    }
    Ok(line_to_record)
}

fn upsert_session(
    tx: &Transaction<'_>,
    source_id: i64,
    session: &NormalizedSession,
    message_count: usize,
) -> Result<i64> {
    tx.execute(
        r#"
        INSERT INTO sessions (
          source_id,
          external_session_id,
          title,
          project_path,
          source_url,
          model,
          model_provider,
          cli_version,
          git_branch,
          started_at,
          updated_at,
          message_count,
          raw_capture_count,
          summary,
          metadata_json,
          updated_recorded_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1, ?13, ?14, CURRENT_TIMESTAMP)
        ON CONFLICT(source_id, external_session_id) DO UPDATE SET
          title = COALESCE(excluded.title, sessions.title),
          project_path = COALESCE(excluded.project_path, sessions.project_path),
          source_url = COALESCE(excluded.source_url, sessions.source_url),
          model = COALESCE(excluded.model, sessions.model),
          model_provider = COALESCE(excluded.model_provider, sessions.model_provider),
          cli_version = COALESCE(excluded.cli_version, sessions.cli_version),
          git_branch = COALESCE(excluded.git_branch, sessions.git_branch),
          started_at = COALESCE(sessions.started_at, excluded.started_at),
          updated_at = COALESCE(excluded.updated_at, sessions.updated_at),
          message_count = excluded.message_count,
          raw_capture_count = sessions.raw_capture_count + 1,
          summary = COALESCE(excluded.summary, sessions.summary),
          metadata_json = excluded.metadata_json,
          updated_recorded_at = CURRENT_TIMESTAMP
        "#,
        params![
            source_id,
            session.external_session_id,
            session.title,
            session.project_path,
            session.source_url,
            session.model,
            session.model_provider,
            session.cli_version,
            session.git_branch,
            session.started_at,
            session.updated_at,
            message_count as i64,
            session.summary,
            serde_json::to_string(&session.metadata)?,
        ],
    )?;
    let session_id = tx.query_row(
        "SELECT id FROM sessions WHERE source_id = ?1 AND external_session_id = ?2",
        params![source_id, session.external_session_id],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(session_id)
}

fn replace_session_messages(
    tx: &Transaction<'_>,
    session_id: i64,
    messages: &[NormalizedMessage],
    capture_record_ids: &HashMap<usize, i64>,
) -> Result<MessageLinks> {
    tx.execute("DELETE FROM messages WHERE session_id = ?1", [session_id])?;
    let mut links = MessageLinks::default();
    for (index, message) in messages.iter().enumerate() {
        tx.execute(
            r#"
            INSERT INTO messages (
              session_id,
              capture_record_id,
              external_message_id,
              parent_external_message_id,
              ordinal,
              role,
              text,
              text_hash,
              created_at,
              message_kind,
              metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                session_id,
                capture_record_ids.get(&message.source_line_no),
                message.external_message_id,
                message.parent_external_message_id,
                (index + 1) as i64,
                message.role,
                message.text,
                text_sha1(&message.text),
                message.created_at,
                message.message_kind,
                serde_json::to_string(&message.metadata)?,
            ],
        )?;
        let message_id = tx.last_insert_rowid();
        links
            .message_ids_by_source_line
            .insert(message.source_line_no, message_id);
        if let Some(external_message_id) = message.external_message_id.as_ref() {
            links
                .message_ids_by_external_message_id
                .insert(external_message_id.clone(), message_id);
        }
    }
    Ok(links)
}

fn replace_session_artifacts(
    tx: &Transaction<'_>,
    session_id: i64,
    artifacts: &[NormalizedArtifact],
    capture_record_ids: &HashMap<usize, i64>,
    message_links: &MessageLinks,
) -> Result<()> {
    tx.execute("DELETE FROM artifacts WHERE session_id = ?1", [session_id])?;
    for artifact in artifacts {
        tx.execute(
            r#"
            INSERT INTO artifacts (
              session_id,
              message_id,
              capture_record_id,
              kind,
              mime_type,
              metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                session_id,
                resolve_artifact_message_id(artifact, message_links),
                capture_record_ids.get(&artifact.source_line_no),
                artifact.kind,
                artifact.mime_type,
                serde_json::to_string(&artifact.payload)?,
            ],
        )?;
    }
    Ok(())
}

fn resolve_artifact_message_id(
    artifact: &NormalizedArtifact,
    message_links: &MessageLinks,
) -> Option<i64> {
    artifact
        .external_message_id
        .as_ref()
        .and_then(|external_id| {
            message_links
                .message_ids_by_external_message_id
                .get(external_id)
        })
        .copied()
        .or_else(|| {
            message_links
                .message_ids_by_source_line
                .get(&artifact.source_line_no)
                .copied()
        })
}

#[derive(Clone, Debug, Default)]
struct MessageLinks {
    message_ids_by_source_line: HashMap<usize, i64>,
    message_ids_by_external_message_id: HashMap<String, i64>,
}

fn insert_activity_event(
    connection: &Connection,
    event_type: &str,
    object_type: &str,
    object_id: Option<i64>,
    session_id: Option<i64>,
    payload: Value,
) -> Result<()> {
    connection.execute(
        r#"
        INSERT INTO activity_events (
          event_type,
          object_type,
          object_id,
          session_id,
          payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            event_type,
            object_type,
            object_id,
            session_id,
            serde_json::to_string(&payload)?,
        ],
    )?;
    Ok(())
}

fn insert_activity_event_tx(
    tx: &Transaction<'_>,
    event_type: &str,
    object_type: &str,
    object_id: Option<i64>,
    session_id: Option<i64>,
    payload: Value,
) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO activity_events (
          event_type,
          object_type,
          object_id,
          session_id,
          payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            event_type,
            object_type,
            object_id,
            session_id,
            serde_json::to_string(&payload)?,
        ],
    )?;
    Ok(())
}

fn text_sha1(text: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn classify_sync_status(accumulator: &SyncAccumulator) -> &'static str {
    if !accumulator.failed_entries.is_empty()
        && accumulator.imported_captures == 0
        && accumulator.skipped_captures == 0
    {
        "failed"
    } else if !accumulator.failed_entries.is_empty() {
        "warning"
    } else {
        "completed"
    }
}

fn summarize_sync(accumulator: &SyncAccumulator) -> String {
    if accumulator.discovered_captures == 0 && accumulator.failed_entries.is_empty() {
        return format!(
            "{} sources scanned, no captures found",
            accumulator.source_summaries.len()
        );
    }

    format!(
        "{} sources · {} imported · {} skipped · {} failed",
        accumulator.source_summaries.len(),
        accumulator.imported_captures,
        accumulator.skipped_captures,
        accumulator.failed_entries.len()
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::{INLINE_CAPTURE_MAX_BYTES, RustStore};
    use crate::config::AppPaths;

    fn fixture_root(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../distill-electron/src/test/fixtures/ingest")
            .join(name)
            .join("files")
    }

    fn copy_tree(source: &Path, destination: &Path) {
        if !source.exists() {
            return;
        }
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            if source_path.is_dir() {
                copy_tree(&source_path, &destination_path);
            } else {
                if let Some(parent) = destination_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::copy(source_path, destination_path).unwrap();
            }
        }
    }

    fn temp_paths() -> AppPaths {
        let temp = tempdir().unwrap();
        let app_home = temp.keep();
        AppPaths {
            db_path: app_home.join("distill.db"),
            blobs_dir: app_home.join("blobs"),
            prefs_path: app_home.join("preferences.json"),
            app_home,
        }
    }

    fn create_store() -> RustStore {
        RustStore::initialize(temp_paths()).unwrap()
    }

    #[test]
    fn codex_import_populates_sessions_and_jobs() {
        let fixtures = tempdir().unwrap();
        copy_tree(&fixture_root("codex-live-session"), fixtures.path());
        copy_tree(&fixture_root("codex-archived-duplicate"), fixtures.path());

        let store = create_store();
        let report = store
            .sync_codex_from_home(fixtures.path().join(".codex"), "manual_reload")
            .unwrap();

        assert_eq!(report.discovered_captures, 1);
        assert_eq!(report.imported_captures, 1);
        assert_eq!(report.skipped_captures, 0);
        assert_eq!(report.failed_captures, 0);

        let connection = Connection::open(store.database_path()).unwrap();
        let session_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        let capture_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM captures", [], |row| row.get(0))
            .unwrap();
        let job_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE job_type = 'sync_sources'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(session_count, 1);
        assert_eq!(capture_count, 1);
        assert_eq!(job_count, 1);
    }

    #[test]
    fn multi_connector_sync_imports_claude_and_links_artifacts() {
        let fixtures = tempdir().unwrap();
        copy_tree(&fixture_root("codex-live-session"), fixtures.path());
        copy_tree(&fixture_root("codex-archived-duplicate"), fixtures.path());
        copy_tree(&fixture_root("claude-mixed-blocks"), fixtures.path());

        let store = create_store();
        let report = store
            .sync_codex_and_claude_from_homes(
                fixtures.path().join(".codex"),
                fixtures.path().join(".claude"),
                "multi_connector_sync",
            )
            .unwrap();

        assert_eq!(report.discovered_captures, 2);
        assert_eq!(report.imported_captures, 2);
        assert_eq!(report.skipped_captures, 0);
        assert_eq!(report.failed_captures, 0);
        assert_eq!(report.source_summaries.len(), 2);
        assert_eq!(
            report
                .source_summaries
                .iter()
                .map(|summary| summary.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["codex", "claude_code"]
        );

        let connection = Connection::open(store.database_path()).unwrap();
        let session_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        let artifact_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM artifacts", [], |row| row.get(0))
            .unwrap();
        let claude_title: String = connection
            .query_row(
                r#"
                SELECT s.title
                FROM sessions s
                JOIN sources so ON so.id = s.source_id
                WHERE so.kind = 'claude_code'
                LIMIT 1
                "#,
                [],
                |row| row.get(0),
            )
            .unwrap();
        let linked_artifacts: i64 = connection
            .query_row(
                r#"
                SELECT COUNT(*)
                FROM artifacts a
                JOIN sessions s ON s.id = a.session_id
                JOIN sources so ON so.id = s.source_id
                WHERE so.kind = 'claude_code'
                  AND a.message_id IS NOT NULL
                  AND a.capture_record_id IS NOT NULL
                "#,
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(session_count, 2);
        assert_eq!(artifact_count, 3);
        assert_eq!(claude_title, "Claude mixed content fixture");
        assert_eq!(linked_artifacts, 3);
    }

    #[test]
    fn duplicate_reimport_is_skipped() {
        let fixtures = tempdir().unwrap();
        copy_tree(&fixture_root("codex-live-session"), fixtures.path());
        copy_tree(&fixture_root("codex-archived-duplicate"), fixtures.path());

        let store = create_store();
        store
            .sync_codex_from_home(fixtures.path().join(".codex"), "first_sync")
            .unwrap();
        let report = store
            .sync_codex_from_home(fixtures.path().join(".codex"), "second_sync")
            .unwrap();

        assert_eq!(report.imported_captures, 0);
        assert_eq!(report.skipped_captures, 1);

        let connection = Connection::open(store.database_path()).unwrap();
        let capture_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM captures", [], |row| row.get(0))
            .unwrap();
        assert_eq!(capture_count, 1);
    }

    #[test]
    fn parse_failure_preserves_prior_projection() {
        let fixtures = tempdir().unwrap();
        copy_tree(&fixture_root("codex-live-session"), fixtures.path());

        let store = create_store();
        store
            .sync_codex_from_home(fixtures.path().join(".codex"), "good_sync")
            .unwrap();

        fs::remove_dir_all(fixtures.path().join(".codex/sessions")).unwrap();
        copy_tree(
            &fixture_root("parse-failure-after-snapshot"),
            fixtures.path(),
        );
        let report = store
            .sync_codex_from_home(fixtures.path().join(".codex"), "parse_failure_sync")
            .unwrap();

        assert_eq!(report.failed_captures, 1);

        let connection = Connection::open(store.database_path()).unwrap();
        let statuses = connection
            .prepare("SELECT status FROM captures ORDER BY id ASC")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let assistant_text: String = connection
            .query_row(
                "SELECT text FROM messages WHERE role = 'assistant' ORDER BY ordinal ASC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(
            statuses,
            vec!["normalized".to_string(), "failed_parse".to_string()]
        );
        assert!(assistant_text.contains("I will update the code."));
    }

    #[test]
    fn large_capture_uses_blob_persistence() {
        let fixtures = tempdir().unwrap();
        let codex_home = fixtures.path().join(".codex");
        let archived = codex_home.join("archived_sessions");
        fs::create_dir_all(&archived).unwrap();
        let capture_path = archived.join("rollout-2026-03-26T09-00-00-blob-large-session.jsonl");
        let large_text = "A".repeat(INLINE_CAPTURE_MAX_BYTES + 256);
        fs::write(
            &capture_path,
            [
                r#"{"timestamp":"2026-03-26T09:00:00.000Z","type":"session_meta","payload":{"id":"blob-large-session","cwd":"/tmp/large"}}"#,
                &format!(
                    "{{\"timestamp\":\"2026-03-26T09:00:01.000Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"{}\"}}]}}}}",
                    large_text
                ),
                r#"{"timestamp":"2026-03-26T09:00:02.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Blob-backed capture imported."}]}}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        let store = create_store();
        store.sync_codex_from_home(codex_home, "blob_sync").unwrap();

        let connection = Connection::open(store.database_path()).unwrap();
        let (raw_blob_path, raw_payload_json): (Option<String>, String) = connection
            .query_row(
                "SELECT raw_blob_path, raw_payload_json FROM captures LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let raw_blob_path = raw_blob_path.expect("blob path should be recorded");
        let blob_file = store.app_home().join("blobs").join(&raw_blob_path);
        assert!(blob_file.exists());
        assert!(raw_payload_json.contains("\"kind\":\"blob\""));
    }
}
