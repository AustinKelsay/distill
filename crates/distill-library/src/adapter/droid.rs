//! Factory Droid SourceAdapter: file-backed detect/discover/snapshot/parse.
//!
//! Sessions live under `~/.factory/sessions/<workspace-slug>/<session-id>.jsonl`
//! with optional `<session-id>.settings.json` sidecars. No provider subprocess or
//! SQLite access belongs in this adapter.

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::{
    CaptureCandidate, CaptureSnapshot, DiscoveredSource, ParsedArtifact, ParsedCapture, ParsedFact,
    ParsedMessage, ParserIdentity, SourceAdapter, SourceKind, SourceStageError,
};

/// Default Droid parser identity for Normalization Attempts.
pub const DROID_PARSER_ID: &str = "droid";

/// Default Droid parser contract version.
pub const DROID_PARSER_VERSION: &str = "1.0.0";

/// Media type for Factory Droid session JSONL Captures.
pub const DROID_MEDIA_TYPE: &str = "application/x-distill-droid+jsonl";

/// Sidecar suffix paired with a session JSONL file.
const SETTINGS_SUFFIX: &str = ".settings.json";

/**
 * Resolve the default Factory Droid sessions root (`$HOME/.factory/sessions`).
 *
 * Returns `None` when `HOME` is unset. Callers treat a missing path as an absent
 * root rather than a configured-root validation failure.
 */
pub fn default_droid_sessions_root() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".factory").join("sessions"))
}

/// Factory Droid SourceAdapter bound to one sessions root.
pub struct DroidAdapter {
    root: PathBuf,
    parser: ParserIdentity,
}

impl DroidAdapter {
    /// Create an adapter that detects only the supplied Droid sessions root.
    #[allow(dead_code)]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_parser(
            root,
            ParserIdentity {
                id: DROID_PARSER_ID.to_string(),
                version: DROID_PARSER_VERSION.to_string(),
            },
        )
    }

    /**
     * Create an adapter with an explicit Droid parser identity.
     *
     * Parameters:
     * - `root`: Sessions root containing workspace-slug directories of JSONL files.
     * - `parser`: Parser identity/version recorded on Normalization Attempts.
     */
    pub fn with_parser(root: impl Into<PathBuf>, parser: ParserIdentity) -> Self {
        Self {
            root: root.into(),
            parser,
        }
    }
}

impl SourceAdapter for DroidAdapter {
    fn detect(&self) -> Result<DiscoveredSource, SourceStageError> {
        let root = canonicalize_existing(&self.root).map_err(SourceStageError::Detect)?;
        if !root.is_dir() {
            return Err(SourceStageError::Detect(
                "droid root is not a directory".into(),
            ));
        }
        Ok(DiscoveredSource {
            kind: SourceKind::Droid,
            display_name: "Droid".to_string(),
            data_root: root,
            parser: self.parser.clone(),
        })
    }

    fn discover(
        &self,
        source: &DiscoveredSource,
    ) -> Result<Vec<CaptureCandidate>, SourceStageError> {
        let mut discovered =
            discover_session_jsonl(&source.data_root).map_err(SourceStageError::Discover)?;
        discovered.sort_by(|left, right| {
            left.absolute_path
                .cmp(&right.absolute_path)
                .then_with(|| left.source_path.cmp(&right.source_path))
        });

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
        parse_droid_jsonl(candidate, &snapshot.bytes)
    }
}

/**
 * Parse Distill-owned Droid Capture bytes without rereading session files or sidecars.
 *
 * Parameters:
 * - `candidate`: Replay Candidate rebuilt from persisted Capture identity.
 * - `bytes`: Checksum-verified Distill-owned Capture bytes.
 */
pub(crate) fn parse_droid_bytes(
    candidate: &CaptureCandidate,
    bytes: &[u8],
) -> Result<ParsedCapture, SourceStageError> {
    parse_droid_jsonl(candidate, bytes)
}

/**
 * Recursively discover session `.jsonl` files, excluding sidecar settings files.
 *
 * Parameters:
 * - `sessions_root`: Absolute Factory sessions directory.
 */
fn discover_session_jsonl(sessions_root: &Path) -> Result<Vec<CaptureCandidate>, String> {
    if !sessions_root.exists() {
        return Ok(Vec::new());
    }
    if !sessions_root.is_dir() {
        return Err("droid sessions root is not a directory".into());
    }

    let mut candidates = Vec::new();
    let mut file_paths = visit_files(sessions_root)?;
    file_paths.sort();
    for file_path in file_paths {
        let file_name = file_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if file_name.ends_with(SETTINGS_SUFFIX) {
            continue;
        }
        if file_path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }

        let peeked = peek_session_start(&file_path);
        let stem = filename_stem(&file_path);
        let session_id = peeked
            .as_ref()
            .and_then(|start| start.id.clone())
            .or_else(|| stem.clone())
            .filter(|value| !value.is_empty());
        let title = peeked.as_ref().and_then(|start| start.title.clone());
        let sidecar = read_sidecar_metadata(&file_path);
        let identity = session_id
            .clone()
            .unwrap_or_else(|| synthetic_session_id_for_path(&file_path, sessions_root));
        candidates.push(CaptureCandidate {
            source_kind: SourceKind::Droid,
            source_path: format!("droid://session/{identity}"),
            absolute_path: Some(file_path.clone()),
            external_session_id: session_id,
            title: title
                .or_else(|| sidecar.title.clone())
                .filter(|value| !value.is_empty()),
            is_virtual: false,
            virtual_bytes: None,
            media_type: DROID_MEDIA_TYPE.to_string(),
        });
    }
    Ok(candidates)
}

/**
 * Parse Factory Droid session JSONL bytes into Capture Facts, Messages, and Artifacts.
 */
fn parse_droid_jsonl(
    candidate: &CaptureCandidate,
    bytes: &[u8],
) -> Result<ParsedCapture, SourceStageError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|err| SourceStageError::Parse(format!("droid bytes are not utf-8: {err}")))?;

    let sidecar = candidate
        .absolute_path
        .as_ref()
        .map(|path| read_sidecar_metadata(path))
        .unwrap_or_default();

    let mut facts = Vec::new();
    let mut messages = Vec::new();
    let mut artifacts = Vec::new();

    let mut session_id: Option<String> = None;
    let mut title: Option<String> = None;
    let mut owner: Option<String> = None;
    let mut project_path: Option<String> = None;
    let mut started_at: Option<String> = None;
    let mut updated_at: Option<String> = None;

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
        let timestamp = normalize_timestamp(value.get("timestamp"));
        let record_id = value
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        if record_type == "session_start" {
            if let Some(id) = record_id.clone() {
                session_id = Some(id);
            }
            title = text_field(&value, "title")
                .or_else(|| text_field(&value, "sessionTitle"))
                .or(title);
            owner = text_field(&value, "owner").or(owner);
            project_path = text_field(&value, "cwd").or(project_path);

            facts.push(ParsedFact {
                record_type: record_type.to_string(),
                role: None,
                is_meta: true,
                content_text: title.clone(),
                content_json: value,
            });
            continue;
        }

        if let Some(ts) = timestamp.clone() {
            if started_at
                .as_deref()
                .is_none_or(|current| ts.as_str() < current)
            {
                started_at = Some(ts.clone());
            }
            if updated_at
                .as_deref()
                .is_none_or(|current| ts.as_str() > current)
            {
                updated_at = Some(ts);
            }
        }

        let message = value.get("message").and_then(Value::as_object);
        let role = message
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let blocks = normalize_content_blocks(message.and_then(|message| message.get("content")));
        let content_text = extract_text_blocks(&blocks);

        let project_message = record_type == "message"
            && matches!(role.as_deref(), Some("user") | Some("assistant"))
            && !content_text.is_empty();

        let fact_ordinal = facts.len();
        facts.push(ParsedFact {
            record_type: record_type.to_string(),
            role: role.clone(),
            is_meta: !project_message,
            content_text: (!content_text.is_empty()).then_some(content_text.clone()),
            content_json: value.clone(),
        });

        let message_ordinal = if project_message {
            let ordinal = messages.len();
            messages.push(ParsedMessage {
                role: role.clone().unwrap_or_else(|| "user".to_string()),
                message_kind: "text".to_string(),
                text: content_text,
                external_message_id: record_id.clone(),
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
        if let Some(id) = session_id.filter(|value| !value.is_empty()) {
            (id, false)
        } else if let Some(stem) = stem_identity {
            (stem, false)
        } else {
            (synthetic_session_id(candidate), true)
        };

    let resolved_title = title
        .or_else(|| candidate.title.clone())
        .or_else(|| sidecar.title.clone())
        .or_else(|| {
            messages
                .iter()
                .find(|message| message.role == "user")
                .and_then(|message| message.text.lines().next())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.chars().take(160).collect())
        });

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
    if let Some(owner) = owner {
        metadata.insert("owner".to_string(), json!(owner));
    }
    if let Some(model) = sidecar.model {
        metadata.insert("model".to_string(), json!(model));
    }
    if let Some(archived_at) = sidecar.archived_at {
        metadata.insert("archived_at".to_string(), json!(archived_at));
        metadata.insert("archived".to_string(), json!(true));
    }

    Ok(ParsedCapture {
        external_session_id: resolved_external_session_id,
        synthetic_identity,
        title: resolved_title,
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

#[derive(Clone, Debug, Default)]
struct SessionStartPeek {
    id: Option<String>,
    title: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct SidecarMetadata {
    title: Option<String>,
    model: Option<String>,
    archived_at: Option<String>,
}

/**
 * Peek `session_start` identity/title from early JSONL rows without reading the full capture.
 */
fn peek_session_start(path: &Path) -> Option<SessionStartPeek> {
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
        if value.get("type").and_then(Value::as_str) != Some("session_start") {
            continue;
        }
        return Some(SessionStartPeek {
            id: text_field(&value, "id"),
            title: text_field(&value, "title").or_else(|| text_field(&value, "sessionTitle")),
        });
    }
    None
}

/**
 * Read optional sidecar settings next to a session JSONL without surfacing absolute paths.
 */
fn read_sidecar_metadata(session_path: &Path) -> SidecarMetadata {
    let Some(stem) = filename_stem(session_path) else {
        return SidecarMetadata::default();
    };
    let Some(parent) = session_path.parent() else {
        return SidecarMetadata::default();
    };
    let path = parent.join(format!("{stem}{SETTINGS_SUFFIX}"));
    let Ok(text) = fs::read_to_string(&path) else {
        return SidecarMetadata::default();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return SidecarMetadata::default();
    };
    SidecarMetadata {
        title: text_field(&value, "title").or_else(|| text_field(&value, "sessionTitle")),
        model: text_field(&value, "model"),
        archived_at: text_field(&value, "archivedAt"),
    }
}

/**
 * Normalize Droid message content into structured block objects.
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
            .filter_map(|item| match item {
                Value::Object(block) => Some(block.clone()),
                Value::String(text) => {
                    let mut block = Map::new();
                    block.insert("type".to_string(), json!("text"));
                    block.insert("text".to_string(), json!(text));
                    Some(block)
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/**
 * Extract visible text from Droid text blocks only.
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
 * Map a structured Droid content block to a canonical Artifact type.
 */
fn artifact_type_for_block(block: &Map<String, Value>) -> Option<String> {
    let block_type = block.get("type").and_then(Value::as_str)?;
    match block_type {
        "text" => None,
        "image" => Some("image".to_string()),
        "tool_use" => Some("tool_call".to_string()),
        "tool_result" => Some("tool_result".to_string()),
        "thinking" => Some("thinking".to_string()),
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
 * Accept only valid RFC3339 timestamps; invalid values are ignored.
 */
fn normalize_timestamp(value: Option<&Value>) -> Option<String> {
    let raw = value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc).to_rfc3339())
}

/**
 * Non-empty trimmed string field helper.
 */
fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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
    let canonical = fs::canonicalize(root).map_err(|_| "droid path is unreadable".to_string())?;
    if !visited.insert(canonical) {
        return Ok(files);
    }
    let entries = fs::read_dir(root).map_err(|_| "droid path is unreadable".to_string())?;
    for entry in entries {
        let entry = entry.map_err(|_| "droid path is unreadable".to_string())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "droid path is unreadable".to_string())?;
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
 * Derive a deterministic synthetic Session Identity from the candidate path.
 */
fn synthetic_session_id(candidate: &CaptureCandidate) -> String {
    let digest = Sha256::digest(candidate.source_path.as_bytes());
    format!("synthetic-{}", &hex::encode(digest)[..16])
}

/**
 * Deterministic synthetic identity for discovery when session_start and stem are absent.
 */
fn synthetic_session_id_for_path(path: &Path, sessions_root: &Path) -> String {
    let relative = path
        .strip_prefix(sessions_root)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/");
    let digest = Sha256::digest(relative.as_bytes());
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
 * Canonicalize an existing path, mapping IO failures into redacted strings.
 */
fn canonicalize_existing(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|_| "droid root is unreadable".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /**
     * Write a workspace session JSONL and optional sidecar under a Droid root.
     */
    fn write_session(root: &Path, workspace: &str, session_id: &str, body: &str) -> PathBuf {
        let path = root.join(workspace).join(format!("{session_id}.jsonl"));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("session parent");
        }
        fs::write(&path, body).expect("write session");
        path
    }

    fn mixed_body(session_id: &str) -> String {
        [
            format!(
                r#"{{"type":"session_start","id":"{session_id}","title":"Droid mixed fixture","owner":"plebdev","cwd":"/tmp/droid-demo"}}"#
            ),
            r#"{"type":"message","id":"m1","timestamp":"2026-04-12T18:17:28.000Z","message":{"role":"user","content":["Plain array text",{"type":"text","text":"Please review the screenshot."},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"abc"}}]}}"#.to_string(),
            r#"{"type":"message","id":"m2","timestamp":"2026-04-12T18:17:29.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"I will tighten the layout."},{"type":"tool_use","id":"t1","name":"Read","input":{"path":"/tmp/app.ts"}},{"type":"tool_result","tool_use_id":"t1","content":"ok"},{"type":"file","file":{"path":"/tmp/app.ts"}},{"type":"custom_block","payload":{"x":1}}]}}"#.to_string(),
            r#"{"type":"message","id":"m3","timestamp":"not-a-timestamp","message":{"role":"system","content":[{"type":"text","text":"unknown role stays fact"}]}}"#.to_string(),
            r#"{"type":"todo_state","id":"todo1","todos":[]}"#.to_string(),
            String::new(),
        ]
        .join("\n")
    }

    #[test]
    fn detect_reports_configured_droid_root() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("sessions");
        fs::create_dir_all(&root).expect("root");
        let adapter = DroidAdapter::new(&root);
        let discovered = adapter.detect().expect("detect");
        assert_eq!(discovered.kind, SourceKind::Droid);
        assert_eq!(discovered.display_name, "Droid");
        assert_eq!(discovered.parser.id, DROID_PARSER_ID);
    }

    #[test]
    fn discover_excludes_settings_and_uses_canonical_identities() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("sessions");
        let session_id = "123e4567-e89b-12d3-a456-426614174000";
        write_session(&root, "ws-a", session_id, &mixed_body(session_id));
        fs::write(
            root.join("ws-a")
                .join(format!("{session_id}{SETTINGS_SUFFIX}")),
            r#"{"model":"gpt-5.4","archivedAt":"2026-04-12T18:20:00.000Z"}"#,
        )
        .expect("sidecar");
        fs::write(root.join("orphan.settings.json"), "{}\n").expect("root settings");

        let adapter = DroidAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidates = adapter.discover(&source).expect("discover");
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].source_path,
            format!("droid://session/{session_id}")
        );
        assert!(!candidates[0].source_path.starts_with('/'));
        assert!(!candidates[0]
            .source_path
            .contains(root.to_string_lossy().as_ref()));
        assert_eq!(
            candidates[0].external_session_id.as_deref(),
            Some(session_id)
        );
        assert_eq!(candidates[0].title.as_deref(), Some("Droid mixed fixture"));
    }

    #[test]
    fn discover_recurses_deduplicates_and_prefers_sorted_path() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("sessions");
        let duplicate_id = "same-session";
        let first = write_session(
            &root,
            "workspace/a/nested",
            duplicate_id,
            &(format!(r#"{{"type":"session_start","id":"{duplicate_id}","title":"first"}}"#)
                + "\n"),
        );
        let second = write_session(
            &root,
            "workspace/b",
            "filename-id",
            &(format!(r#"{{"type":"session_start","id":"{duplicate_id}","title":"second"}}"#)
                + "\n"),
        );
        assert!(first < second);

        let adapter = DroidAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidates = adapter.discover(&source).expect("discover");
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].external_session_id.as_deref(),
            Some(duplicate_id)
        );
        assert_eq!(candidates[0].title.as_deref(), Some("first"));
        assert_eq!(
            candidates[0].absolute_path.as_deref(),
            Some(fs::canonicalize(first).expect("canonical first").as_path())
        );
    }

    #[test]
    fn parse_mixed_blocks_sidecar_and_invalid_timestamp() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("sessions");
        let session_id = "123e4567-e89b-12d3-a456-426614174000";
        let path = write_session(&root, "ws-a", session_id, &mixed_body(session_id));
        fs::write(
            path.with_file_name(format!("{session_id}{SETTINGS_SUFFIX}")),
            r#"{"model":"claude-sonnet-4-6","archivedAt":"2026-04-12T18:20:00.000Z"}"#,
        )
        .expect("sidecar");

        let adapter = DroidAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidate = adapter.discover(&source).expect("discover").remove(0);
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        let parsed = adapter.parse(&candidate, &snapshot).expect("parse");

        assert_eq!(parsed.external_session_id, session_id);
        assert_eq!(parsed.title.as_deref(), Some("Droid mixed fixture"));
        assert_eq!(parsed.project_path.as_deref(), Some("/tmp/droid-demo"));
        assert_eq!(
            parsed.metadata.get("owner").and_then(Value::as_str),
            Some("plebdev")
        );
        assert_eq!(
            parsed.metadata.get("model").and_then(Value::as_str),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(
            parsed.metadata.get("archived").and_then(Value::as_bool),
            Some(true)
        );
        assert!(parsed.started_at.is_some());
        assert_eq!(parsed.messages.len(), 2);
        assert!(parsed.messages[0].text.contains("Plain array text"));
        assert!(!parsed.messages.iter().any(
            |message| message.text.contains("hidden") || message.text.contains("unknown role")
        ));
        let types: Vec<_> = parsed
            .artifacts
            .iter()
            .map(|artifact| artifact.artifact_type.as_str())
            .collect();
        assert!(types.contains(&"image"));
        assert!(types.contains(&"thinking"));
        assert!(types.contains(&"tool_call"));
        assert!(types.contains(&"tool_result"));
        assert!(types.contains(&"file"));
        assert!(types.contains(&"custom_block"));
        assert!(parsed
            .facts
            .iter()
            .any(|fact| fact.role.as_deref() == Some("system")));
    }

    #[test]
    fn parse_prefers_session_start_then_stem_then_synthetic() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("sessions");
        write_session(
            &root,
            "ws",
            "stem-only",
            concat!(
                r#"{"type":"message","id":"m1","timestamp":"2026-04-12T18:17:28.000Z","message":{"role":"user","content":[{"type":"text","text":"stem"}]}}"#,
                "\n",
            ),
        );
        let adapter = DroidAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidate = adapter.discover(&source).expect("discover").remove(0);
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        let parsed = adapter.parse(&candidate, &snapshot).expect("parse");
        assert_eq!(parsed.external_session_id, "stem-only");
        assert!(!parsed.synthetic_identity);

        let mut synthetic = candidate;
        synthetic.external_session_id = None;
        synthetic.absolute_path = None;
        synthetic.source_path = "droid://session/synthetic-case".into();
        let body = concat!(
            r#"{"type":"message","message":{"role":"user","content":"hello"}}"#,
            "\n",
        );
        let snap = CaptureSnapshot {
            bytes: body.as_bytes().to_vec(),
            sha256: hex::encode(Sha256::digest(body.as_bytes())),
            byte_size: body.len() as u64,
            media_type: DROID_MEDIA_TYPE.into(),
            source_modified_at: None,
        };
        let first = adapter.parse(&synthetic, &snap).expect("parse");
        let second = adapter.parse(&synthetic, &snap).expect("parse again");
        assert!(first.synthetic_identity);
        assert_eq!(first.external_session_id, second.external_session_id);
        assert!(first.external_session_id.starts_with("synthetic-"));

        let start_id_path = write_session(
            &root,
            "ws",
            "filename-id",
            concat!(r#"{"type":"session_start","id":"session-start-id"}"#, "\n",),
        );
        let start_candidate = CaptureCandidate {
            source_kind: SourceKind::Droid,
            source_path: "droid://session/session-start-id".into(),
            absolute_path: Some(start_id_path),
            external_session_id: Some("filename-id".into()),
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: DROID_MEDIA_TYPE.into(),
        };
        let start_snapshot = adapter.snapshot(&start_candidate).expect("snapshot");
        let start_parsed = adapter
            .parse(&start_candidate, &start_snapshot)
            .expect("parse");
        assert_eq!(start_parsed.external_session_id, "session-start-id");
    }

    #[test]
    fn malformed_line_is_typed_parse_error() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("sessions");
        let path = write_session(&root, "ws", "bad", "{not-json\n");
        let adapter = DroidAdapter::new(&root);
        let candidate = CaptureCandidate {
            source_kind: SourceKind::Droid,
            source_path: "droid://session/bad".into(),
            absolute_path: Some(path),
            external_session_id: Some("bad".into()),
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: DROID_MEDIA_TYPE.into(),
        };
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        assert!(matches!(
            adapter.parse(&candidate, &snapshot),
            Err(SourceStageError::Parse(_))
        ));
    }

    #[test]
    fn malformed_utf8_is_typed_parse_error_and_snapshot_hash_is_exact() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("sessions");
        let path = write_session(&root, "ws", "bytes", "hello\n");
        let bytes = [0xff, 0xfe, b'\n'];
        fs::write(&path, bytes).expect("write bytes");
        let candidate = CaptureCandidate {
            source_kind: SourceKind::Droid,
            source_path: "droid://session/bytes".into(),
            absolute_path: Some(path),
            external_session_id: Some("bytes".into()),
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: DROID_MEDIA_TYPE.into(),
        };
        let adapter = DroidAdapter::new(&root);
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        assert_eq!(snapshot.byte_size, bytes.len() as u64);
        assert_eq!(snapshot.bytes, bytes);
        assert_eq!(snapshot.sha256, hex::encode(Sha256::digest(bytes)));
        assert!(matches!(
            adapter.parse(&candidate, &snapshot),
            Err(SourceStageError::Parse(_))
        ));
    }
}
