//! Codex SourceAdapter: detect/discover/snapshot/parse for a configured Codex home.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::{
    CaptureCandidate, CaptureSnapshot, DiscoveredSource, ParsedArtifact, ParsedCapture, ParsedFact,
    ParsedMessage, ParserIdentity, SourceAdapter, SourceKind, SourceStageError,
};

/// Default Codex parser identity for Normalization Attempts.
pub const CODEX_PARSER_ID: &str = "codex";

/// Default Codex parser contract version.
pub const CODEX_PARSER_VERSION: &str = "1.0.0";

/// Media type for Codex session JSONL Captures.
pub const CODEX_MEDIA_TYPE: &str = "application/x-distill-codex+jsonl";

/// Live session root under a Codex home.
const LIVE_SESSIONS_DIR: &str = "sessions";

/// Archived session root under a Codex home.
const ARCHIVED_SESSIONS_DIR: &str = "archived_sessions";

/// Codex SourceAdapter bound to one configured Codex home root.
pub struct CodexAdapter {
    root: PathBuf,
    parser: ParserIdentity,
}

impl CodexAdapter {
    /// Create an adapter that detects only the supplied Codex home root.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_parser(
            root,
            ParserIdentity {
                id: CODEX_PARSER_ID.to_string(),
                version: CODEX_PARSER_VERSION.to_string(),
            },
        )
    }

    /**
     * Create an adapter with an explicit Codex parser identity.
     *
     * Parameters:
     * - `root`: Configured Codex home containing `sessions/` and/or `archived_sessions/`.
     * - `parser`: Parser identity/version recorded on Normalization Attempts.
     */
    pub fn with_parser(root: impl Into<PathBuf>, parser: ParserIdentity) -> Self {
        Self {
            root: root.into(),
            parser,
        }
    }
}

impl SourceAdapter for CodexAdapter {
    fn detect(&self) -> Result<DiscoveredSource, SourceStageError> {
        let root = canonicalize_existing(&self.root).map_err(SourceStageError::Detect)?;
        if !root.is_dir() {
            return Err(SourceStageError::Detect(format!(
                "codex root is not a directory: {}",
                root.display()
            )));
        }
        Ok(DiscoveredSource {
            kind: SourceKind::Codex,
            display_name: "Codex".to_string(),
            data_root: root,
            parser: self.parser.clone(),
        })
    }

    fn discover(
        &self,
        source: &DiscoveredSource,
    ) -> Result<Vec<CaptureCandidate>, SourceStageError> {
        let live_root = source.data_root.join(LIVE_SESSIONS_DIR);
        let archived_root = source.data_root.join(ARCHIVED_SESSIONS_DIR);

        let archived = discover_jsonl_candidates(&source.data_root, &archived_root, "archived")
            .map_err(SourceStageError::Discover)?;
        let live = discover_jsonl_candidates(&source.data_root, &live_root, "live")
            .map_err(SourceStageError::Discover)?;

        // Archived first, then live: identical Session Identity resolves to live.
        let mut by_session: HashMap<String, CaptureCandidate> = HashMap::new();
        let mut without_session = Vec::new();
        for candidate in archived.into_iter().chain(live) {
            match candidate.external_session_id.clone() {
                Some(session_id) => {
                    by_session.insert(session_id, candidate);
                }
                None => without_session.push(candidate),
            }
        }

        let mut discovered: Vec<CaptureCandidate> = by_session.into_values().collect();
        discovered.extend(without_session);
        discovered.sort_by(|left, right| left.source_path.cmp(&right.source_path));
        Ok(discovered)
    }

    fn snapshot(&self, candidate: &CaptureCandidate) -> Result<CaptureSnapshot, SourceStageError> {
        let path = candidate
            .absolute_path
            .as_ref()
            .ok_or_else(|| SourceStageError::Snapshot("missing absolute path".into()))?;
        let bytes = fs::read(path).map_err(|err| SourceStageError::Snapshot(err.to_string()))?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let source_modified_at = fs::metadata(path)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .map(|modified| {
                let datetime: chrono::DateTime<chrono::Utc> = modified.into();
                datetime.to_rfc3339()
            });
        Ok(CaptureSnapshot {
            byte_size: bytes.len() as u64,
            bytes,
            sha256,
            media_type: candidate.media_type.clone(),
            source_modified_at,
        })
    }

    fn parse(
        &self,
        candidate: &CaptureCandidate,
        snapshot: &CaptureSnapshot,
    ) -> Result<ParsedCapture, SourceStageError> {
        parse_codex_jsonl(candidate, &snapshot.bytes, &self.root)
    }
}

/**
 * Parse Codex session JSONL bytes into Capture Facts, Messages, and Artifacts.
 *
 * Parameters:
 * - `candidate`: Capture Candidate providing identity and path hints.
 * - `bytes`: Exact snapshot bytes preserved by Distill.
 * - `codex_root`: Configured Codex home used to read auxiliary session index metadata.
 */
fn parse_codex_jsonl(
    candidate: &CaptureCandidate,
    bytes: &[u8],
    codex_root: &Path,
) -> Result<ParsedCapture, SourceStageError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|err| SourceStageError::Parse(format!("codex bytes are not utf-8: {err}")))?;

    let session_index = read_session_index(codex_root);
    let mut facts = Vec::new();
    let mut messages = Vec::new();
    let mut artifacts = Vec::new();

    let mut started_at = None;
    let mut updated_at = None;
    let mut external_session_id = candidate.external_session_id.clone();
    let mut project_path = None;
    let mut model_provider = None;
    let mut cli_version = None;
    let mut model = None;

    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = crate::privacy::parse_json_line_bounded(trimmed, line_no + 1)
            .map_err(SourceStageError::Parse)?;

        let record_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .map(str::to_string);
        let payload = value
            .get("payload")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let payload_type = payload
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string);
        let role = payload
            .get("role")
            .and_then(Value::as_str)
            .map(str::to_string);
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
                .map(str::to_string)
                .or(external_session_id);
            started_at = payload
                .get("timestamp")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(started_at);
            project_path = payload
                .get("cwd")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(project_path);
            model_provider = payload
                .get("model_provider")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(model_provider);
            cli_version = payload
                .get("cli_version")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(cli_version);
        }

        if record_type == "turn_context" && model.is_none() {
            model = payload
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string);
        }

        let is_canonical_message = record_type == "response_item"
            && payload_type.as_deref() == Some("message")
            && matches!(role.as_deref(), Some("user" | "assistant"));
        let is_message_record =
            record_type == "response_item" && payload_type.as_deref() == Some("message");
        let skip_noise =
            is_message_record && should_skip_codex_message(role.as_deref(), &content_text);
        let project_message = is_canonical_message && !skip_noise && !content_text.is_empty();

        let fact_record_type = payload_type
            .as_deref()
            .map(|payload_type| format!("{record_type}:{payload_type}"))
            .unwrap_or_else(|| record_type.to_string());
        let fact_ordinal = facts.len();
        facts.push(ParsedFact {
            record_type: fact_record_type.clone(),
            role: role.clone(),
            is_meta: !project_message,
            content_text: (!content_text.is_empty()).then_some(content_text.clone()),
            content_json: value.clone(),
        });

        if project_message {
            messages.push(ParsedMessage {
                role: role.clone().unwrap_or_else(|| "user".to_string()),
                message_kind: "text".to_string(),
                text: content_text,
                external_message_id: None,
            });
        } else if record_type != "session_meta" && !skip_noise {
            // Retain reasoning, tool, unknown-role, and other meta rows as Artifacts.
            artifacts.push(ParsedArtifact {
                artifact_type: fact_record_type,
                message_ordinal: None,
                fact_ordinal: Some(fact_ordinal),
                text_preview: (!content_text.is_empty()).then_some(content_text),
                content_json: value,
            });
        }
    }

    let source_provided_id = external_session_id.clone();
    let synthetic_identity = source_provided_id.is_none();
    let resolved_external_session_id =
        source_provided_id.unwrap_or_else(|| synthetic_session_id(candidate));

    let index_row = session_index.get(&resolved_external_session_id);
    let title = pick_codex_title(index_row, &messages);
    if let Some(index_updated) = index_row.and_then(|row| row.updated_at.clone()) {
        updated_at = Some(index_updated);
    }

    let provenance = if synthetic_identity {
        json!({
            "kind": "synthetic",
            "strategy": "source_path_sha256"
        })
    } else {
        json!({ "kind": "source" })
    };

    let mut metadata = Map::new();
    metadata.insert("external_session_id_provenance".to_string(), provenance);
    if synthetic_identity {
        metadata.insert("synthetic_identity".to_string(), json!(true));
        metadata.insert(
            "source_path".to_string(),
            json!(candidate.source_path.clone()),
        );
    }
    if let Some(model) = model.clone() {
        metadata.insert("model".to_string(), json!(model));
    }
    if let Some(model_provider) = model_provider {
        metadata.insert("model_provider".to_string(), json!(model_provider));
    }
    if let Some(cli_version) = cli_version {
        metadata.insert("cli_version".to_string(), json!(cli_version));
    }

    Ok(ParsedCapture {
        external_session_id: resolved_external_session_id,
        synthetic_identity,
        title,
        summary: None,
        project_path,
        source_url: None,
        started_at,
        updated_at,
        metadata: Value::Object(metadata),
        facts,
        messages,
        artifacts,
    })
}

#[derive(Clone, Debug)]
struct CodexSessionIndexRow {
    thread_name: Option<String>,
    updated_at: Option<String>,
}

/**
 * Read auxiliary `session_index.jsonl` when present. Missing or invalid rows are ignored.
 */
fn read_session_index(codex_root: &Path) -> HashMap<String, CodexSessionIndexRow> {
    let mut map = HashMap::new();
    let path = codex_root.join("session_index.jsonl");
    let Ok(text) = fs::read_to_string(&path) else {
        return map;
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let Some(id) = row.get("id").and_then(Value::as_str) else {
            continue;
        };
        map.insert(
            id.to_string(),
            CodexSessionIndexRow {
                thread_name: row
                    .get("thread_name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                updated_at: row
                    .get("updated_at")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
        );
    }
    map
}

/**
 * Recursively discover `.jsonl` session files under a live or archived root.
 *
 * Parameters:
 * - `data_root`: Configured Codex home used for logical `source_path` identity.
 * - `scan_root`: Live or archived directory to walk.
 * - `capture_kind`: `live` or `archived` label embedded in logical identity.
 */
fn discover_jsonl_candidates(
    data_root: &Path,
    scan_root: &Path,
    capture_kind: &str,
) -> Result<Vec<CaptureCandidate>, String> {
    if !scan_root.exists() {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::new();
    let mut file_paths = visit_files(scan_root)?;
    file_paths.sort();
    for file_path in file_paths {
        if file_path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let relative = file_path
            .strip_prefix(data_root)
            .unwrap_or(file_path.as_path())
            .display()
            .to_string()
            .replace('\\', "/");
        candidates.push(CaptureCandidate {
            source_kind: SourceKind::Codex,
            source_path: format!("codex://{capture_kind}/{relative}"),
            absolute_path: Some(file_path.clone()),
            external_session_id: extract_session_id_from_content(&file_path)
                .or_else(|| extract_session_id(&file_path)),
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: CODEX_MEDIA_TYPE.to_string(),
        });
    }
    Ok(candidates)
}

/**
 * Recover a provider Session Identity from a session metadata row when the
 * filename does not use Codex's rollout naming convention.
 *
 * Discovery only peeks at the metadata id; the complete source bytes are still
 * snapshotted by [`SourceAdapter::snapshot`] before parsing or persistence.
 */
fn extract_session_id_from_content(path: &Path) -> Option<String> {
    const ID_SCAN_LIMIT: u64 = 1024 * 1024;
    let file = fs::File::open(path).ok()?;
    for line in BufReader::new(file).take(ID_SCAN_LIMIT).lines() {
        let Ok(line) = line else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        if let Some(id) = value
            .pointer("/payload/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            return Some(id.to_string());
        }
    }
    None
}

/**
 * Walk files under `root`, skipping directory cycles via canonical paths.
 */
fn visit_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut visited = HashSet::new();
    visit_files_impl(root, &mut visited)
}

fn visit_files_impl(root: &Path, visited: &mut HashSet<PathBuf>) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    let canonical = fs::canonicalize(root).map_err(|err| format!("{}: {err}", root.display()))?;
    if !visited.insert(canonical) {
        return Ok(files);
    }
    let entries = fs::read_dir(root).map_err(|err| format!("{}: {err}", root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("{}: {err}", root.display()))?;
        let file_type = entry
            .file_type()
            .map_err(|err| format!("{}: {err}", root.display()))?;
        // Hostile corpus: never follow directory/file symlinks during discovery.
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            files.extend(visit_files_impl(&path, visited)?);
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(files)
}

/**
 * Extract the provider Session Identity from a Codex rollout filename when present.
 */
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

/**
 * Extract visible text from Codex message content (string or structured array).
 */
fn extract_message_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_object())
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
            .trim()
            .to_string(),
        _ => String::new(),
    }
}

/**
 * Skip Codex bootstrap/developer instruction noise from projected transcript Messages.
 */
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

/**
 * Prefer session-index thread names, else the first user message line (truncated).
 */
fn pick_codex_title(
    index_row: Option<&CodexSessionIndexRow>,
    messages: &[ParsedMessage],
) -> Option<String> {
    if let Some(title) = index_row
        .and_then(|row| row.thread_name.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(title.to_string());
    }
    messages
        .iter()
        .find(|message| message.role == "user")
        .and_then(|message| message.text.lines().next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(160).collect())
}

/**
 * Derive a deterministic synthetic Session Identity from the candidate path.
 */
fn synthetic_session_id(candidate: &CaptureCandidate) -> String {
    let digest = Sha256::digest(candidate.source_path.as_bytes());
    format!("synthetic-{}", &hex::encode(digest)[..16])
}

/**
 * Canonicalize an existing path, mapping IO failures into strings.
 */
fn canonicalize_existing(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|err| format!("{}: {err}", path.display()))
}

/**
 * Locate an executable on PATH without shelling out.
 *
 * Parameters:
 * - `name`: Executable basename such as `codex`.
 */
pub(crate) fn find_executable(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /**
     * Build a minimal Codex home with live and optional archived session files.
     */
    fn write_session(root: &Path, relative: &str, body: &str) -> PathBuf {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("session parent");
        }
        fs::write(&path, body).expect("write session");
        path
    }

    fn dialogue_body() -> String {
        concat!(
            r#"{"timestamp":"2026-03-25T10:00:00.000Z","type":"session_meta","payload":{"id":"abc12345-1111-2222-3333-abcdefabcdef","timestamp":"2026-03-25T10:00:00.000Z","cwd":"/tmp/demo","cli_version":"1.2.3","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-03-25T10:00:00.500Z","type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
            "\n",
            r#"{"timestamp":"2026-03-25T10:00:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n  <cwd>/tmp/demo</cwd>"}]}}"#,
            "\n",
            r#"{"timestamp":"2026-03-25T10:00:02.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello codex"}]}}"#,
            "\n",
            r#"{"timestamp":"2026-03-25T10:00:03.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"I will update the code."}]}}"#,
            "\n",
            r#"{"timestamp":"2026-03-25T10:00:04.000Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"cmd\":\"ls\"}"}}"#,
            "\n",
            r#"{"timestamp":"2026-03-25T10:00:05.000Z","type":"response_item","payload":{"type":"reasoning","summary":[{"text":"consider options"}]}}"#,
            "\n",
            r#"{"timestamp":"2026-03-25T10:00:06.000Z","type":"response_item","payload":{"type":"message","role":"system","content":[{"type":"output_text","text":"unknown role stays fact"}]}}"#,
            "\n",
            r#"{"timestamp":"2026-03-25T10:00:07.000Z","type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"developer bootstrap stays fact-only"}]}}"#,
            "\n",
        )
        .to_string()
    }

    #[test]
    fn detect_reports_configured_codex_home() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("codex-home");
        fs::create_dir_all(root.join("sessions")).expect("sessions");
        let adapter = CodexAdapter::new(&root);
        let discovered = adapter.detect().expect("detect");
        assert_eq!(discovered.kind, SourceKind::Codex);
        assert_eq!(discovered.display_name, "Codex");
        assert_eq!(discovered.parser.id, CODEX_PARSER_ID);
        assert!(discovered.data_root.ends_with("codex-home"));
    }

    #[test]
    fn discover_prefers_live_over_archived_duplicate() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("codex-home");
        write_session(
            &root,
            "archived_sessions/rollout-2026-03-24T09-55-00-abc12345-1111-2222-3333-abcdefabcdef.jsonl",
            r#"{"type":"session_meta","payload":{"id":"abc12345-1111-2222-3333-abcdefabcdef"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"archived"}]}}
"#,
        );
        write_session(
            &root,
            "sessions/2026/03/25/rollout-2026-03-25T10-00-00-abc12345-1111-2222-3333-abcdefabcdef.jsonl",
            &dialogue_body(),
        );
        fs::write(
            root.join("session_index.jsonl"),
            r#"{"id":"abc12345-1111-2222-3333-abcdefabcdef","thread_name":"Demo Thread"}
"#,
        )
        .expect("index");
        fs::write(root.join("history.jsonl"), "{}\n").expect("history");

        let adapter = CodexAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidates = adapter.discover(&source).expect("discover");
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].external_session_id.as_deref(),
            Some("abc12345-1111-2222-3333-abcdefabcdef")
        );
        assert!(candidates[0]
            .absolute_path
            .as_ref()
            .expect("path")
            .display()
            .to_string()
            .contains("/sessions/"));
        assert!(!candidates[0]
            .absolute_path
            .as_ref()
            .expect("path")
            .display()
            .to_string()
            .contains("/archived_sessions/"));
    }

    #[test]
    fn snapshot_preserves_exact_bytes_and_sha256() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("codex-home");
        let body = dialogue_body();
        let path = write_session(
            &root,
            "sessions/rollout-2026-03-25T10-00-00-abc12345-1111-2222-3333-abcdefabcdef.jsonl",
            &body,
        );
        let adapter = CodexAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidate = adapter.discover(&source).expect("discover").remove(0);
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        assert_eq!(snapshot.bytes, body.as_bytes());
        assert_eq!(
            snapshot.sha256,
            hex::encode(Sha256::digest(body.as_bytes()))
        );
        assert_eq!(snapshot.byte_size, body.len() as u64);
        assert_eq!(fs::read(&path).expect("reread"), snapshot.bytes);
    }

    #[test]
    fn parse_dialogue_tool_reasoning_metadata_and_unknown_role() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("codex-home");
        write_session(
            &root,
            "sessions/rollout-2026-03-25T10-00-00-abc12345-1111-2222-3333-abcdefabcdef.jsonl",
            &dialogue_body(),
        );
        fs::write(
            root.join("session_index.jsonl"),
            r#"{"id":"abc12345-1111-2222-3333-abcdefabcdef","thread_name":"Demo Thread","updated_at":"2026-03-25T11:00:00.000Z"}
"#,
        )
        .expect("index");

        let adapter = CodexAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidate = adapter.discover(&source).expect("discover").remove(0);
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        let parsed = adapter.parse(&candidate, &snapshot).expect("parse");

        assert_eq!(
            parsed.external_session_id,
            "abc12345-1111-2222-3333-abcdefabcdef"
        );
        assert!(!parsed.synthetic_identity);
        assert_eq!(parsed.title.as_deref(), Some("Demo Thread"));
        assert_eq!(parsed.project_path.as_deref(), Some("/tmp/demo"));
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].role, "user");
        assert_eq!(parsed.messages[0].text, "hello codex");
        assert_eq!(parsed.messages[1].role, "assistant");
        assert!(parsed.facts.len() >= 6);
        assert!(parsed
            .artifacts
            .iter()
            .any(|artifact| artifact.artifact_type.contains("function_call")));
        assert!(parsed
            .artifacts
            .iter()
            .any(|artifact| artifact.artifact_type.contains("reasoning")));
        assert!(parsed
            .artifacts
            .iter()
            .any(|artifact| artifact.artifact_type.contains("message")
                && artifact
                    .content_json
                    .pointer("/payload/role")
                    .and_then(Value::as_str)
                    == Some("system")));
        assert!(!parsed.artifacts.iter().any(|artifact| {
            artifact
                .content_json
                .pointer("/payload/role")
                .and_then(Value::as_str)
                == Some("developer")
        }));
        assert!(!parsed.artifacts.iter().any(|artifact| {
            artifact
                .content_json
                .pointer("/payload/content/0/text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.starts_with("<environment_context>"))
        }));
        assert!(parsed
            .facts
            .iter()
            .all(|fact| fact.content_json.is_object()));
        assert_eq!(
            parsed.metadata.get("model").and_then(Value::as_str),
            Some("gpt-5.4")
        );
    }

    #[test]
    fn parse_synthesizes_deterministic_identity_without_session_id() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("codex-home");
        write_session(
            &root,
            "sessions/orphan-session.jsonl",
            concat!(
                r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"orphan"}]}}"#,
                "\n",
            ),
        );
        let adapter = CodexAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidate = adapter.discover(&source).expect("discover").remove(0);
        assert!(candidate.external_session_id.is_none());
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        let parsed = adapter.parse(&candidate, &snapshot).expect("parse");
        assert!(parsed.synthetic_identity);
        assert!(parsed.external_session_id.starts_with("synthetic-"));
        assert_eq!(
            parsed
                .metadata
                .pointer("/external_session_id_provenance/kind")
                .and_then(Value::as_str),
            Some("synthetic")
        );
        let again = adapter.parse(&candidate, &snapshot).expect("parse again");
        assert_eq!(parsed.external_session_id, again.external_session_id);
    }

    #[test]
    fn parse_tolerates_string_content() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("codex-home");
        write_session(
            &root,
            "sessions/rollout-2026-03-25T10-00-00-string-content-session.jsonl",
            concat!(
                r#"{"type":"session_meta","payload":{"id":"string-content-session"}}"#,
                "\n",
                r#"{"type":"response_item","payload":{"type":"message","role":"user","content":"plain string user"}}"#,
                "\n",
                r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":"plain string assistant"}}"#,
                "\n",
            ),
        );
        let adapter = CodexAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidate = adapter.discover(&source).expect("discover").remove(0);
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        let parsed = adapter.parse(&candidate, &snapshot).expect("parse");
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].text, "plain string user");
        assert_eq!(parsed.messages[1].text, "plain string assistant");
    }

    #[test]
    fn stage_errors_are_typed() {
        let missing = CodexAdapter::new("/tmp/distill-codex-missing-root-does-not-exist");
        assert!(matches!(missing.detect(), Err(SourceStageError::Detect(_))));

        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("codex-home");
        fs::create_dir_all(&root).expect("root");
        let adapter = CodexAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        assert!(adapter.discover(&source).expect("discover").is_empty());

        let bad_candidate = CaptureCandidate {
            source_kind: SourceKind::Codex,
            source_path: "codex://live/missing.jsonl".into(),
            absolute_path: Some(root.join("missing.jsonl")),
            external_session_id: None,
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: CODEX_MEDIA_TYPE.into(),
        };
        assert!(matches!(
            adapter.snapshot(&bad_candidate),
            Err(SourceStageError::Snapshot(_))
        ));

        let path = write_session(&root, "sessions/bad.jsonl", "{not-json\n");
        let candidate = CaptureCandidate {
            source_kind: SourceKind::Codex,
            source_path: format!("codex://live/{}", path.display()),
            absolute_path: Some(path),
            external_session_id: None,
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: CODEX_MEDIA_TYPE.into(),
        };
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        assert!(matches!(
            adapter.parse(&candidate, &snapshot),
            Err(SourceStageError::Parse(_))
        ));
    }

    #[test]
    fn auxiliary_index_and_history_are_not_discovered() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("codex-home");
        fs::create_dir_all(root.join("sessions")).expect("sessions");
        fs::write(root.join("session_index.jsonl"), "{}\n").expect("index");
        fs::write(root.join("history.jsonl"), "{}\n").expect("history");
        let adapter = CodexAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        assert!(adapter.discover(&source).expect("discover").is_empty());
    }

    #[test]
    fn discover_deduplicates_by_session_meta_id_without_rollout_filename() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("codex-home");
        write_session(
            &root,
            "archived_sessions/archive-copy.jsonl",
            concat!(
                r#"{"type":"session_meta","payload":{"id":"content-session"}}"#,
                "\n",
            ),
        );
        write_session(
            &root,
            "sessions/live-copy.jsonl",
            concat!(
                r#"{"type":"session_meta","payload":{"id":"content-session"}}"#,
                "\n",
            ),
        );

        let adapter = CodexAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidates = adapter.discover(&source).expect("discover");
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0]
            .source_path
            .contains("codex://live/sessions/live-copy.jsonl"));
    }
}
