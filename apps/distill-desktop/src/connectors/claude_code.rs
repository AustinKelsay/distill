use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::types::{
    CaptureSnapshot, DiscoveredCapture, DiscoveredSource, InstallStatus, JsonMap,
    NormalizedArtifact, NormalizedMessage, NormalizedSession, ParsedCapture, ParsedCaptureRecord,
    SourceConnector, SourceKind, SourcePathCheck,
};

#[derive(Clone, Debug)]
pub struct ClaudeCodeConnector {
    home: PathBuf,
}

impl ClaudeCodeConnector {
    #[cfg(test)]
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }

    pub fn from_env() -> Result<Self> {
        Ok(Self {
            home: resolve_claude_home()?,
        })
    }

    fn history_index(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        let path = self.home.join("history.jsonl");
        let Ok(text) = fs::read_to_string(&path) else {
            return map;
        };

        for row in parse_jsonl_text(&text).unwrap_or_default() {
            let Some(session_id) = row.get("sessionId").and_then(Value::as_str) else {
                continue;
            };
            let Some(display) = row.get("display").and_then(Value::as_str) else {
                continue;
            };
            if !map.contains_key(session_id) {
                map.insert(session_id.to_string(), display.to_string());
            }
        }

        map
    }
}

impl SourceConnector for ClaudeCodeConnector {
    fn kind(&self) -> SourceKind {
        SourceKind::ClaudeCode
    }

    fn detect(&self) -> Result<DiscoveredSource> {
        let projects = self.home.join("projects");
        let history = self.home.join("history.jsonl");
        let settings = self.home.join("settings.json");
        let executable_path = find_executable("claude");

        let checks = vec![
            SourcePathCheck {
                label: "data_root".to_string(),
                path: self.home.display().to_string(),
                exists: self.home.exists(),
                file_count: None,
            },
            SourcePathCheck {
                label: "projects".to_string(),
                path: projects.display().to_string(),
                exists: projects.exists(),
                file_count: Some(count_files_matching(&projects, |path| {
                    path.extension().and_then(|value| value.to_str()) == Some("jsonl")
                })),
            },
            SourcePathCheck {
                label: "history".to_string(),
                path: history.display().to_string(),
                exists: history.exists(),
                file_count: Some(usize::from(history.exists())),
            },
            SourcePathCheck {
                label: "settings".to_string(),
                path: settings.display().to_string(),
                exists: settings.exists(),
                file_count: Some(usize::from(settings.exists())),
            },
        ];

        let install_status = if executable_path.is_some() && checks[0].exists && checks[1].exists {
            InstallStatus::Installed
        } else if executable_path.is_some() || checks.iter().any(|check| check.exists) {
            InstallStatus::Partial
        } else {
            InstallStatus::NotFound
        };

        Ok(DiscoveredSource {
            kind: SourceKind::ClaudeCode,
            display_name: "Claude Code".to_string(),
            executable_path,
            data_root: Some(self.home.clone()),
            install_status,
            checks,
            metadata: json_map(json!({
                "primaryCapturePath": projects.display().to_string(),
                "auxiliaryFiles": [
                    history.display().to_string(),
                    settings.display().to_string()
                ]
            })),
        })
    }

    fn discover_captures(&self) -> Result<Vec<DiscoveredCapture>> {
        let projects = self.home.join("projects");
        let mut captures = Vec::new();

        for file_path in visit_files(&projects)
            .into_iter()
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("jsonl"))
        {
            let stat = fs::metadata(&file_path)
                .with_context(|| format!("failed to read {}", file_path.display()))?;
            captures.push(DiscoveredCapture {
                source_kind: SourceKind::ClaudeCode,
                capture_kind: "project_session".to_string(),
                source_path: file_path.display().to_string(),
                external_session_id: file_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(ToOwned::to_owned),
                source_modified_at: stat.modified().ok().map(format_system_time),
                source_size_bytes: Some(stat.len()),
                metadata: json_map(json!({
                    "projectFolder": file_path
                        .parent()
                        .map(|value| value.display().to_string())
                        .unwrap_or_default()
                })),
            });
        }

        Ok(captures)
    }

    fn snapshot_capture(&self, capture: &DiscoveredCapture) -> Result<CaptureSnapshot> {
        let raw_text = fs::read_to_string(&capture.source_path).with_context(|| {
            format!(
                "failed to read Claude Code capture at {}",
                capture.source_path
            )
        })?;
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
        let history_index = self.history_index();
        let mut raw_records = Vec::new();
        let mut messages = Vec::new();
        let mut artifacts = Vec::new();

        let mut session_id = capture.external_session_id.clone();
        let mut started_at = None;
        let mut updated_at = None;
        let mut project_path = capture
            .metadata
            .get("projectFolder")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let mut git_branch = None;

        for (index, row) in rows.iter().enumerate() {
            let record_type = row.get("type").and_then(Value::as_str).unwrap_or("unknown");
            let timestamp = row
                .get("timestamp")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let uuid = row
                .get("uuid")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let parent_uuid = row
                .get("parentUuid")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let record_session_id = row
                .get("sessionId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let cwd = row
                .get("cwd")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let branch = row
                .get("gitBranch")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let is_meta = row.get("isMeta").and_then(Value::as_bool).unwrap_or(false);
            let message = row.get("message").and_then(Value::as_object).cloned();
            let role = message
                .as_ref()
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let blocks = normalize_content_blocks(
                message
                    .as_ref()
                    .and_then(|message| message.get("content"))
                    .cloned(),
            );
            let content_text = extract_text_blocks(&blocks);

            session_id = record_session_id.or(session_id);
            project_path = cwd.or(project_path);
            git_branch = branch.or(git_branch);

            if started_at
                .as_deref()
                .is_none_or(|current| timestamp.as_deref().is_some_and(|next| next < current))
            {
                started_at = timestamp.clone().or(started_at);
            }
            if updated_at
                .as_deref()
                .is_none_or(|current| timestamp.as_deref().is_some_and(|next| next > current))
            {
                updated_at = timestamp.clone().or(updated_at);
            }

            raw_records.push(ParsedCaptureRecord {
                line_no: index + 1,
                record_type: record_type.to_string(),
                record_timestamp: timestamp.clone(),
                provider_message_id: uuid.clone(),
                parent_provider_message_id: parent_uuid.clone(),
                role: role.clone(),
                is_meta,
                content_text: (!content_text.is_empty()).then_some(content_text.clone()),
                content_json: row.clone(),
                metadata: Map::new(),
            });

            if matches!(record_type, "user" | "assistant")
                && !is_meta
                && role
                    .as_deref()
                    .is_some_and(|role| matches!(role, "user" | "assistant"))
                && !content_text.is_empty()
                && !is_suppressed_claude_text(&content_text)
            {
                messages.push(NormalizedMessage {
                    source_line_no: index + 1,
                    external_message_id: uuid.clone(),
                    parent_external_message_id: parent_uuid.clone(),
                    role: role.clone().unwrap_or_else(|| "user".to_string()),
                    text: content_text.clone(),
                    created_at: timestamp.clone(),
                    message_kind: "text".to_string(),
                    metadata: Map::new(),
                });
            }

            for block in blocks {
                let block_type = block.get("type").and_then(Value::as_str);
                match block_type {
                    Some("image") => artifacts.push(NormalizedArtifact {
                        source_line_no: index + 1,
                        external_message_id: uuid.clone(),
                        kind: "image".to_string(),
                        mime_type: block
                            .get("source")
                            .and_then(Value::as_object)
                            .and_then(|source| source.get("media_type"))
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        payload: block.clone(),
                    }),
                    Some("tool_use") => artifacts.push(NormalizedArtifact {
                        source_line_no: index + 1,
                        external_message_id: uuid.clone(),
                        kind: "tool_call".to_string(),
                        mime_type: None,
                        payload: block.clone(),
                    }),
                    Some("tool_result") => artifacts.push(NormalizedArtifact {
                        source_line_no: index + 1,
                        external_message_id: uuid.clone(),
                        kind: "tool_result".to_string(),
                        mime_type: None,
                        payload: block.clone(),
                    }),
                    _ => {}
                }
            }
        }

        let resolved_session_id = session_id.clone().unwrap_or_else(|| {
            Path::new(&capture.source_path)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or(&capture.source_path)
                .to_string()
        });
        let external_session_id_provenance = if session_id.is_some() {
            json!({ "kind": "source" })
        } else {
            json!({
                "kind": "synthetic",
                "strategy": "capture_path_basename"
            })
        };

        Ok(ParsedCapture {
            session: NormalizedSession {
                source_kind: SourceKind::ClaudeCode,
                external_session_id: resolved_session_id.clone(),
                title: pick_claude_title(&resolved_session_id, &history_index, &messages),
                project_path,
                source_url: None,
                model: None,
                model_provider: None,
                cli_version: None,
                git_branch,
                started_at,
                updated_at,
                summary: None,
                metadata: json_map(json!({
                    "capturePath": capture.source_path,
                    "externalSessionIdProvenance": external_session_id_provenance
                })),
            },
            messages,
            artifacts,
            raw_records,
        })
    }
}

fn resolve_claude_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("CLAUDE_HOME") {
        return Ok(PathBuf::from(home));
    }
    let home =
        dirs::home_dir().context("home directory is unavailable for CLAUDE_HOME fallback")?;
    Ok(home.join(".claude"))
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
    let mut files = Vec::new();
    if !root.exists() {
        return files;
    }

    let Ok(entries) = fs::read_dir(root) else {
        return files;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(visit_files(&path));
        } else if path.is_file() {
            files.push(path);
        }
    }

    files
}

fn format_system_time(value: std::time::SystemTime) -> String {
    let value = chrono::DateTime::<chrono::Utc>::from(value);
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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

fn normalize_content_blocks(content: Option<Value>) -> Vec<JsonMap> {
    match content {
        Some(Value::String(text)) => vec![json_map(json!({ "type": "text", "text": text }))],
        Some(Value::Array(items)) => items
            .into_iter()
            .filter_map(|item| item.as_object().cloned())
            .collect(),
        _ => Vec::new(),
    }
}

fn extract_text_blocks(blocks: &[JsonMap]) -> String {
    blocks
        .iter()
        .filter_map(|block| {
            let block_type = block.get("type").and_then(Value::as_str);
            let text = block.get("text").and_then(Value::as_str);
            if block_type == Some("text") {
                text.map(str::trim).filter(|value| !value.is_empty())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
}

fn is_suppressed_claude_text(text: &str) -> bool {
    text.starts_with("<local-command-caveat>")
        || text.starts_with("<command-name>")
        || text.starts_with("<local-command-stdout>")
        || text.starts_with("[Image: original ")
}

fn pick_claude_title(
    session_id: &str,
    history_index: &BTreeMap<String, String>,
    messages: &[NormalizedMessage],
) -> Option<String> {
    if let Some(title) = history_index
        .get(session_id)
        .map(String::as_str)
        .filter(|title| !is_suppressed_claude_text(title))
    {
        return Some(title.to_string());
    }

    messages
        .iter()
        .find(|message| message.role == "user")
        .and_then(|message| message.text.lines().next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(160).collect::<String>())
}

fn json_map(value: Value) -> JsonMap {
    value.as_object().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::ClaudeCodeConnector;
    use crate::connectors::SourceConnector;

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

    #[test]
    fn parse_claude_capture_preserves_text_and_structured_artifacts() {
        let temp = tempdir().unwrap();
        copy_tree(&fixture_root("claude-mixed-blocks"), temp.path());

        let connector = ClaudeCodeConnector::new(temp.path().join(".claude"));
        let capture = connector.discover_captures().unwrap().remove(0);
        let snapshot = connector.snapshot_capture(&capture).unwrap();
        let parsed = connector.parse_capture(&capture, &snapshot).unwrap();

        assert_eq!(
            parsed.session.external_session_id,
            "123e4567-e89b-12d3-a456-426614174000"
        );
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(
            parsed
                .messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Please review the screenshot and fix the layout.",
                "I will tighten the layout."
            ]
        );
        assert_eq!(
            parsed
                .artifacts
                .iter()
                .map(|artifact| artifact.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["image", "tool_call", "tool_result"]
        );
    }
}
