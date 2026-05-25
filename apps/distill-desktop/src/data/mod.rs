mod db;
mod logs;
mod sessions;
mod settings;
mod shell;
mod sql_guard;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::Connection;
use rusqlite::types::ValueRef;
use serde_json::Value;

use crate::compat::ElectronCompatStore;
use crate::config::{DesktopRuntimeConfig, SourceMode};
use crate::storage::{RustStore, SyncReport};
use crate::view_models::KeyValueRowVm;

pub use sql_guard::guard_read_only_sql;

pub(super) const PAGE_SIZE: usize = 50;
pub(super) const FILTER_OPERATORS: [&str; 13] = [
    "contains",
    "equals",
    "not_equals",
    "starts_with",
    "ends_with",
    "eq",
    "neq",
    "gt",
    "gte",
    "lt",
    "lte",
    "is_null",
    "is_not_null",
];

#[derive(Clone, Debug)]
pub enum DesktopBackend {
    RustOwned(RustStore),
    ElectronCompat(ElectronCompatStore),
}

#[derive(Clone, Debug)]
pub struct DesktopDataSource {
    runtime: DesktopRuntimeConfig,
    backend: DesktopBackend,
}

#[derive(Clone, Debug, Default)]
pub struct DbBrowseRequestVm {
    pub table_name: String,
    pub filter_column: String,
    pub filter_operator: String,
    pub filter_value: String,
    pub sort_column: String,
    pub sort_direction: String,
    pub page: usize,
}

impl DesktopDataSource {
    pub fn new(runtime: DesktopRuntimeConfig) -> Result<Self> {
        let backend = match runtime.source_mode {
            SourceMode::RustOwned => {
                DesktopBackend::RustOwned(RustStore::initialize(runtime.app_paths.clone())?)
            }
            SourceMode::ElectronCompatReadOnly => {
                let home = runtime
                    .electron_home
                    .clone()
                    .context("electron compatibility mode requires an Electron home path")?;
                DesktopBackend::ElectronCompat(ElectronCompatStore::new(home))
            }
        };

        Ok(Self { runtime, backend })
    }

    pub fn source_mode(&self) -> SourceMode {
        self.runtime.source_mode
    }

    pub fn home_path(&self) -> &Path {
        match &self.backend {
            DesktopBackend::RustOwned(store) => store.app_home(),
            DesktopBackend::ElectronCompat(store) => store.home_path(),
        }
    }

    pub fn database_path(&self) -> PathBuf {
        match &self.backend {
            DesktopBackend::RustOwned(store) => store.database_path().to_path_buf(),
            DesktopBackend::ElectronCompat(store) => store.database_path(),
        }
    }

    pub fn database_exists(&self) -> bool {
        match &self.backend {
            DesktopBackend::RustOwned(store) => store.database_exists(),
            DesktopBackend::ElectronCompat(store) => store.database_exists(),
        }
    }

    pub fn sync_now(&self, reason: &str) -> Result<SyncReport> {
        match &self.backend {
            DesktopBackend::RustOwned(store) => store.sync_now(reason),
            DesktopBackend::ElectronCompat(_) => {
                bail!("sync is unavailable in Electron compatibility mode")
            }
        }
    }

    pub(super) fn open_read_only(&self) -> Result<Connection> {
        match &self.backend {
            DesktopBackend::RustOwned(store) => store.open_read_only(),
            DesktopBackend::ElectronCompat(store) => store.open_read_only(),
        }
    }

    pub(super) fn scalar_count(&self, conn: &Connection, sql: &str) -> Result<usize> {
        let count = conn.query_row(sql, [], |row| row.get::<_, i64>(0))?;
        Ok(count.max(0) as usize)
    }
}

pub(super) fn key_value(key: &str, value: &str) -> KeyValueRowVm {
    KeyValueRowVm {
        key: key.to_string(),
        value: value.to_string(),
    }
}

pub(super) fn push_if_some(target: &mut Vec<KeyValueRowVm>, key: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        target.push(key_value(key, value));
    }
}

pub(super) fn parse_json_object(raw: Option<&str>) -> serde_json::Map<String, Value> {
    raw.and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

pub(super) fn derive_session_title(title: Option<&str>, first_user_text: Option<&str>) -> String {
    title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| clean_excerpt(first_user_text, 120))
        .unwrap_or_else(|| "Untitled Session".to_string())
}

pub(super) fn derive_session_preview(
    first_assistant_text: Option<&str>,
    first_user_text: Option<&str>,
) -> Option<String> {
    clean_excerpt(first_assistant_text, 220).or_else(|| clean_excerpt(first_user_text, 220))
}

pub(super) fn clean_excerpt(value: Option<&str>, max_len: usize) -> Option<String> {
    let cleaned = value.map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))?;
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return None;
    }

    Some(if cleaned.chars().count() > max_len {
        format!(
            "{}…",
            cleaned
                .chars()
                .take(max_len.saturating_sub(1))
                .collect::<String>()
                .trim_end()
        )
    } else {
        cleaned.to_string()
    })
}

pub(super) fn prettify_source(source_kind: &str) -> &'static str {
    match source_kind {
        "claude_code" => "Claude Code",
        "opencode" => "OpenCode",
        _ => "Codex",
    }
}

pub(super) fn uppercase_role(role: &str) -> String {
    let mut chars = role.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Message".to_string(),
    }
}

pub(super) fn join_or_none<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let joined = values
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if joined.is_empty() {
        "None".to_string()
    } else {
        joined.join(", ")
    }
}

pub(super) fn matches_query(query: &str, haystacks: &[&str]) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return true;
    }

    let query = trimmed.to_lowercase();
    haystacks
        .iter()
        .any(|value| value.to_lowercase().contains(&query))
}

pub(super) fn quote_identifier(identifier: &str) -> Result<String> {
    if identifier.trim().is_empty() {
        bail!("identifier cannot be empty");
    }
    Ok(format!("\"{}\"", identifier.replace('"', "\"\"")))
}

pub(super) fn quote_sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(super) fn escape_like_pattern(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub(super) fn cell_to_text(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".to_string(),
        ValueRef::Integer(value) => value.to_string(),
        ValueRef::Real(value) => value.to_string(),
        ValueRef::Text(value) => String::from_utf8_lossy(value).to_string(),
        ValueRef::Blob(value) => format!("[blob {} bytes]", value.len()),
    }
}

pub(super) fn truncate_inline(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        return value.to_string();
    }

    format!(
        "{}…",
        value
            .chars()
            .take(max_len.saturating_sub(1))
            .collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppPaths, DesktopRuntimeConfig};
    use crate::view_models::{LogFilter, SessionLane};
    use rusqlite::Connection;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn fixture_home() -> tempfile::TempDir {
        let tmp = tempdir().unwrap();
        let db_path = tmp.path().join("distill-electron.db");
        let conn = Connection::open(&db_path).unwrap();
        let schema = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../distill-electron/schema.sql"),
        )
        .unwrap();
        conn.execute_batch(&schema).unwrap();

        conn.execute(
            "INSERT INTO sources (id, kind, display_name, install_status) VALUES (1, 'codex', 'Codex', 'installed')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, source_id, external_session_id, title, project_path, updated_at, message_count, raw_capture_count, model, git_branch, summary, metadata_json) VALUES (1, 1, 'session-1', 'Search Pipeline', '/tmp/project', '2026-04-04T12:00:00Z', 3, 2, 'gpt-5.4', 'main', 'Distill session summary', '{\"origin\":\"fixture\"}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json) VALUES (1, 0, 'user', 'Investigate the search pipeline', 'a', '2026-04-04T10:00:00Z', 'text', '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json) VALUES (1, 1, 'assistant', 'The pipeline is reading from SQLite FTS.', 'b', '2026-04-04T10:01:00Z', 'text', '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json) VALUES (1, 2, 'assistant', 'reasoning trace', 'c', '2026-04-04T10:02:00Z', 'meta', '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO labels (id, name, scope) VALUES (1, 'train', 'session')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO label_assignments (object_type, object_id, label_id, origin) VALUES ('session', 1, 1, 'manual')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tags (id, name, kind) VALUES (1, 'research', 'general')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tag_assignments (object_type, object_id, tag_id, origin) VALUES ('session', 1, 1, 'manual')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO artifacts (session_id, kind, mime_type, metadata_json, created_at) VALUES (1, 'tool_result', 'application/json', '{\"output\":\"done\"}', '2026-04-04T10:03:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO jobs (job_type, object_type, object_id, status, payload_json, created_at, updated_at) VALUES ('sync_sources', 'sync_job', 1, 'completed', '{\"summary\":\"Sync completed\",\"discoveredCaptures\":4,\"importedCaptures\":2,\"skippedCaptures\":1,\"failedCaptures\":0}', '2026-04-04T11:00:00Z', '2026-04-04T11:01:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO exports (export_type, label_filter, output_path, record_count, metadata_json, created_at) VALUES ('jsonl', 'train', '/tmp/export.jsonl', 1, '{\"dataset\":\"train\"}', '2026-04-04T11:30:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_preferences (key, value) VALUES ('theme', 'light')",
            [],
        )
        .unwrap();
        tmp
    }

    fn temp_paths(root: &Path) -> AppPaths {
        let app_home = root.join("distill-desktop");
        AppPaths {
            db_path: app_home.join("distill.db"),
            blobs_dir: app_home.join("blobs"),
            prefs_path: app_home.join("preferences.json"),
            app_home,
        }
    }

    fn electron_source(home: &Path) -> DesktopDataSource {
        DesktopDataSource::new(DesktopRuntimeConfig {
            app_paths: temp_paths(home),
            source_mode: SourceMode::ElectronCompatReadOnly,
            electron_home: Some(home.to_path_buf()),
        })
        .unwrap()
    }

    fn rust_source(root: &Path) -> (DesktopDataSource, AppPaths) {
        let app_paths = temp_paths(root);
        let source = DesktopDataSource::new(DesktopRuntimeConfig {
            app_paths: app_paths.clone(),
            source_mode: SourceMode::RustOwned,
            electron_home: None,
        })
        .unwrap();
        (source, app_paths)
    }

    #[test]
    fn boot_snapshot_reads_fixture_database() {
        let home = fixture_home();
        let snapshot = electron_source(home.path()).app_snapshot().unwrap();
        assert!(snapshot.database_exists);
        assert_eq!(snapshot.session_count, 1);
        assert!(snapshot.table_count > 0);
        assert_eq!(
            snapshot.source_mode_label,
            "Electron Compatibility / Read Only"
        );
    }

    #[test]
    fn sessions_lane_and_search_queries_work() {
        let home = fixture_home();
        let sessions = electron_source(home.path())
            .load_sessions(SessionLane::TrainReady, "pipeline", None)
            .unwrap();
        assert_eq!(sessions.rows.len(), 1);
        assert_eq!(
            sessions.rows[0]
                .workflow_badge
                .as_ref()
                .map(|badge| badge.text.as_str()),
            Some("train")
        );
    }

    #[test]
    fn session_detail_renders_transcript_and_artifacts() {
        let home = fixture_home();
        let detail = electron_source(home.path()).load_session_detail(1).unwrap();
        assert_eq!(detail.title, "Search Pipeline");
        assert_eq!(detail.messages.len(), 3);
        assert_eq!(detail.artifacts.len(), 1);
        assert!(detail.tags.iter().any(|tag| tag == "research"));
    }

    #[test]
    fn logs_filtering_works() {
        let home = fixture_home();
        let logs = electron_source(home.path())
            .load_logs(LogFilter::Sync, "", None)
            .unwrap();
        assert_eq!(logs.entries.len(), 1);
        assert!(logs.entries[0].summary.contains("Sync"));
    }

    #[test]
    fn db_browse_and_query_are_read_only() {
        let home = fixture_home();
        let db = electron_source(home.path());
        let browse = db
            .browse_db_table(DbBrowseRequestVm {
                table_name: "sessions".to_string(),
                page: 1,
                ..DbBrowseRequestVm::default()
            })
            .unwrap();
        assert!(!browse.result_rows.is_empty());

        let query = db
            .run_read_only_query("SELECT id, title FROM sessions")
            .unwrap();
        assert!(query.preview.contains("title=Search Pipeline"));
        assert!(guard_read_only_sql("DELETE FROM sessions").is_err());
    }

    #[test]
    fn read_only_queries_do_not_create_new_files() {
        let home = fixture_home();
        let before = fs::read_dir(home.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        let db = electron_source(home.path());
        db.app_snapshot().unwrap();
        db.load_sessions(SessionLane::All, "", None).unwrap();
        db.run_read_only_query("SELECT * FROM sessions").unwrap();
        let after = fs::read_dir(home.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(before, after);
    }

    #[test]
    fn rust_mode_initializes_store_and_empty_views() {
        let root = tempdir().unwrap();
        let (db, app_paths) = rust_source(root.path());
        let snapshot = db.app_snapshot().unwrap();
        assert_eq!(snapshot.source_mode_label, "Rust-Owned / Writable");
        assert!(snapshot.database_exists);
        assert_eq!(snapshot.session_count, 0);
        assert!(app_paths.db_path.exists());
        assert!(app_paths.blobs_dir.exists());

        let sessions = db.load_sessions(SessionLane::All, "", None).unwrap();
        assert!(sessions.rows.is_empty());
        assert_eq!(sessions.empty_title, "No sessions in All");

        let browse = db
            .browse_db_table(DbBrowseRequestVm {
                table_name: "sources".to_string(),
                page: 1,
                ..DbBrowseRequestVm::default()
            })
            .unwrap();
        assert!(!browse.result_rows.is_empty());
    }

    #[test]
    fn rust_mode_query_guard_still_rejects_mutation() {
        let root = tempdir().unwrap();
        let (db, _) = rust_source(root.path());
        assert!(
            db.run_read_only_query("UPDATE sessions SET title = 'x'")
                .is_err()
        );
    }

    #[test]
    fn guard_rejects_multiple_or_mutating_statements() {
        assert!(guard_read_only_sql("SELECT 1; SELECT 2").is_err());
        assert!(guard_read_only_sql("UPDATE sessions SET title = 'x'").is_err());
        assert!(guard_read_only_sql("PRAGMA table_info('sessions')").is_ok());
    }
}
