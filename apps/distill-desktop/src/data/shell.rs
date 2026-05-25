use anyhow::Result;
use rusqlite::Connection;
use serde::Deserialize;

use crate::config::SourceMode;
use crate::connectors::{SourceKind, configured_rust_connectors};
use crate::view_models::{
    AppSnapshotVm, ShellStatVm, SourceCheckVm, SourceRowVm, SyncStatusVm,
};

use super::DesktopDataSource;

#[derive(Clone, Debug, Default, Deserialize)]
struct JobPayload {
    #[serde(default, rename = "summary")]
    summary: Option<String>,
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

impl DesktopDataSource {
    pub fn app_snapshot(&self) -> Result<AppSnapshotVm> {
        let database_path = self.database_path();
        let source_rows = self.detect_source_rows();
        let installed_source_count = source_rows
            .iter()
            .filter(|row| row.status_tone == "ok")
            .count();

        let mut snapshot = AppSnapshotVm {
            home_path: self.home_path().to_path_buf(),
            database_path: database_path.clone(),
            database_exists: self.database_exists(),
            source_mode_label: self.source_mode().label().to_string(),
            source_badge_text: match self.source_mode() {
                SourceMode::RustOwned => "Rust-Owned".to_string(),
                SourceMode::ElectronCompatReadOnly => "Electron Compat".to_string(),
            },
            app_status_text: match self.source_mode() {
                SourceMode::RustOwned => "Rust-owned store ready".to_string(),
                SourceMode::ElectronCompatReadOnly => "Read-only compatibility mode".to_string(),
            },
            source_rows,
            sync_status: SyncStatusVm {
                text: match self.source_mode() {
                    SourceMode::RustOwned => "idle".to_string(),
                    SourceMode::ElectronCompatReadOnly => "read only".to_string(),
                },
                tone: match self.source_mode() {
                    SourceMode::RustOwned => "idle".to_string(),
                    SourceMode::ElectronCompatReadOnly => "warning".to_string(),
                },
                enabled: matches!(self.source_mode(), SourceMode::RustOwned),
                button_label: "Sync".to_string(),
            },
            onboarding_title: "No sessions found".to_string(),
            onboarding_message: match self.source_mode() {
                SourceMode::RustOwned => {
                    "Distill Desktop reads conversations from Codex CLI and Claude Code. Sync local histories into this Rust-owned store to populate Sessions."
                        .to_string()
                }
                SourceMode::ElectronCompatReadOnly => {
                    "Distill Desktop can inspect an existing Distill Electron database backed by Codex CLI and Claude Code imports."
                        .to_string()
                }
            },
            ..AppSnapshotVm::default()
        };

        if !snapshot.database_exists {
            snapshot.app_status_text = match self.source_mode() {
                SourceMode::RustOwned => {
                    format!("Rust store missing at {}", database_path.display())
                }
                SourceMode::ElectronCompatReadOnly => {
                    format!(
                        "Waiting for Distill Electron data at {}",
                        database_path.display()
                    )
                }
            };
            snapshot.show_onboarding = true;
            snapshot.sidebar_count_label = "Sessions".to_string();
            snapshot.scanned_at_label = "not synced".to_string();
            snapshot.shell_stats = vec![
                ShellStatVm {
                    label: "sessions".to_string(),
                    value: "0".to_string(),
                },
                ShellStatVm {
                    label: "messages".to_string(),
                    value: "0".to_string(),
                },
                ShellStatVm {
                    label: "sources".to_string(),
                    value: installed_source_count.to_string(),
                },
            ];
            snapshot.settings = self.build_settings_snapshot(&snapshot.source_rows);
            return Ok(snapshot);
        }

        let conn = self.open_read_only()?;
        snapshot.session_count = self.scalar_count(&conn, "SELECT COUNT(*) FROM sessions")?;
        snapshot.message_count = self.scalar_count(&conn, "SELECT COUNT(*) FROM messages")?;
        snapshot.log_count = self.scalar_count(
            &conn,
            "SELECT COUNT(*) FROM jobs WHERE job_type = 'sync_sources'",
        )? + self.scalar_count(&conn, "SELECT COUNT(*) FROM exports")?;
        snapshot.table_count = self.list_tables(&conn)?.len();
        snapshot.sync_status = self.load_sync_status(&conn)?;
        snapshot.app_status_text = format!(
            "{} sessions, {} messages, {} logs",
            snapshot.session_count, snapshot.message_count, snapshot.log_count
        );
        snapshot.sidebar_count_label = format!("{} sessions", snapshot.session_count);
        snapshot.scanned_at_label = snapshot.sync_status.text.clone();
        snapshot.show_onboarding = snapshot.session_count == 0;
        snapshot.shell_stats = vec![
            ShellStatVm {
                label: "sessions".to_string(),
                value: snapshot.session_count.to_string(),
            },
            ShellStatVm {
                label: "messages".to_string(),
                value: snapshot.message_count.to_string(),
            },
            ShellStatVm {
                label: "sources".to_string(),
                value: installed_source_count.to_string(),
            },
        ];
        snapshot.settings = self.build_settings_snapshot(&snapshot.source_rows);

        Ok(snapshot)
    }

    fn load_sync_status(&self, conn: &Connection) -> Result<SyncStatusVm> {
        if !matches!(self.source_mode(), SourceMode::RustOwned) {
            return Ok(SyncStatusVm {
                text: "read only".to_string(),
                tone: "warning".to_string(),
                enabled: false,
                button_label: "Sync".to_string(),
            });
        }

        let latest = conn.query_row(
            r#"
            SELECT status, payload_json, COALESCE(updated_at, created_at)
            FROM jobs
            WHERE job_type = 'sync_sources'
            ORDER BY COALESCE(updated_at, created_at) DESC
            LIMIT 1
            "#,
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        );

        let (status, payload_json, updated_at) = match latest {
            Ok(value) => value,
            Err(_) => {
                return Ok(SyncStatusVm {
                    text: "idle".to_string(),
                    tone: "idle".to_string(),
                    enabled: true,
                    button_label: "Sync".to_string(),
                });
            }
        };

        let payload = serde_json::from_str::<JobPayload>(&payload_json).unwrap_or_default();
        let normalized = match status.as_str() {
            "pending" => "queued",
            "running" => "running",
            "warning" => "warning",
            "failed" => "failed",
            "completed" => {
                if payload.outcome.as_deref() == Some("warning")
                    || payload.failed_captures.unwrap_or(0) > 0
                {
                    "warning"
                } else {
                    "completed"
                }
            }
            _ => "idle",
        };

        let summary = payload.summary.unwrap_or_else(|| {
            format!(
                "{} · {} imported · {} skipped · {} failed",
                payload.discovered_captures.unwrap_or(0),
                payload.imported_captures.unwrap_or(0),
                payload.skipped_captures.unwrap_or(0),
                payload.failed_captures.unwrap_or(0)
            )
        });

        Ok(SyncStatusVm {
            text: if normalized == "completed" {
                format!("synced {}", truncate_sync_label(payload.finished_at.as_deref().unwrap_or(&updated_at)))
            } else if normalized == "warning" {
                format!("sync warnings {}", truncate_sync_label(payload.finished_at.as_deref().unwrap_or(&updated_at)))
            } else if normalized == "failed" {
                "sync failed".to_string()
            } else if normalized == "running" {
                "syncing...".to_string()
            } else if normalized == "queued" {
                "sync queued".to_string()
            } else {
                summary
            },
            tone: normalized.to_string(),
            enabled: true,
            button_label: "Sync".to_string(),
        })
    }

    fn detect_source_rows(&self) -> Vec<SourceRowVm> {
        let mut rows = Vec::new();

        match configured_rust_connectors() {
            Ok(connectors) => {
                for connector in connectors {
                    let kind = connector.kind();
                    match connector.detect() {
                        Ok(source) => rows.push(SourceRowVm {
                            kind: kind.as_str().to_string(),
                            display_name: source.display_name,
                            status_text: source.install_status.as_str().replace('_', " "),
                            status_tone: match source.install_status.as_str() {
                                "installed" => "ok".to_string(),
                                "partial" => "warning".to_string(),
                                _ => "error".to_string(),
                            },
                            data_root: source
                                .data_root
                                .map(|path| path.display().to_string())
                                .unwrap_or_else(|| "not found".to_string()),
                            checks: source
                                .checks
                                .into_iter()
                                .map(|check| SourceCheckVm {
                                    label: check.label,
                                    state_text: if check.exists {
                                        match check.file_count {
                                            Some(count) => format!("found {count} files"),
                                            None => "found".to_string(),
                                        }
                                    } else {
                                        "missing".to_string()
                                    },
                                    exists: check.exists,
                                })
                                .collect(),
                            is_stub: false,
                        }),
                        Err(error) => rows.push(SourceRowVm {
                            kind: kind.as_str().to_string(),
                            display_name: kind.display_name().to_string(),
                            status_text: format!("detect failed: {error}"),
                            status_tone: "warning".to_string(),
                            data_root: "not found".to_string(),
                            checks: Vec::new(),
                            is_stub: false,
                        }),
                    }
                }
            }
            Err(error) => rows.push(SourceRowVm {
                kind: "connectors".to_string(),
                display_name: "Connector Registry".to_string(),
                status_text: format!("detect failed: {error}"),
                status_tone: "warning".to_string(),
                data_root: "not available".to_string(),
                checks: Vec::new(),
                is_stub: false,
            }),
        }

        if !rows.iter().any(|row| row.kind == SourceKind::OpenCode.as_str()) {
            rows.push(SourceRowVm {
                kind: SourceKind::OpenCode.as_str().to_string(),
                display_name: SourceKind::OpenCode.display_name().to_string(),
                status_text: "not wired yet".to_string(),
                status_tone: "warning".to_string(),
                data_root: "not available".to_string(),
                checks: vec![SourceCheckVm {
                    label: "connector".to_string(),
                    state_text: "missing".to_string(),
                    exists: false,
                }],
                is_stub: true,
            });
        }

        rows.sort_by(|left, right| left.display_name.cmp(&right.display_name));
        rows
    }
}

fn truncate_sync_label(value: &str) -> String {
    if value.len() <= 19 {
        value.to_string()
    } else {
        value.chars().take(19).collect()
    }
}
