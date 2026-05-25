use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::types::{
    CaptureSnapshot, DiscoveredCapture, DiscoveredSource, InstallStatus, JsonMap,
    NormalizedMessage, NormalizedSession, ParsedCapture, ParsedCaptureRecord, SourceConnector,
    SourceKind, SourcePathCheck,
};

#[derive(Clone, Debug)]
pub struct CodexConnector {
    home: PathBuf,
}

impl CodexConnector {
    #[cfg(test)]
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }

    pub fn from_env() -> Result<Self> {
        Ok(Self {
            home: resolve_codex_home()?,
        })
    }

    fn session_index(&self) -> BTreeMap<String, CodexSessionIndexRow> {
        let mut map = BTreeMap::new();
        let path = self.home.join("session_index.jsonl");
        let Ok(text) = fs::read_to_string(&path) else {
            return map;
        };

        for row in parse_jsonl_text(&text).unwrap_or_default() {
            let Some(id) = row.get("id").and_then(Value::as_str) else {
                continue;
            };

            let thread_name = row
                .get("thread_name")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let updated_at = row
                .get("updated_at")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            map.insert(
                id.to_string(),
                CodexSessionIndexRow {
                    thread_name,
                    updated_at,
                },
            );
        }

        map
    }
}

impl SourceConnector for CodexConnector {
    fn kind(&self) -> SourceKind {
        SourceKind::Codex
    }

    fn detect(&self) -> Result<DiscoveredSource> {
        let archived_sessions = self.home.join("archived_sessions");
        let live_sessions = self.home.join("sessions");
        let session_index = self.home.join("session_index.jsonl");
        let history = self.home.join("history.jsonl");
        let executable_path = find_executable("codex");

        let data_root_check = SourcePathCheck {
            label: "data_root".to_string(),
            path: self.home.display().to_string(),
            exists: self.home.exists(),
            file_count: None,
        };
        let archived_sessions_check = SourcePathCheck {
            label: "archived_sessions".to_string(),
            path: archived_sessions.display().to_string(),
            exists: archived_sessions.exists(),
            file_count: Some(count_files_matching(&archived_sessions, |path| {
                path.extension().and_then(|value| value.to_str()) == Some("jsonl")
            })),
        };
        let live_sessions_check = SourcePathCheck {
            label: "sessions".to_string(),
            path: live_sessions.display().to_string(),
            exists: live_sessions.exists(),
            file_count: Some(count_files_matching(&live_sessions, |path| {
                path.extension().and_then(|value| value.to_str()) == Some("jsonl")
            })),
        };
        let session_index_check = SourcePathCheck {
            label: "session_index".to_string(),
            path: session_index.display().to_string(),
            exists: session_index.exists(),
            file_count: Some(usize::from(session_index.exists())),
        };
        let history_check = SourcePathCheck {
            label: "history".to_string(),
            path: history.display().to_string(),
            exists: history.exists(),
            file_count: Some(usize::from(history.exists())),
        };
        let checks = vec![
            data_root_check.clone(),
            archived_sessions_check.clone(),
            live_sessions_check.clone(),
            session_index_check,
            history_check,
        ];

        let has_data_root = data_root_check.exists;
        let has_archived = archived_sessions_check.exists;
        let has_live = live_sessions_check.exists;
        let install_status =
            if executable_path.is_some() && has_data_root && (has_archived || has_live) {
                InstallStatus::Installed
            } else if executable_path.is_some() || checks.iter().any(|check| check.exists) {
                InstallStatus::Partial
            } else {
                InstallStatus::NotFound
            };

        let primary_capture_path = if has_live {
            live_sessions.display().to_string()
        } else {
            archived_sessions.display().to_string()
        };

        Ok(DiscoveredSource {
            kind: SourceKind::Codex,
            display_name: SourceKind::Codex.display_name().to_string(),
            executable_path,
            data_root: Some(self.home.clone()),
            install_status,
            checks,
            metadata: json_map(json!({
                "primaryCapturePath": primary_capture_path,
                "capturePaths": [
                    primary_capture_path,
                    archived_sessions.display().to_string(),
                    live_sessions.display().to_string()
                ],
                "auxiliaryFiles": [
                    session_index.display().to_string(),
                    history.display().to_string()
                ]
            })),
        })
    }

    fn discover_captures(&self) -> Result<Vec<DiscoveredCapture>> {
        let archived_root = self.home.join("archived_sessions");
        let live_root = self.home.join("sessions");

        let archived =
            discover_capture_files(&archived_root, "archived_session", SourceKind::Codex)?;
        let live = discover_capture_files(&live_root, "live_session", SourceKind::Codex)?;

        let mut captures_by_session = BTreeMap::new();
        let mut without_session = Vec::new();

        for capture in archived.into_iter().chain(live.into_iter()) {
            if let Some(external_session_id) = capture.external_session_id.as_ref() {
                captures_by_session.insert(external_session_id.clone(), capture);
            } else {
                without_session.push(capture);
            }
        }

        let mut discovered = captures_by_session.into_values().collect::<Vec<_>>();
        discovered.extend(without_session);
        discovered.sort_by(|left, right| left.source_path.cmp(&right.source_path));
        Ok(discovered)
    }

    fn snapshot_capture(&self, capture: &DiscoveredCapture) -> Result<CaptureSnapshot> {
        let raw_text = fs::read_to_string(&capture.source_path)
            .with_context(|| format!("failed to read Codex capture at {}", capture.source_path))?;
        let raw_sha256 = format!("{:x}", Sha256::digest(raw_text.as_bytes()));
        let source_size_bytes = u64::try_from(raw_text.len()).unwrap_or(u64::MAX);

        Ok(CaptureSnapshot {
            raw_text,
            raw_sha256,
            source_modified_at: capture.source_modified_at.clone(),
            source_size_bytes: Some(source_size_bytes),
        })
    }

    fn parse_capture(
        &self,
        capture: &DiscoveredCapture,
        snapshot: &CaptureSnapshot,
    ) -> Result<ParsedCapture> {
        let rows = parse_jsonl_text(&snapshot.raw_text)?;
        let session_index = self.session_index();
        let mut raw_records = Vec::new();
        let mut messages = Vec::new();

        let mut started_at = None;
        let mut updated_at = None;
        let mut external_session_id = capture.external_session_id.clone();
        let mut project_path = None;
        let mut model_provider = None;
        let mut cli_version = None;
        let mut model = None;

        for (index, row) in rows.iter().enumerate() {
            let record_type = row.get("type").and_then(Value::as_str).unwrap_or("unknown");
            let timestamp = row
                .get("timestamp")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let payload = row
                .get("payload")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let payload_type = payload
                .get("type")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let role = payload
                .get("role")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let content_text = extract_message_text(payload.get("content"));

            if updated_at
                .as_deref()
                .is_none_or(|current| timestamp.as_deref().is_some_and(|next| next > current))
            {
                updated_at = timestamp.clone().or(updated_at);
            }

            if record_type == "session_meta" {
                external_session_id = payload
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or(external_session_id);
                started_at = payload
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or(started_at);
                project_path = payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or(project_path);
                model_provider = payload
                    .get("model_provider")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or(model_provider);
                cli_version = payload
                    .get("cli_version")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or(cli_version);
            }

            if record_type == "turn_context" && model.is_none() {
                model = payload
                    .get("model")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or(model);
            }

            let canonical_message = record_type == "response_item"
                && payload_type.as_deref() == Some("message")
                && matches!(role.as_deref(), Some("user" | "assistant"));

            raw_records.push(ParsedCaptureRecord {
                line_no: index + 1,
                record_type: payload_type
                    .as_deref()
                    .map(|payload_type| format!("{record_type}:{payload_type}"))
                    .unwrap_or_else(|| record_type.to_string()),
                record_timestamp: timestamp.clone(),
                provider_message_id: None,
                parent_provider_message_id: None,
                role: role.clone(),
                is_meta: !canonical_message,
                content_text: (!content_text.is_empty()).then_some(content_text.clone()),
                content_json: row.clone(),
                metadata: Map::new(),
            });

            if canonical_message
                && !should_skip_codex_message(role.as_deref(), &content_text)
                && !content_text.is_empty()
            {
                messages.push(NormalizedMessage {
                    source_line_no: index + 1,
                    external_message_id: None,
                    parent_external_message_id: None,
                    role: role.unwrap_or_else(|| "user".to_string()),
                    text: content_text,
                    created_at: timestamp,
                    message_kind: "text".to_string(),
                    metadata: Map::new(),
                });
            }
        }

        let session_meta = external_session_id
            .as_ref()
            .and_then(|id| session_index.get(id))
            .cloned();
        let resolved_external_session_id = external_session_id
            .clone()
            .unwrap_or_else(|| capture_fallback_session_id(&capture.source_path));
        let external_session_id_provenance = if external_session_id.is_some() {
            json!({ "kind": "source" })
        } else {
            json!({
                "kind": "synthetic",
                "strategy": "capture_path_basename"
            })
        };

        Ok(ParsedCapture {
            session: NormalizedSession {
                source_kind: SourceKind::Codex,
                external_session_id: resolved_external_session_id,
                title: pick_codex_title(
                    external_session_id.as_deref(),
                    session_meta.as_ref(),
                    &messages,
                ),
                project_path,
                source_url: None,
                model,
                model_provider,
                cli_version,
                git_branch: None,
                started_at,
                updated_at: session_meta
                    .as_ref()
                    .and_then(|row| row.updated_at.clone())
                    .or(updated_at),
                summary: None,
                metadata: json_map(json!({
                    "capturePath": capture.source_path,
                    "externalSessionIdProvenance": external_session_id_provenance
                })),
            },
            messages,
            artifacts: Vec::new(),
            raw_records,
        })
    }
}

#[derive(Clone, Debug)]
struct CodexSessionIndexRow {
    thread_name: Option<String>,
    updated_at: Option<String>,
}

fn resolve_codex_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(home));
    }

    let home = dirs::home_dir().context("home directory is unavailable for CODEX_HOME fallback")?;
    Ok(home.join(".codex"))
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn count_files_matching(root: &Path, mut predicate: impl FnMut(&Path) -> bool) -> usize {
    visit_files(root)
        .into_iter()
        .filter(|path| predicate(path))
        .count()
}

fn visit_files(root: &Path) -> Vec<PathBuf> {
    let mut visited = std::collections::HashSet::new();
    visit_files_impl(root, &mut visited)
}

fn visit_files_impl(root: &Path, visited: &mut std::collections::HashSet<PathBuf>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if !root.exists() {
        return files;
    }

    let canonical = match fs::canonicalize(root) {
        Ok(path) => path,
        Err(_) => return files,
    };
    if !visited.insert(canonical) {
        return files;
    }

    let Ok(entries) = fs::read_dir(root) else {
        return files;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(visit_files_impl(&path, visited));
        } else if path.is_file() {
            files.push(path);
        }
    }

    files
}

fn discover_capture_files(
    root: &Path,
    capture_kind: &str,
    source_kind: SourceKind,
) -> Result<Vec<DiscoveredCapture>> {
    let mut captures = Vec::new();
    for file_path in visit_files(root)
        .into_iter()
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("jsonl"))
    {
        let stat = fs::metadata(&file_path)
            .with_context(|| format!("failed to read {}", file_path.display()))?;
        captures.push(DiscoveredCapture {
            source_kind,
            capture_kind: capture_kind.to_string(),
            source_path: file_path.display().to_string(),
            external_session_id: extract_session_id(&file_path),
            source_modified_at: stat.modified().ok().map(format_system_time),
            source_size_bytes: Some(stat.len()),
            metadata: Map::new(),
        });
    }
    Ok(captures)
}

fn format_system_time(value: std::time::SystemTime) -> String {
    let value = chrono::DateTime::<chrono::Utc>::from(value);
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn extract_session_id(path: &Path) -> Option<String> {
    let file_name = path.file_stem()?.to_str()?;
    let rest = file_name.strip_prefix("rollout-")?;
    let rest = if rest.len() > 19 && rest.as_bytes().get(10) == Some(&b'T') {
        rest.get(20..)
    } else {
        rest.get(11..)
    }?;
    let trimmed = rest.trim_start_matches('-').trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn parse_jsonl_text(raw_text: &str) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    for (index, line) in raw_text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(trimmed)
            .with_context(|| format!("invalid JSONL record on line {}", index + 1))?;
        rows.push(value);
    }
    Ok(rows)
}

fn extract_message_text(content: Option<&Value>) -> String {
    let Some(content) = content.and_then(Value::as_array) else {
        return String::new();
    };

    content
        .iter()
        .filter_map(|item| item.as_object())
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
}

fn should_skip_codex_message(role: Option<&str>, text: &str) -> bool {
    let Some(role) = role else {
        return true;
    };
    if role == "developer" || text.is_empty() {
        return true;
    }

    text.starts_with("<environment_context>")
        || text.starts_with("<local-command-caveat>")
        || text.starts_with("<permissions instructions>")
        || text.starts_with("<collaboration_mode>")
        || text.starts_with("<skills_instructions>")
        || text.starts_with("<image ")
        || text.starts_with("# AGENTS.md instructions for ")
        || text.starts_with("# CLAUDE.md instructions for ")
        || text.starts_with("# Repository Guidelines")
        || text.contains("\n<INSTRUCTIONS>\n")
}

fn pick_codex_title(
    external_session_id: Option<&str>,
    session_index: Option<&CodexSessionIndexRow>,
    messages: &[NormalizedMessage],
) -> Option<String> {
    if let Some(title) = session_index
        .and_then(|row| row.thread_name.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(title.to_string());
    }

    let _ = external_session_id;
    messages
        .iter()
        .find(|message| message.role == "user")
        .and_then(|message| message.text.lines().next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(160).collect::<String>())
}

fn capture_fallback_session_id(source_path: &str) -> String {
    Path::new(source_path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(source_path)
        .to_string()
}

fn json_map(value: Value) -> JsonMap {
    value.as_object().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::CodexConnector;
    use crate::connectors::SourceConnector;
    use tempfile::tempdir;

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

    fn install_fixture(root: &Path, name: &str) {
        copy_tree(&fixture_root(name), root);
    }

    #[test]
    fn discover_prefers_live_codex_capture_when_live_and_archived_exist() {
        let temp = tempdir().unwrap();
        install_fixture(temp.path(), "codex-live-session");
        install_fixture(temp.path(), "codex-archived-duplicate");

        let connector = CodexConnector::new(temp.path().join(".codex"));
        let captures = connector.discover_captures().unwrap();

        assert_eq!(captures.len(), 1);
        assert_eq!(
            captures[0].external_session_id.as_deref(),
            Some("abc12345-1111-2222-3333-abcdefabcdef")
        );
        assert_eq!(captures[0].capture_kind, "live_session");
        assert!(captures[0].source_path.contains("/.codex/sessions/"));
    }

    #[test]
    fn parse_codex_capture_emits_canonical_messages_and_raw_records() {
        let temp = tempdir().unwrap();
        install_fixture(temp.path(), "codex-live-session");

        let connector = CodexConnector::new(temp.path().join(".codex"));
        let capture = connector.discover_captures().unwrap().remove(0);
        let snapshot = connector.snapshot_capture(&capture).unwrap();
        let parsed = connector.parse_capture(&capture, &snapshot).unwrap();

        assert_eq!(
            parsed.session.external_session_id,
            "abc12345-1111-2222-3333-abcdefabcdef"
        );
        assert_eq!(parsed.messages.len(), 2);
        assert!(parsed.artifacts.is_empty());
        assert!(parsed.raw_records.len() >= 4);
        assert_eq!(parsed.messages[0].role, "user");
        assert_eq!(parsed.messages[1].role, "assistant");
    }
}
