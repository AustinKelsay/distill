//! Pi SourceAdapter: file-backed detect/discover/snapshot/parse for Pi session JSONL files.
//!
//! Pi stores sessions as JSONL files under `~/.pi/agent/sessions/--<encoded-cwd>--/`.
//! Each file begins with a `session` header line containing the session id and cwd.
//! Subsequent lines are entries: `message` (user/assistant), `compaction`, `label`, etc.

use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::{
    CaptureCandidate, CaptureSnapshot, DiscoveredSource, ParsedArtifact, ParsedCapture, ParsedFact,
    ParsedMessage, ParserIdentity, SourceAdapter, SourceKind, SourceStageError,
};

/// Default Pi parser identity for Normalization Attempts.
pub const PI_PARSER_ID: &str = "pi";

/// Default Pi parser contract version.
pub const PI_PARSER_VERSION: &str = "1.0.0";

/// Media type for Pi session JSONL Captures.
pub const PI_MEDIA_TYPE: &str = "application/x-distill-pi+jsonl";

/// Pi SourceAdapter bound to one configured sessions root.
pub struct PiAdapter {
    root: PathBuf,
    parser: ParserIdentity,
}

impl PiAdapter {
    /// Create an adapter that detects only the supplied sessions root.
    #[allow(dead_code)]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_parser(
            root,
            ParserIdentity {
                id: PI_PARSER_ID.to_string(),
                version: PI_PARSER_VERSION.to_string(),
            },
        )
    }

    /**
     * Create an adapter with an explicit Pi parser identity.
     *
     * Parameters:
     * - `root`: Configured Pi sessions root (default `~/.pi/agent/sessions`).
     * - `parser`: Parser identity/version recorded on Normalization Attempts.
     */
    pub fn with_parser(root: impl Into<PathBuf>, parser: ParserIdentity) -> Self {
        Self {
            root: root.into(),
            parser,
        }
    }
}

impl SourceAdapter for PiAdapter {
    fn detect(&self) -> Result<DiscoveredSource, SourceStageError> {
        let root = canonicalize_existing(&self.root).map_err(SourceStageError::Detect)?;
        if !root.is_dir() {
            return Err(SourceStageError::Detect(format!(
                "pi sessions root is not a directory: {}",
                root.display()
            )));
        }
        Ok(DiscoveredSource {
            kind: SourceKind::Pi,
            display_name: "Pi".to_string(),
            data_root: root,
            parser: self.parser.clone(),
        })
    }

    fn discover(
        &self,
        source: &DiscoveredSource,
    ) -> Result<Vec<CaptureCandidate>, SourceStageError> {
        let mut candidates = discover_pi_sessions(&source.data_root)
            .map_err(SourceStageError::Discover)?;
        candidates.sort_by(|left, right| left.source_path.cmp(&right.source_path));
        Ok(candidates)
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
        parse_pi_jsonl(candidate, &snapshot.bytes)
    }
}

/**
 * Parse Distill-owned Pi Capture bytes without rereading the Pi sessions root.
 *
 * Parameters:
 * - `candidate`: Replay Candidate rebuilt from persisted Capture identity.
 * - `bytes`: Checksum-verified Distill-owned Capture bytes.
 */
pub(crate) fn parse_pi_bytes(
    candidate: &CaptureCandidate,
    bytes: &[u8],
) -> Result<ParsedCapture, SourceStageError> {
    parse_pi_jsonl(candidate, bytes)
}

/**
 * Recursively discover Pi session JSONL files under a sessions root.
 *
 * Parameters:
 * - `sessions_root`: Pi sessions directory.
 */
fn discover_pi_sessions(sessions_root: &Path) -> Result<Vec<CaptureCandidate>, String> {
    if !sessions_root.exists() {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::new();
    let mut file_paths = visit_files(sessions_root)?;
    file_paths.sort();
    for file_path in file_paths {
        if file_path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let relative = file_path
            .strip_prefix(sessions_root)
            .unwrap_or(file_path.as_path())
            .display()
            .to_string()
            .replace('\\', "/");
        let session_id = peek_session_id(&file_path).or_else(|| {
            // Scope the filename stem by the relative directory path to prevent
            // collisions between files with the same stem in different subdirectories.
            let stem = filename_stem(&file_path)?;
            let parent = file_path.parent().and_then(|p| {
                p.strip_prefix(sessions_root).ok().and_then(|rel| {
                    let s = rel.display().to_string().replace('\\', "/");
                    (!s.is_empty()).then_some(s)
                })
            });
            let scoped = match parent {
                Some(dir) => format!("{dir}::{stem}"),
                None => stem,
            };
            Some(scoped)
        });
        candidates.push(CaptureCandidate {
            source_kind: SourceKind::Pi,
            source_path: format!("pi://session/{relative}"),
            absolute_path: Some(file_path.clone()),
            external_session_id: session_id,
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: PI_MEDIA_TYPE.to_string(),
        });
    }
    Ok(candidates)
}

/**
 * Peek the session id from the `session` header line of a Pi JSONL file.
 */
fn peek_session_id(path: &Path) -> Option<String> {
    const HEADER_SCAN_BYTES: u64 = 1024 * 64;
    let file = fs::File::open(path).ok()?;
    for line_result in BufReader::new(file).take(HEADER_SCAN_BYTES).lines() {
        let Ok(line) = line_result else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("session") {
            continue;
        }
        if let Some(id) = value
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(id.to_string());
        }
    }
    None
}

/**
 * Parse Pi session JSONL bytes into Capture Facts, Messages, and Artifacts.
 *
 * Parameters:
 * - `candidate`: Capture Candidate providing identity and path hints.
 * - `bytes`: Exact snapshot bytes preserved by Distill.
 */
fn parse_pi_jsonl(
    candidate: &CaptureCandidate,
    bytes: &[u8],
) -> Result<ParsedCapture, SourceStageError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|err| SourceStageError::Parse(format!("pi bytes are not utf-8: {err}")))?;

    let mut facts = Vec::new();
    let mut messages = Vec::new();
    let mut artifacts = Vec::new();

    let mut session_id: Option<String> = None;
    let mut started_at: Option<String> = None;
    let mut updated_at: Option<String> = None;
    let mut project_path: Option<String> = None;
    let mut session_version: Option<i64> = None;

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

        if updated_at
            .as_deref()
            .is_none_or(|current| timestamp.as_deref().is_some_and(|next| next > current))
        {
            updated_at = timestamp.clone().or(updated_at);
        }

        // Session header line. First-wins to match peek_session_id behavior.
        if record_type == "session" {
            if session_id.is_none() {
                session_id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
            }
            if started_at.is_none() {
                started_at = value
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            if project_path.is_none() {
                project_path = value
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            if session_version.is_none() {
                session_version = value.get("version").and_then(Value::as_i64);
            }

            facts.push(ParsedFact {
                record_type: "session".to_string(),
                role: None,
                is_meta: true,
                content_text: None,
                content_json: value,
            });
            continue;
        }

        // Message entries.
        let message = value.get("message").and_then(Value::as_object);
        let role = message
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let content_blocks = normalize_content_blocks(message.and_then(|message| message.get("content")));
        let content_text = extract_text_blocks(&content_blocks);

        let is_message_entry = record_type == "message"
            && matches!(role.as_deref(), Some("user") | Some("assistant"));
        let has_text = !content_text.is_empty();
        let entry_id = value
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);

        let fact_ordinal = facts.len();
        facts.push(ParsedFact {
            record_type: record_type.to_string(),
            role: role.clone(),
            is_meta: !has_text,
            content_text: has_text.then_some(content_text.clone()),
            content_json: value.clone(),
        });

        let message_ordinal = if is_message_entry {
            let ordinal = messages.len();
            messages.push(ParsedMessage {
                role: role.clone().unwrap_or_else(|| "user".to_string()),
                message_kind: if has_text { "text".into() } else { "meta".into() },
                text: if has_text { content_text } else { "[tool]".into() },
                external_message_id: entry_id,
            });
            Some(ordinal)
        } else {
            None
        };
        for block in content_blocks {
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

        // Non-message entries (compaction, label, etc.) become meta facts.
        if record_type != "message" {
            // Already added as a meta fact above; nothing extra needed.
        }
    }

    let has_header_id = session_id
        .as_ref()
        .is_some_and(|value| !value.is_empty());
    let has_candidate_id = candidate
        .external_session_id
        .as_deref()
        .is_some_and(|value| !value.is_empty());

    let (resolved_external_session_id, provenance_strategy) = if has_header_id {
        // Real session ID from the file header — source provenance.
        (session_id.clone().unwrap(), json!({"kind": "source"}))
    } else if has_candidate_id {
        // Filename-derived fallback from discovery — synthetic filename_stem provenance.
        (
            candidate.external_session_id.clone().unwrap(),
            json!({"kind": "synthetic", "strategy": "filename_stem"}),
        )
    } else {
        // No identity available — synthetic SHA256 provenance.
        (
            synthetic_session_id(candidate),
            json!({"kind": "synthetic", "strategy": "source_path_sha256"}),
        )
    };
    let synthetic_identity = provenance_strategy.get("kind") == Some(&json!("synthetic"));

    let title = pick_pi_title(&messages);

    let mut metadata = Map::new();
    metadata.insert("external_session_id_provenance".to_string(), provenance_strategy);
    if synthetic_identity {
        metadata.insert("synthetic_identity".to_string(), json!(true));
        metadata.insert(
            "source_path".to_string(),
            json!(candidate.source_path.clone()),
        );
    }
    if let Some(version) = session_version {
        metadata.insert("session_version".to_string(), json!(version));
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
 * Normalize Pi message content into structured block objects.
 *
 * Pi content can be a string, an array of blocks, or absent.
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
 * Extract visible text from text blocks only.
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
 * Map a structured Pi content block to a canonical Artifact type.
 *
 * Text blocks are transcript candidates, not Artifacts. Other blocks
 * are preserved as Artifacts.
 */
fn artifact_type_for_block(block: &Map<String, Value>) -> Option<String> {
    let block_type = block.get("type").and_then(Value::as_str)?;
    match block_type {
        "text" => None,
        "image" => Some("image".to_string()),
        "tool_use" => Some("tool_call".to_string()),
        "tool_result" => Some("tool_result".to_string()),
        "file" => Some("file".to_string()),
        "audio" => Some("audio".to_string()),
        "video" => Some("video".to_string()),
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
        .or_else(|| block.get("name").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(160).collect())
}

/**
 * Prefer the first user message line (truncated) as the session title.
 */
fn pick_pi_title(messages: &[ParsedMessage]) -> Option<String> {
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
     * Build a minimal Pi sessions directory with session files.
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
            r#"{"type":"session","version":3,"id":"pi-ses-001","timestamp":"2026-06-01T10:00:00.000Z","cwd":"/tmp/demo"}"#,
            "\n",
            r#"{"type":"message","id":"msg_1","parentId":null,"timestamp":"2026-06-01T10:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"hello pi"}]}}"#,
            "\n",
            r#"{"type":"message","id":"msg_2","parentId":"msg_1","timestamp":"2026-06-01T10:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"hello! how can i help?"},{"type":"tool_use","name":"Read","input":{"file_path":"/tmp/demo/src/main.ts"}}]}}"#,
            "\n",
            r#"{"type":"message","id":"msg_3","parentId":"msg_2","timestamp":"2026-06-01T10:00:03.000Z","message":{"role":"user","content":[{"type":"text","text":"fix the import"}]}}"#,
            "\n",
            r#"{"type":"compaction","id":"cmp_1","parentId":"msg_3","timestamp":"2026-06-01T10:05:00.000Z","tokensBefore":500,"tokensAfter":200}"#,
            "\n",
            r#"{"type":"label","id":"lbl_1","parentId":"msg_3","timestamp":"2026-06-01T10:06:00.000Z","label":"important","targetId":"msg_3"}"#,
            "\n",
        )
        .to_string()
    }

    #[test]
    fn detect_reports_configured_pi_sessions_root() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("pi-sessions");
        fs::create_dir_all(&root).expect("root");
        let adapter = PiAdapter::new(&root);
        let discovered = adapter.detect().expect("detect");
        assert_eq!(discovered.kind, SourceKind::Pi);
        assert_eq!(discovered.display_name, "Pi");
        assert_eq!(discovered.parser.id, PI_PARSER_ID);
        assert!(discovered.data_root.ends_with("pi-sessions"));
    }

    #[test]
    fn discover_finds_jsonl_files_and_peeks_session_ids() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("pi-sessions");
        write_session(
            &root,
            "--home-user-project--/20260601_100000_pi-ses-001.jsonl",
            &dialogue_body(),
        );
        write_session(
            &root,
            "--home-user-other--/20260601_110000_pi-ses-002.jsonl",
            concat!(
                r#"{"type":"session","version":3,"id":"pi-ses-002","timestamp":"2026-06-01T11:00:00.000Z","cwd":"/home/user/other"}"#,
                "\n",
                r#"{"type":"message","id":"msg_1","timestamp":"2026-06-01T11:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"second session"}]}}"#,
                "\n",
            ),
        );

        let adapter = PiAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidates = adapter.discover(&source).expect("discover");
        assert_eq!(candidates.len(), 2);

        let first = candidates
            .iter()
            .find(|c| c.external_session_id.as_deref() == Some("pi-ses-001"))
            .expect("first session");
        assert!(first.source_path.starts_with("pi://session/"));
        assert!(first.absolute_path.is_some());

        let second = candidates
            .iter()
            .find(|c| c.external_session_id.as_deref() == Some("pi-ses-002"))
            .expect("second session");
        assert!(second.source_path.starts_with("pi://session/"));
    }

    #[test]
    fn snapshot_preserves_exact_bytes_and_sha256() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("pi-sessions");
        let body = dialogue_body();
        let path = write_session(
            &root,
            "--home-user-project--/20260601_100000_pi-ses-001.jsonl",
            &body,
        );
        let adapter = PiAdapter::new(&root);
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
    fn parse_dialogue_metadata_and_artifacts() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("pi-sessions");
        write_session(
            &root,
            "--home-user-project--/20260601_100000_pi-ses-001.jsonl",
            &dialogue_body(),
        );

        let adapter = PiAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidate = adapter.discover(&source).expect("discover").remove(0);
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        let parsed = adapter.parse(&candidate, &snapshot).expect("parse");

        assert_eq!(parsed.external_session_id, "pi-ses-001");
        assert!(!parsed.synthetic_identity);
        assert_eq!(parsed.title.as_deref(), Some("hello pi"));
        assert_eq!(parsed.project_path.as_deref(), Some("/tmp/demo"));
        assert_eq!(parsed.messages.len(), 3);
        assert_eq!(parsed.messages[0].role, "user");
        assert_eq!(parsed.messages[0].text, "hello pi");
        assert_eq!(parsed.messages[1].role, "assistant");
        assert_eq!(parsed.messages[1].text, "hello! how can i help?");
        assert_eq!(parsed.messages[2].role, "user");
        assert_eq!(parsed.messages[2].text, "fix the import");

        assert!(parsed.facts.len() >= 6);
        assert!(parsed
            .facts
            .iter()
            .any(|fact| fact.record_type == "session"));
        assert!(parsed
            .facts
            .iter()
            .any(|fact| fact.record_type == "compaction"));
        assert!(parsed
            .facts
            .iter()
            .any(|fact| fact.record_type == "label"));

        assert!(parsed
            .artifacts
            .iter()
            .any(|artifact| artifact.artifact_type == "tool_call"));

        assert_eq!(
            parsed
                .metadata
                .get("session_version")
                .and_then(Value::as_i64),
            Some(3)
        );
    }

    #[test]
    fn parse_synthesizes_deterministic_identity_without_session_header() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("pi-sessions");
        write_session(
            &root,
            "orphan-session.jsonl",
            concat!(
                r#"{"type":"message","id":"msg_1","timestamp":"2026-06-01T10:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"orphan"}]}}"#,
                "\n",
            ),
        );
        let adapter = PiAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidate = adapter.discover(&source).expect("discover").remove(0);
        // Filename stem "orphan-session" becomes the external_session_id.
        assert_eq!(
            candidate.external_session_id.as_deref(),
            Some("orphan-session")
        );
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        let parsed = adapter.parse(&candidate, &snapshot).expect("parse");
        // No session header line with id, so the candidate's stem identity is used.
        // Filename-derived identities are synthetic with filename_stem provenance.
        assert_eq!(parsed.external_session_id, "orphan-session");
        assert!(parsed.synthetic_identity);
        assert_eq!(
            parsed
                .metadata
                .pointer("/external_session_id_provenance/kind")
                .and_then(Value::as_str),
            Some("synthetic")
        );
        assert_eq!(
            parsed
                .metadata
                .pointer("/external_session_id_provenance/strategy")
                .and_then(Value::as_str),
            Some("filename_stem")
        );
        let again = adapter.parse(&candidate, &snapshot).expect("parse again");
        assert_eq!(parsed.external_session_id, again.external_session_id);
    }

    #[test]
    fn parse_tolerates_string_content() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("pi-sessions");
        write_session(
            &root,
            "string-content.jsonl",
            concat!(
                r#"{"type":"session","version":3,"id":"string-session","timestamp":"2026-06-01T10:00:00.000Z"}"#,
                "\n",
                r#"{"type":"message","id":"msg_1","timestamp":"2026-06-01T10:00:01.000Z","message":{"role":"user","content":"plain string user"}}"#,
                "\n",
                r#"{"type":"message","id":"msg_2","timestamp":"2026-06-01T10:00:02.000Z","message":{"role":"assistant","content":"plain string assistant"}}"#,
                "\n",
            ),
        );
        let adapter = PiAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        let candidate = adapter.discover(&source).expect("discover").remove(0);
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        let parsed = adapter.parse(&candidate, &snapshot).expect("parse");
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].text, "plain string user");
        assert_eq!(parsed.messages[1].text, "plain string assistant");
    }

    #[test]
    fn parse_force_synthetic_when_no_header_and_no_stem() {
        let candidate = CaptureCandidate {
            source_kind: SourceKind::Pi,
            source_path: "pi://session/no-identity.jsonl".into(),
            absolute_path: None,
            external_session_id: None,
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: PI_MEDIA_TYPE.into(),
        };
        let body = concat!(
            r#"{"type":"message","id":"msg_1","timestamp":"2026-06-01T10:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"no identity"}]}}"#,
            "\n",
        );
        let snapshot = CaptureSnapshot {
            bytes: body.as_bytes().to_vec(),
            sha256: hex::encode(Sha256::digest(body.as_bytes())),
            byte_size: body.len() as u64,
            media_type: PI_MEDIA_TYPE.into(),
            source_modified_at: None,
        };
        let first = parse_pi_jsonl(&candidate, &snapshot.bytes).expect("parse");
        let second = parse_pi_jsonl(&candidate, &snapshot.bytes).expect("parse again");
        assert!(first.synthetic_identity);
        assert!(first.external_session_id.starts_with("synthetic-"));
        assert_eq!(first.external_session_id, second.external_session_id);
        assert_eq!(
            first
                .metadata
                .pointer("/external_session_id_provenance/kind")
                .and_then(Value::as_str),
            Some("synthetic")
        );
    }

    #[test]
    fn parse_empty_session_with_only_header() {
        let candidate = CaptureCandidate {
            source_kind: SourceKind::Pi,
            source_path: "pi://session/empty.jsonl".into(),
            absolute_path: None,
            external_session_id: Some("empty-session".into()),
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: PI_MEDIA_TYPE.into(),
        };
        let body = concat!(
            r#"{"type":"session","version":3,"id":"empty-session","timestamp":"2026-06-01T12:00:00.000Z","cwd":"/tmp/empty"}"#,
            "\n",
        );
        let parsed = parse_pi_jsonl(&candidate, body.as_bytes()).expect("parse");
        assert_eq!(parsed.external_session_id, "empty-session");
        assert!(!parsed.synthetic_identity);
        assert_eq!(parsed.messages.len(), 0);
        assert_eq!(parsed.artifacts.len(), 0);
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(parsed.facts[0].record_type, "session");
        assert_eq!(parsed.project_path.as_deref(), Some("/tmp/empty"));
        assert_eq!(
            parsed.started_at.as_deref(),
            Some("2026-06-01T12:00:00.000Z")
        );
    }

    #[test]
    fn parse_branched_session_tree() {
        // Pi sessions use parentId for tree-structured branching.
        let body = concat!(
            r#"{"type":"session","version":3,"id":"branch-session","timestamp":"2026-06-01T12:00:00.000Z","cwd":"/tmp/branch"}"#,
            "\n",
            r#"{"type":"message","id":"msg_1","parentId":null,"timestamp":"2026-06-01T12:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"initial request"}]}}"#,
            "\n",
            r#"{"type":"message","id":"msg_2","parentId":"msg_1","timestamp":"2026-06-01T12:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"main branch response"}]}}"#,
            "\n",
            r#"{"type":"message","id":"msg_3","parentId":"msg_1","timestamp":"2026-06-01T12:00:03.000Z","message":{"role":"assistant","content":[{"type":"text","text":"forked branch response"}]}}"#,
            "\n",
        );
        let candidate = CaptureCandidate {
            source_kind: SourceKind::Pi,
            source_path: "pi://session/branch.jsonl".into(),
            absolute_path: None,
            external_session_id: Some("branch-session".into()),
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: PI_MEDIA_TYPE.into(),
        };
        let parsed = parse_pi_jsonl(&candidate, body.as_bytes()).expect("parse");
        assert_eq!(parsed.external_session_id, "branch-session");
        assert_eq!(parsed.messages.len(), 3);
        assert_eq!(parsed.messages[0].text, "initial request");
        assert_eq!(parsed.messages[1].text, "main branch response");
        assert_eq!(parsed.messages[2].text, "forked branch response");
        // All entries preserved as facts regardless of tree structure.
        assert_eq!(parsed.facts.len(), 4);
    }

    #[test]
    fn parse_header_id_is_first_wins() {
        // Only the first session header contributes the identity.
        let body = concat!(
            r#"{"type":"session","version":3,"id":"first-session","timestamp":"2026-06-01T12:00:00.000Z","cwd":"/tmp/first"}"#,
            "\n",
            r#"{"type":"session","version":3,"id":"second-session","timestamp":"2026-06-01T12:01:00.000Z","cwd":"/tmp/second"}"#,
            "\n",
            r#"{"type":"message","id":"msg_1","parentId":null,"timestamp":"2026-06-01T12:02:00.000Z","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#,
            "\n",
        );
        let candidate = CaptureCandidate {
            source_kind: SourceKind::Pi,
            source_path: "pi://session/dual-header.jsonl".into(),
            absolute_path: None,
            external_session_id: None,
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: PI_MEDIA_TYPE.into(),
        };
        let parsed = parse_pi_jsonl(&candidate, body.as_bytes()).expect("parse");
        // First session header wins for id, timestamp, and cwd.
        assert_eq!(parsed.external_session_id, "first-session");
        assert_eq!(parsed.project_path.as_deref(), Some("/tmp/first"));
        assert_eq!(
            parsed.started_at.as_deref(),
            Some("2026-06-01T12:00:00.000Z")
        );
    }

    #[test]
    fn stage_errors_are_typed() {
        let missing = PiAdapter::new("/tmp/distill-pi-missing-root-does-not-exist");
        assert!(matches!(missing.detect(), Err(SourceStageError::Detect(_))));

        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("pi-sessions");
        fs::create_dir_all(&root).expect("root");
        let adapter = PiAdapter::new(&root);
        let source = adapter.detect().expect("detect");
        assert!(adapter.discover(&source).expect("discover").is_empty());

        let bad_candidate = CaptureCandidate {
            source_kind: SourceKind::Pi,
            source_path: "pi://session/missing.jsonl".into(),
            absolute_path: Some(root.join("missing.jsonl")),
            external_session_id: None,
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: PI_MEDIA_TYPE.into(),
        };
        assert!(matches!(
            adapter.snapshot(&bad_candidate),
            Err(SourceStageError::Snapshot(_))
        ));

        let path = write_session(&root, "bad.jsonl", "{not-json\n");
        let candidate = CaptureCandidate {
            source_kind: SourceKind::Pi,
            source_path: format!("pi://session/{}", path.display()),
            absolute_path: Some(path),
            external_session_id: None,
            title: None,
            is_virtual: false,
            virtual_bytes: None,
            media_type: PI_MEDIA_TYPE.into(),
        };
        let snapshot = adapter.snapshot(&candidate).expect("snapshot");
        assert!(matches!(
            adapter.parse(&candidate, &snapshot),
            Err(SourceStageError::Parse(_))
        ));
    }
}
