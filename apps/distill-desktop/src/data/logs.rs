use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::view_models::{LogEntryVm, LogFilter, LogsPageVm};

use super::{DesktopDataSource, matches_query, truncate_inline};

#[derive(Clone, Debug)]
struct LogRow {
    id: String,
    kind: String,
    title: String,
    subtitle: String,
    summary: String,
    status: String,
    level: String,
    metrics: String,
    raw_json: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct JobPayload {
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default, rename = "startedAt")]
    started_at: Option<String>,
    #[serde(default, rename = "finishedAt")]
    finished_at: Option<String>,
    #[serde(default, rename = "discoveredCaptures")]
    discovered_captures: Option<i64>,
    #[serde(default, rename = "importedCaptures")]
    imported_captures: Option<i64>,
    #[serde(default, rename = "skippedCaptures")]
    skipped_captures: Option<i64>,
    #[serde(default, rename = "failedCaptures")]
    failed_captures: Option<i64>,
    #[serde(default)]
    outcome: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct ExportPayload {
    #[serde(default, rename = "exportedAt")]
    exported_at: Option<String>,
    #[serde(default)]
    dataset: Option<String>,
}

impl DesktopDataSource {
    pub fn load_logs(
        &self,
        filter: LogFilter,
        query: &str,
        expanded_log_id: Option<&str>,
    ) -> Result<LogsPageVm> {
        if !self.database_exists() {
            return Ok(LogsPageVm {
                entries: Vec::new(),
                summary_total_text: "0 entries".to_string(),
                summary_error_text: "0 errors".to_string(),
                summary_sync_text: "idle".to_string(),
                empty_title: "No logs yet".to_string(),
                empty_message: match self.source_mode() {
                    crate::config::SourceMode::RustOwned => {
                        "Sync jobs and exports from the Rust-owned Distill store will appear here when present."
                            .to_string()
                    }
                    crate::config::SourceMode::ElectronCompatReadOnly => {
                        "Open a compatible Distill Electron database to inspect sync and export history."
                            .to_string()
                    }
                },
            });
        }

        let conn = self.open_read_only()?;
        let mut rows = self.load_log_rows(&conn)?;
        rows.sort_by(|left, right| right.subtitle.cmp(&left.subtitle));
        let total_count = rows.len();
        let latest_sync_label = rows
            .iter()
            .find(|row| row.kind == "sync")
            .map(|row| normalize_summary_sync_label(row))
            .unwrap_or_else(|| "idle".to_string());
        let error_count = rows.iter().filter(|row| row.level == "error").count();

        rows.retain(|row| {
            matches_log_filter(row, filter)
                && matches_query(
                    query,
                    &[&row.title, &row.summary, &row.metrics, &row.raw_json],
                )
        });

        let expanded_id = expanded_log_id
            .filter(|candidate| rows.iter().any(|row| row.id == *candidate))
            .map(ToOwned::to_owned);

        let entries = rows
            .into_iter()
            .map(|row| LogEntryVm {
                expanded: expanded_id.as_deref() == Some(row.id.as_str()),
                id: row.id,
                title: row.title,
                subtitle: row.subtitle,
                summary: row.summary,
                status: row.status,
                level: row.level,
                metrics: row.metrics,
                raw_json: row.raw_json,
            })
            .collect::<Vec<_>>();

        let (empty_title, empty_message) = if entries.is_empty() {
            if query.trim().is_empty() && matches!(filter, LogFilter::All) {
                (
                    "No logs yet".to_string(),
                    match self.source_mode() {
                        crate::config::SourceMode::RustOwned => {
                            "Sync and export activity will show up here once the Rust-owned Distill store has operational history."
                                .to_string()
                        }
                        crate::config::SourceMode::ElectronCompatReadOnly => {
                            "Sync and export activity will show up here once Distill Electron has operational history."
                                .to_string()
                        }
                    },
                )
            } else {
                (
                    "No matching logs".to_string(),
                    "Adjust the log search or filters to widen the current result set."
                        .to_string(),
                )
            }
        } else {
            (String::new(), String::new())
        };

        Ok(LogsPageVm {
            entries,
            summary_total_text: format!("{total_count} entries"),
            summary_error_text: format!("{error_count} errors"),
            summary_sync_text: latest_sync_label,
            empty_title,
            empty_message,
        })
    }

    fn load_log_rows(&self, conn: &Connection) -> Result<Vec<LogRow>> {
        let mut rows = Vec::new();

        let mut job_stmt = conn.prepare(
            r#"
            SELECT id, status, last_error, payload_json, created_at, updated_at
            FROM jobs
            WHERE job_type = 'sync_sources'
            ORDER BY COALESCE(updated_at, created_at) DESC
            "#,
        )?;
        let mut job_rows = job_stmt.query([])?;
        while let Some(row) = job_rows.next()? {
            let id: i64 = row.get(0)?;
            let status: String = row.get(1)?;
            let last_error: Option<String> = row.get(2)?;
            let payload_json: String = row.get(3)?;
            let created_at: String = row.get(4)?;
            let updated_at: Option<String> = row.get(5)?;
            let payload = serde_json::from_str::<JobPayload>(&payload_json).unwrap_or_default();
            let normalized = normalize_sync_status(&status, &payload);
            let metrics = format!(
                "{} found · {} imported · {} skipped · {} failed",
                payload.discovered_captures.unwrap_or(0),
                payload.imported_captures.unwrap_or(0),
                payload.skipped_captures.unwrap_or(0),
                payload.failed_captures.unwrap_or(0)
            );
            rows.push(LogRow {
                id: format!("sync-{id}"),
                kind: "sync".to_string(),
                title: "Background sync".to_string(),
                subtitle: updated_at.unwrap_or(created_at),
                summary: payload.summary.clone().unwrap_or_else(|| {
                    payload
                        .reason
                        .clone()
                        .unwrap_or_else(|| "Sync activity".to_string())
                }),
                status: normalized.clone(),
                level: if normalized == "failed" {
                    "error".to_string()
                } else {
                    "info".to_string()
                },
                metrics,
                raw_json: serde_json::to_string_pretty(&serde_json::json!({
                    "jobId": id,
                    "status": status,
                    "lastError": last_error,
                    "payload": payload,
                }))
                .unwrap_or_else(|_| payload_json.clone()),
            });
        }

        let mut export_stmt = conn.prepare(
            r#"
            SELECT id, export_type, label_filter, output_path, record_count, metadata_json, created_at
            FROM exports
            ORDER BY created_at DESC
            "#,
        )?;
        let mut export_rows = export_stmt.query([])?;
        while let Some(row) = export_rows.next()? {
            let id: i64 = row.get(0)?;
            let export_type: String = row.get(1)?;
            let label_filter: Option<String> = row.get(2)?;
            let output_path: String = row.get(3)?;
            let record_count: i64 = row.get(4)?;
            let metadata_json: String = row.get(5)?;
            let created_at: String = row.get(6)?;
            let payload = serde_json::from_str::<ExportPayload>(&metadata_json).unwrap_or_default();
            let dataset = payload
                .dataset
                .clone()
                .or(label_filter.clone())
                .unwrap_or_else(|| "all".to_string());
            rows.push(LogRow {
                id: format!("export-{id}"),
                kind: "export".to_string(),
                title: "Export".to_string(),
                subtitle: created_at,
                summary: format!("Exported {record_count} {dataset} records"),
                status: "completed".to_string(),
                level: "info".to_string(),
                metrics: format!(
                    "type={export_type} · output={}",
                    truncate_inline(&output_path, 48)
                ),
                raw_json: serde_json::to_string_pretty(&serde_json::json!({
                    "exportId": id,
                    "exportType": export_type,
                    "dataset": dataset,
                    "outputPath": output_path,
                    "recordCount": record_count,
                    "payload": payload,
                }))
                .unwrap_or_else(|_| metadata_json.clone()),
            });
        }

        Ok(rows)
    }
}

fn normalize_sync_status(status: &str, payload: &JobPayload) -> String {
    match status {
        "pending" => "queued".to_string(),
        "running" | "warning" | "failed" => status.to_string(),
        "completed" => {
            if let Some(outcome) = payload.outcome.as_deref() {
                outcome.to_string()
            } else if payload.failed_captures.unwrap_or(0) > 0 {
                "warning".to_string()
            } else {
                "completed".to_string()
            }
        }
        other => other.to_string(),
    }
}

fn normalize_summary_sync_label(row: &LogRow) -> String {
    match row.status.as_str() {
        "warning" => "sync warnings".to_string(),
        "failed" => "sync failed".to_string(),
        "running" => "syncing...".to_string(),
        "queued" => "sync queued".to_string(),
        "completed" => format!("synced {}", row.subtitle),
        _ => row.status.clone(),
    }
}

fn matches_log_filter(row: &LogRow, filter: LogFilter) -> bool {
    match filter {
        LogFilter::All => true,
        LogFilter::Sync => row.kind == "sync",
        LogFilter::Export => row.kind == "export",
        LogFilter::Errors => row.level == "error",
    }
}
