//! Claude Code SourceAdapter: detect/discover/snapshot/parse for a configured Claude home.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::{
    CaptureCandidate, CaptureSnapshot, DiscoveredSource, ParsedArtifact, ParsedCapture, ParsedFact,
    ParsedMessage, ParserIdentity, SourceAdapter, SourceKind, SourceStageError,
};

/// Default Claude Code parser identity for Normalization Attempts.
pub const CLAUDE_PARSER_ID: &str = "claude_code";

/// Default Claude Code parser contract version.
pub const CLAUDE_PARSER_VERSION: &str = "1.0.0";

/// Media type for Claude Code session JSONL Captures.
pub const CLAUDE_MEDIA_TYPE: &str = "application/x-distill-claude-code+jsonl";

/// Project session root under a Claude home.
const PROJECTS_DIR: &str = "projects";

/// Auxiliary history file under a Claude home (not a Capture).
const HISTORY_FILE: &str = "history.jsonl";

/// Auxiliary settings file under a Claude home (not a Capture).
const SETTINGS_FILE: &str = "settings.json";

/// Claude Code SourceAdapter bound to one configured Claude home root.
pub struct ClaudeAdapter {
    root: PathBuf,
    parser: ParserIdentity,
}

impl ClaudeAdapter {
    /// Create an adapter that detects only the supplied Claude home root.
    #[allow(dead_code)]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_parser(
            root,
            ParserIdentity {
                id: CLAUDE_PARSER_ID.to_string(),
                version: CLAUDE_PARSER_VERSION.to_string(),
            },
        )
    }

    /**
     * Create an adapter with an explicit Claude Code parser identity.
     *
     * Parameters:
     * - `root`: Configured Claude home containing `projects/` session JSONL files.
     * - `parser`: Parser identity/version recorded on Normalization Attempts.
     */
    pub fn with_parser(root: impl Into<PathBuf>, parser: ParserIdentity) -> Self {
        Self {
            root: root.into(),
            parser,
        }
    }
}

impl SourceAdapter for ClaudeAdapter {
    fn detect(&self) -> Result<DiscoveredSource, SourceStageError> {
        let root = canonicalize_existing(&self.root).map_err(SourceStageError::Detect)?;
        if !root.is_dir() {
            return Err(SourceStageError::Detect(format!(
                "claude root is not a directory: {}",
                root.display()
            )));
        }
        Ok(DiscoveredSource {
            kind: SourceKind::ClaudeCode,
            display_name: "Claude Code".to_string(),
            data_root: root,
            parser: self.parser.clone(),
        })
    }

    fn discover(
        &self,
        source: &DiscoveredSource,
    ) -> Result<Vec<CaptureCandidate>, SourceStageError> {
        let projects_root = source.data_root.join(PROJECTS_DIR);
        let mut discovered =
            discover_project_jsonl(&projects_root).map_err(SourceStageError::Discover)?;
        discovered.sort_by(|left, right| left.source_path.cmp(&right.source_path));

        // Duplicate Session Identities resolve by sorted source_path order (first wins).
        let mut by_session: HashMap<String, CaptureCandidate> = HashMap::new();
        let mut without_session = Vec::new();
        for candidate in discovered {
            match candidate.external_session_id.clone() {
                Some(session_id) => {
                    by_session.entry(session_id).or_insert(candidate);
                }
                None => without_session.push(candidate),
            }
        }

        let mut resolved: Vec<CaptureCandidate> = by_session.into_values().collect();
        resolved.extend(without_session);
        resolved.sort_by(|left, right| left.source_path.cmp(&right.source_path));
        Ok(resolved)
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
        parse_claude_jsonl(candidate, &snapshot.bytes, Some(self.root.as_path()))
    }
}

/**
 * Parse Distill-owned Claude Code Capture bytes without rereading the Claude home.
 *
 * Parameters:
 * - `candidate`: Replay Candidate rebuilt from persisted Capture identity.
 * - `bytes`: Checksum-verified Distill-owned Capture bytes.
 */
pub(crate) fn parse_claude_bytes(
    candidate: &CaptureCandidate,
    bytes: &[u8],
) -> Result<ParsedCapture, SourceStageError> {
    parse_claude_jsonl(candidate, bytes, None)
}

/**
 * Parse Claude Code session JSONL bytes into Capture Facts, Messages, and Artifacts.
 *
 * Parameters:
 * - `candidate`: Capture Candidate providing identity and path hints.
 * - `bytes`: Exact snapshot bytes preserved by Distill.
 * - `claude_root`: Optional Claude home for auxiliary history metadata. Replay
 *   passes `None` so renormalization never rereads the original root.
 */
fn parse_claude_jsonl(
    candidate: &CaptureCandidate,
    bytes: &[u8],
    claude_root: Option<&Path>,
) -> Result<ParsedCapture, SourceStageError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|err| SourceStageError::Parse(format!("claude bytes are not utf-8: {err}")))?;

    let history_index = claude_root.map(read_history_index).unwrap_or_default();
    let mut facts = Vec::new();
    let mut messages = Vec::new();
    let mut artifacts = Vec::new();

    let mut row_session_id: Option<String> = None;
    let mut started_at = None;
    let mut updated_at = None;
    let mut project_path = None;
    let mut git_branch = None;

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
        let uuid = value
            .get("uuid")
            .and_then(Value::as_str)
            .map(str::to_string);
        let record_session_id = value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
        let branch = value
            .get("gitBranch")
            .and_then(Value::as_str)
            .map(str::to_string);
        let is_meta = value
            .get("isMeta")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let message = value.get("message").and_then(Value::as_object);
        let role = message
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let blocks = normalize_content_blocks(message.and_then(|message| message.get("content")));
        let content_text = extract_text_blocks(&blocks);

        if let Some(session_id) = record_session_id {
            row_session_id = Some(session_id);
        }
        if let Some(cwd) = cwd {
            project_path = Some(cwd);
        }
        if let Some(branch) = branch {
            git_branch = Some(branch);
        }

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

        let project_message = matches!(record_type, "user" | "assistant")
            && !is_meta
            && matches!(role.as_deref(), Some("user") | Some("assistant"))
            && !content_text.is_empty()
            && !is_suppressed_claude_text(&content_text);

        let fact_ordinal = facts.len();
        facts.push(ParsedFact {
            record_type: record_type.to_string(),
            role: role.clone(),
            is_meta: is_meta || !project_message,
            content_text: (!content_text.is_empty()).then_some(content_text.clone()),
            content_json: value.clone(),
        });

        let message_ordinal = if project_message {
            let ordinal = messages.len();
            messages.push(ParsedMessage {
                role: role.clone().unwrap_or_else(|| "user".to_string()),
                message_kind: "text".to_string(),
                text: content_text,
                external_message_id: uuid.clone(),
            });
            Some(ordinal)
        } else {
            None
        };

        for block in blocks {
            let Some(artifact_type) = artifact_type_for_block(&block) else {
                continue;
            };
            artifacts.push(ParsedArtifact {
                artifact_type,
                message_ordinal,
                fact_ordinal: Some(fact_ordinal),
                text_preview: block_text_preview(&block),
                content_json: Value::Object(block),
            });
        }
    }

    let stem_identity = candidate
        .external_session_id
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            candidate
                .absolute_path
                .as_ref()
                .and_then(|path| filename_stem(path))
        });

    let (resolved_external_session_id, synthetic_identity) =
        if let Some(session_id) = row_session_id.filter(|value| !value.is_empty()) {
            (session_id, false)
        } else if let Some(stem) = stem_identity {
            (stem, false)
        } else {
            (synthetic_session_id(candidate), true)
        };

    let title = pick_claude_title(&resolved_external_session_id, &history_index, &messages);

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
    if let Some(git_branch) = git_branch {
        metadata.insert("git_branch".to_string(), json!(git_branch));
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

/**
 * Recursively discover project session `.jsonl` files under `projects/`.
 *
 * Parameters:
 * - `projects_root`: Absolute `projects/` directory under the Claude home.
 */
fn discover_project_jsonl(projects_root: &Path) -> Result<Vec<CaptureCandidate>, String> {
    if !projects_root.exists() {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::new();
    let mut file_paths = visit_files(projects_root)?;
    file_paths.sort();
    for file_path in file_paths {
        if file_path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let file_name = file_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if file_name == HISTORY_FILE || file_name == SETTINGS_FILE {
            continue;
        }
        let relative = file_path
            .strip_prefix(projects_root)
            .unwrap_or(file_path.as_path())
            .display()
            .to_string()
            .replace('\\', "/");
        candidates.push(CaptureCandidate {
            source_kind: SourceKind::ClaudeCode,
            source_path: format!("claude://project/{relative}"),
            absolute_path: Some(file_path.clone()),
            external_session_id: peek_session_id(&file_path).or_else(|| filename_stem(&file_path)),
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: CLAUDE_MEDIA_TYPE.to_string(),
        });
    }
    Ok(candidates)
}

/**
 * Peek a non-empty `sessionId` from early JSONL rows without reading the full capture.
 *
 * Discovery only peeks identity metadata; exact bytes are still snapshotted later.
 */
fn peek_session_id(path: &Path) -> Option<String> {
    const ID_SCAN_LIMIT: u64 = 1024 * 1024;
    let file = fs::File::open(path).ok()?;
    for line in BufReader::new(file).take(ID_SCAN_LIMIT).lines() {
        let Ok(line) = line else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some(session_id) = value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(session_id.to_string());
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
 * Read auxiliary `history.jsonl` when present. Missing or invalid rows are ignored.
 */
fn read_history_index(claude_root: &Path) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let path = claude_root.join(HISTORY_FILE);
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
        let Some(session_id) = row
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(display) = row
            .get("display")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        map.entry(session_id.to_string())
            .or_insert_with(|| display.to_string());
    }
    map
}

/**
 * Normalize Claude message content into structured block objects.
 */
fn normalize_content_blocks(content: Option<&Value>) -> Vec<Map<String, Value>> {
    match content {
        Some(Value::String(text)) => {
            let mut block = Map::new();
            block.insert("type".to_string(), json!("text"));
            block.insert("text".to_string(), json!(text));
            vec![block]
        }
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_object().cloned())
            .collect(),
        _ => Vec::new(),
    }
}

/**
 * Extract visible text from Claude text blocks only.
 */
fn extract_text_blocks(blocks: &[Map<String, Value>]) -> String {
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

/**
 * Map a structured Claude content block to a canonical Artifact type.
 *
 * Text blocks are transcript candidates, not Artifacts. Thinking/reasoning and other
 * structured blocks are preserved as Artifacts and never become visible transcript.
 */
fn artifact_type_for_block(block: &Map<String, Value>) -> Option<String> {
    let block_type = block.get("type").and_then(Value::as_str)?;
    match block_type {
        "text" => None,
        "image" => Some("image".to_string()),
        "tool_use" => Some("tool_call".to_string()),
        "tool_result" => Some("tool_result".to_string()),
        "file" => Some("file".to_string()),
        other => Some(other.to_string()),
    }
}

/**
 * Optional short preview for a structured block Artifact.
 */
fn block_text_preview(block: &Map<String, Value>) -> Option<String> {
    block
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| block.get("thinking").and_then(Value::as_str))
        .or_else(|| block.get("name").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(160).collect())
}

/**
 * Suppress local-command and image-placeholder noise from projected transcript Messages.
 */
fn is_suppressed_claude_text(text: &str) -> bool {
    text.starts_with("<local-command-caveat>")
        || text.starts_with("<command-name>")
        || text.starts_with("<local-command-stdout>")
        || text.starts_with("[Image: original ")
}

/**
 * Prefer history display titles, else the first user message line (truncated).
 */
fn pick_claude_title(
    session_id: &str,
    history_index: &BTreeMap<String, String>,
    messages: &[ParsedMessage],
) -> Option<String> {
    if let Some(title) = history_index
        .get(session_id)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| !is_suppressed_claude_text(value))
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
 * Non-empty filename stem used as Session Identity fallback.
 */
fn filename_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/**
 * Canonicalize an existing path, mapping IO failures into strings.
 */
fn canonicalize_existing(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|err| format!("{}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /**
     * Write a Claude project session JSONL under `projects/<relative>`.
     */
    fn write_session(root: &Path, relative: &str, body: &str) -> PathBuf {
        let path = root.join(PROJECTS_DIR).join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("session parent");
        }
        fs::write(&path, body).expect("write session");
        path
    }

    fn mixed_body() -> String {
        concat!(
            r#"{"type":"user","uuid":"u1","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-03-25T11:00:00.000Z","cwd":"/tmp/demo-project","message":{"role":"user","content":[{"type":"text","text":"Please review the screenshot and fix the layout."},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB"}}]}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a1","parentUuid":"u1","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-03-25T11:00:02.000Z","cwd":"/tmp/demo-project","gitBranch":"feature/layout","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"I will tighten the layout."},{"type":"tool_use","name":"Read","input":{"file_path":"/tmp/demo-project/src/app.ts"}},{"type":"tool_result","content":"ok"},{"type":"file","file":{"path":"/tmp/demo-project/src/app.ts"}},{"type":"custom_block","payload":{"x":1}}]}}"#,
            "\n",
            r#"{"type":"user","uuid":"u2","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-03-25T11:00:03.000Z","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"meta row stays fact"}]}}"#,
            "\n",
            r#"{"type":"user","uuid":"u3","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-03-25T11:00:04.000Z","message":{"role":"system","content":[{"type":"text","text":"unknown role stays fact"}]}}"#,
            "\n",
            r#"{"type":"queue-operation","uuid":"q1","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-03-25T11:00:05.000Z","operation":"enqueue"}"#,
            "\n",
            r#"{"type":"progress","uuid":"p1","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-03-25T11:00:06.000Z","progress":0.5}"#,
            "\n",
        )
        .to_string()
    }

    #[test]
    fn detect_reports_configured_claude_home() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("claude-home");
        fs::create_dir_all(root.join(PROJECTS_DIR)).expect("projects");
        let adapter = ClaudeAdapter::new(&root);
        let discovered = adapter.detect().expect("detect");
        assert_eq!(discovered.kind, SourceKind::ClaudeCode);
        assert_eq!(discovered.display_name, "Claude Code");
        assert_eq!(discovered.parser.id, CLAUDE_PARSER_ID);
        assert!(discovered.data_root.ends_with("claude-home"));
    }

    #[test]
    fn discover_project_sessions_and_excludes_auxiliary() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("claude-home");
        write_session(
            &root,
            "demo-project/123e4567-e89b-12d3-a456-426614174000.jsonl",
            &mixed_body(),
        );
        fs::write(root.join(HISTORY_FILE), "{}\n").expect("history");
        fs::write(root.join(SETTINGS_FILE), "{}\n").expect("settings");
        fs::write(
            root.join(PROJECTS_DIR).join(HISTORY_FILE),
            r#"{"sessionId":"should-not-discover"}
"#,
        )
        .expect("nested history");

        let adapter = ClaudeAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidates = adapter.discover(&source).expect("discover");
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].source_path,
            "claude://project/demo-project/123e4567-e89b-12d3-a456-426614174000.jsonl"
        );
        assert_eq!(
            candidates[0].external_session_id.as_deref(),
            Some("123e4567-e89b-12d3-a456-426614174000")
        );
    }

    #[test]
    fn discover_dedupes_duplicate_identities_by_source_path_order() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("claude-home");
        write_session(
            &root,
            "z-project/same-session.jsonl",
            concat!(
                r#"{"type":"user","sessionId":"dup-session","message":{"role":"user","content":[{"type":"text","text":"later path"}]}}"#,
                "\n",
            ),
        );
        write_session(
            &root,
            "a-project/same-session.jsonl",
            concat!(
                r#"{"type":"user","sessionId":"dup-session","message":{"role":"user","content":[{"type":"text","text":"earlier path"}]}}"#,
                "\n",
            ),
        );

        let adapter = ClaudeAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidates = adapter.discover(&source).expect("discover");
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].source_path,
            "claude://project/a-project/same-session.jsonl"
        );
    }

    #[test]
    fn snapshot_preserves_exact_bytes_and_sha256() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("claude-home");
        let body = mixed_body();
        let path = write_session(
            &root,
            "demo-project/123e4567-e89b-12d3-a456-426614174000.jsonl",
            &body,
        );
        let adapter = ClaudeAdapter::new(&root);
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
    fn parse_mixed_blocks_unknown_role_meta_and_history_title() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("claude-home");
        write_session(
            &root,
            "demo-project/123e4567-e89b-12d3-a456-426614174000.jsonl",
            &mixed_body(),
        );
        fs::write(
            root.join(HISTORY_FILE),
            r#"{"display":"Claude mixed content fixture","sessionId":"123e4567-e89b-12d3-a456-426614174000"}
"#,
        )
        .expect("history");

        let adapter = ClaudeAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidate = adapter.discover(&source).expect("discover").remove(0);
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        let parsed = adapter.parse(&candidate, &snapshot).expect("parse");

        assert_eq!(
            parsed.external_session_id,
            "123e4567-e89b-12d3-a456-426614174000"
        );
        assert!(!parsed.synthetic_identity);
        assert_eq!(
            parsed.title.as_deref(),
            Some("Claude mixed content fixture")
        );
        assert_eq!(parsed.project_path.as_deref(), Some("/tmp/demo-project"));
        assert_eq!(
            parsed.metadata.get("git_branch").and_then(Value::as_str),
            Some("feature/layout")
        );
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(
            parsed.messages[0].text,
            "Please review the screenshot and fix the layout."
        );
        assert_eq!(parsed.messages[1].text, "I will tighten the layout.");
        assert!(!parsed
            .messages
            .iter()
            .any(|message| message.text.contains("meta row")
                || message.text.contains("unknown role")
                || message.text.contains("hidden")));

        let artifact_types: Vec<_> = parsed
            .artifacts
            .iter()
            .map(|artifact| artifact.artifact_type.as_str())
            .collect();
        assert!(artifact_types.contains(&"image"));
        assert!(artifact_types.contains(&"thinking"));
        assert!(artifact_types.contains(&"tool_call"));
        assert!(artifact_types.contains(&"tool_result"));
        assert!(artifact_types.contains(&"file"));
        assert!(artifact_types.contains(&"custom_block"));

        assert!(parsed
            .facts
            .iter()
            .any(|fact| fact.record_type == "queue-operation"));
        assert!(parsed
            .facts
            .iter()
            .any(|fact| fact.record_type == "progress"));
        assert!(parsed
            .facts
            .iter()
            .any(|fact| fact.role.as_deref() == Some("system")));
        assert!(parsed
            .facts
            .iter()
            .all(|fact| fact.content_json.is_object()));
    }

    #[test]
    fn parse_prefers_row_session_id_then_stem_then_synthetic() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("claude-home");

        write_session(
            &root,
            "demo/stem-only.jsonl",
            concat!(
                r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"stem identity"}]}}"#,
                "\n",
            ),
        );
        write_session(
            &root,
            "demo/no-identity.jsonl",
            concat!(
                r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"synthetic"}]}}"#,
                "\n",
            ),
        );
        // Force no stem by using a path without a usable stem after discovery override.
        let adapter = ClaudeAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let mut candidates = adapter.discover(&source).expect("discover");
        candidates.sort_by(|left, right| left.source_path.cmp(&right.source_path));

        let stem_candidate = candidates
            .iter()
            .find(|candidate| candidate.source_path.ends_with("stem-only.jsonl"))
            .cloned()
            .expect("stem candidate");
        let stem_snapshot = adapter.snapshot(&stem_candidate).expect("snapshot");
        let stem_parsed = adapter
            .parse(&stem_candidate, &stem_snapshot)
            .expect("parse");
        assert_eq!(stem_parsed.external_session_id, "stem-only");
        assert!(!stem_parsed.synthetic_identity);

        let mut synthetic_candidate = candidates
            .into_iter()
            .find(|candidate| candidate.source_path.ends_with("no-identity.jsonl"))
            .expect("synthetic candidate");
        // Clear discovery identity and path stem so parse must synthesize.
        synthetic_candidate.external_session_id = None;
        synthetic_candidate.absolute_path = None;
        let body = concat!(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"synthetic"}]}}"#,
            "\n",
        );
        let synthetic_snapshot = CaptureSnapshot {
            bytes: body.as_bytes().to_vec(),
            sha256: hex::encode(Sha256::digest(body.as_bytes())),
            byte_size: body.len() as u64,
            media_type: CLAUDE_MEDIA_TYPE.into(),
            source_modified_at: None,
        };
        let synthetic_parsed = adapter
            .parse(&synthetic_candidate, &synthetic_snapshot)
            .expect("parse synthetic");
        assert!(synthetic_parsed.synthetic_identity);
        assert!(synthetic_parsed
            .external_session_id
            .starts_with("synthetic-"));
        let again = adapter
            .parse(&synthetic_candidate, &synthetic_snapshot)
            .expect("parse again");
        assert_eq!(
            synthetic_parsed.external_session_id,
            again.external_session_id
        );
    }

    #[test]
    fn parse_suppresses_local_command_noise() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("claude-home");
        write_session(
            &root,
            "demo/noise.jsonl",
            concat!(
                r#"{"type":"user","sessionId":"noise-session","message":{"role":"user","content":[{"type":"text","text":"<local-command-caveat>hidden"}]}}"#,
                "\n",
                r#"{"type":"user","sessionId":"noise-session","message":{"role":"user","content":[{"type":"text","text":"[Image: original 100x100]"}]}}"#,
                "\n",
                r#"{"type":"user","sessionId":"noise-session","message":{"role":"user","content":[{"type":"text","text":"visible after noise"}]}}"#,
                "\n",
            ),
        );
        let adapter = ClaudeAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidate = adapter.discover(&source).expect("discover").remove(0);
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        let parsed = adapter.parse(&candidate, &snapshot).expect("parse");
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].text, "visible after noise");
    }

    #[test]
    fn stage_errors_are_typed() {
        let missing = ClaudeAdapter::new("/tmp/distill-claude-missing-root-does-not-exist");
        assert!(matches!(missing.detect(), Err(SourceStageError::Detect(_))));

        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("claude-home");
        fs::create_dir_all(&root).expect("root");
        let adapter = ClaudeAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        assert!(adapter.discover(&source).expect("discover").is_empty());

        let bad_candidate = CaptureCandidate {
            source_kind: SourceKind::ClaudeCode,
            source_path: "claude://project/missing.jsonl".into(),
            absolute_path: Some(root.join("missing.jsonl")),
            external_session_id: None,
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: CLAUDE_MEDIA_TYPE.into(),
        };
        assert!(matches!(
            adapter.snapshot(&bad_candidate),
            Err(SourceStageError::Snapshot(_))
        ));

        let path = write_session(&root, "demo/bad.jsonl", "{not-json\n");
        let candidate = CaptureCandidate {
            source_kind: SourceKind::ClaudeCode,
            source_path: "claude://project/demo/bad.jsonl".into(),
            absolute_path: Some(path),
            external_session_id: None,
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: CLAUDE_MEDIA_TYPE.into(),
        };
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        assert!(matches!(
            adapter.parse(&candidate, &snapshot),
            Err(SourceStageError::Parse(_))
        ));
    }

    #[test]
    fn unreadable_projects_root_is_discover_error() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("claude-home");
        fs::create_dir_all(&root).expect("root");
        let projects = root.join(PROJECTS_DIR);
        fs::write(&projects, "not-a-directory").expect("file where projects should be");

        let adapter = ClaudeAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        assert!(matches!(
            adapter.discover(&source),
            Err(SourceStageError::Discover(_))
        ));
    }
}
