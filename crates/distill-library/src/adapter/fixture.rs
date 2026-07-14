//! Fixture SourceAdapter: detect/discover/snapshot/parse for explicit test roots.

use std::fs;
use std::path::{Path, PathBuf};

use semver::Version;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{
    CaptureCandidate, CaptureSnapshot, DiscoveredSource, ParsedArtifact, ParsedCapture, ParsedFact,
    ParsedMessage, ParserIdentity, SourceAdapter, SourceKind, SourceStageError,
};

/// Manifest file name that marks a Fixture root.
pub const FIXTURE_MANIFEST_NAME: &str = "distill.fixture.json";

/// Default Fixture parser identity for Normalization Attempts.
pub const FIXTURE_PARSER_ID: &str = "fixture";

/// Default Fixture parser contract version.
pub const FIXTURE_PARSER_VERSION: &str = "1.0.0";

/// Fixture SourceAdapter bound to one explicitly supplied root.
pub struct FixtureAdapter {
    root: PathBuf,
    parser: ParserIdentity,
}

impl FixtureAdapter {
    /// Create an adapter that only detects the supplied root.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_parser(
            root,
            ParserIdentity {
                id: FIXTURE_PARSER_ID.to_string(),
                version: FIXTURE_PARSER_VERSION.to_string(),
            },
        )
    }

    /**
     * Create an adapter with an explicit registered Fixture parser identity.
     *
     * Parameters:
     * - `root`: Fixture root containing `distill.fixture.json`.
     * - `parser`: Registered Fixture parser identity/version for Attempts.
     */
    pub fn with_parser(root: impl Into<PathBuf>, parser: ParserIdentity) -> Self {
        Self {
            root: root.into(),
            parser,
        }
    }
}

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    version: u32,
    captures: Vec<FixtureCaptureEntry>,
}

#[derive(Debug, Deserialize)]
struct FixtureCaptureEntry {
    id: String,
    #[serde(default = "default_kind")]
    kind: String,
    #[serde(default)]
    relative_path: Option<String>,
    #[serde(default)]
    virtual_text: Option<String>,
    #[serde(default)]
    external_session_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default = "default_media_type")]
    media_type: String,
}

fn default_kind() -> String {
    "file".to_string()
}

fn default_media_type() -> String {
    "application/x-distill-fixture+jsonl".to_string()
}

impl SourceAdapter for FixtureAdapter {
    fn detect(&self) -> Result<DiscoveredSource, SourceStageError> {
        let root = canonicalize_existing(&self.root).map_err(SourceStageError::Detect)?;
        let manifest = root.join(FIXTURE_MANIFEST_NAME);
        if !manifest.is_file() {
            return Err(SourceStageError::Detect(format!(
                "missing {FIXTURE_MANIFEST_NAME} under {}",
                root.display()
            )));
        }
        Ok(DiscoveredSource {
            kind: SourceKind::Fixture,
            display_name: "Fixture".to_string(),
            data_root: root,
            parser: self.parser.clone(),
        })
    }

    fn discover(
        &self,
        source: &DiscoveredSource,
    ) -> Result<Vec<CaptureCandidate>, SourceStageError> {
        let manifest_path = source.data_root.join(FIXTURE_MANIFEST_NAME);
        let raw = fs::read_to_string(&manifest_path)
            .map_err(|err| SourceStageError::Discover(err.to_string()))?;
        let manifest_value = crate::privacy::parse_json_document_bounded(&raw)
            .map_err(SourceStageError::Discover)?;
        let manifest: FixtureManifest = serde_json::from_value(manifest_value)
            .map_err(|err| SourceStageError::Discover(err.to_string()))?;
        if manifest.version != 1 {
            return Err(SourceStageError::Discover(format!(
                "unsupported fixture manifest version {}",
                manifest.version
            )));
        }

        let mut candidates = Vec::with_capacity(manifest.captures.len());
        for entry in manifest.captures {
            let candidate = match entry.kind.as_str() {
                "file" => {
                    let relative = entry.relative_path.ok_or_else(|| {
                        SourceStageError::Discover(format!(
                            "fixture capture {} missing relative_path",
                            entry.id
                        ))
                    })?;
                    let absolute = source.data_root.join(&relative);
                    CaptureCandidate {
                        source_kind: SourceKind::Fixture,
                        source_path: format!("fixture://{}/{}", entry.id, relative),
                        absolute_path: Some(absolute),
                        external_session_id: entry.external_session_id,
                        title: entry.title,
                        is_virtual: false,
                        virtual_bytes: None,
                        media_type: entry.media_type,
                    }
                }
                "virtual" => {
                    let text = entry.virtual_text.ok_or_else(|| {
                        SourceStageError::Discover(format!(
                            "fixture capture {} missing virtual_text",
                            entry.id
                        ))
                    })?;
                    CaptureCandidate {
                        source_kind: SourceKind::Fixture,
                        source_path: format!("fixture://virtual/{}", entry.id),
                        absolute_path: None,
                        external_session_id: entry.external_session_id,
                        title: entry.title,
                        is_virtual: true,
                        virtual_bytes: Some(text.into_bytes()),
                        media_type: entry.media_type,
                    }
                }
                other => {
                    return Err(SourceStageError::Discover(format!(
                        "unknown fixture capture kind {other}"
                    )));
                }
            };
            candidates.push(candidate);
        }
        Ok(candidates)
    }

    fn snapshot(&self, candidate: &CaptureCandidate) -> Result<CaptureSnapshot, SourceStageError> {
        let bytes = if candidate.is_virtual {
            candidate
                .virtual_bytes
                .clone()
                .ok_or_else(|| SourceStageError::Snapshot("missing virtual bytes".into()))?
        } else {
            let path = candidate
                .absolute_path
                .as_ref()
                .ok_or_else(|| SourceStageError::Snapshot("missing absolute path".into()))?;
            fs::read(path).map_err(|err| SourceStageError::Snapshot(err.to_string()))?
        };
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let source_modified_at = candidate
            .absolute_path
            .as_ref()
            .and_then(|path| fs::metadata(path).ok())
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
        parse_fixture_jsonl(candidate, &snapshot.bytes, &self.parser.version)
    }
}

/**
 * Parse Fixture JSONL bytes using a registered Fixture parser version.
 *
 * Parameters:
 * - `candidate`: Capture Candidate providing identity hints.
 * - `bytes`: Exact Distill-owned or snapshot bytes.
 * - `parser_version`: Registered Fixture parser version executing this Attempt.
 */
pub fn parse_fixture_bytes(
    candidate: &CaptureCandidate,
    bytes: &[u8],
    parser_version: &str,
) -> Result<ParsedCapture, SourceStageError> {
    parse_fixture_jsonl(candidate, bytes, parser_version)
}

/**
 * Parse Fixture JSONL into Capture Facts, Transcript Messages, and Artifacts.
 *
 * Parameters:
 * - `candidate`: Capture Candidate providing identity hints.
 * - `bytes`: Exact snapshot bytes.
 * - `parser_version`: Registered Fixture parser version executing this Attempt.
 */
fn parse_fixture_jsonl(
    candidate: &CaptureCandidate,
    bytes: &[u8],
    parser_version: &str,
) -> Result<ParsedCapture, SourceStageError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|err| SourceStageError::Parse(format!("fixture bytes are not utf-8: {err}")))?;

    let mut facts = Vec::new();
    let mut messages = Vec::new();
    let mut artifacts = Vec::new();
    let mut title = candidate.title.clone();
    let mut summary = None;
    let mut project_path = None;
    let mut source_url = None;
    let mut started_at = None;
    let mut updated_at = None;
    let mut metadata = json!({});

    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = crate::privacy::parse_json_line_bounded(trimmed, line_no + 1)
            .map_err(SourceStageError::Parse)?;
        let record_type = value
            .get("record_type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        match record_type.as_str() {
            "require_parser_min" => {
                let required = value
                    .get("version")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        SourceStageError::Parse(format!(
                            "line {}: require_parser_min missing version",
                            line_no + 1
                        ))
                    })?;
                if !version_at_least(parser_version, required) {
                    return Err(SourceStageError::Parse(format!(
                        "parser {parser_version} is below required minimum {required}"
                    )));
                }
                facts.push(ParsedFact {
                    record_type: "require_parser_min".into(),
                    role: None,
                    is_meta: true,
                    content_text: Some(required.to_string()),
                    content_json: value,
                });
            }
            "force_projection_fail" => {
                // Parses successfully, then fails projection CHECK intentionally.
                let body = value
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("projection fail")
                    .to_string();
                let fact_ordinal = facts.len();
                facts.push(ParsedFact {
                    record_type: "force_projection_fail".into(),
                    role: None,
                    is_meta: true,
                    content_text: Some(body.clone()),
                    content_json: value.clone(),
                });
                messages.push(ParsedMessage {
                    role: "system".into(),
                    message_kind: "invalid".into(),
                    text: body,
                    external_message_id: None,
                });
                let _ = fact_ordinal;
            }
            "message" => {
                let role = value
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let body = value
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let message_kind = value
                    .get("message_kind")
                    .and_then(Value::as_str)
                    .unwrap_or("text")
                    .to_string();
                let fact_ordinal = facts.len();
                facts.push(ParsedFact {
                    record_type: "message".into(),
                    role: Some(role.clone()),
                    is_meta: message_kind == "meta",
                    content_text: Some(body.clone()),
                    content_json: value.clone(),
                });
                let message_ordinal = messages.len();
                messages.push(ParsedMessage {
                    role,
                    message_kind,
                    text: body,
                    external_message_id: value
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                });
                let _ = (fact_ordinal, message_ordinal);
            }
            "session_meta" => {
                if let Some(t) = value.get("title").and_then(Value::as_str) {
                    title = Some(t.to_string());
                }
                if let Some(s) = value.get("summary").and_then(Value::as_str) {
                    summary = Some(s.to_string());
                }
                if let Some(path) = value.get("project_path").and_then(Value::as_str) {
                    project_path = Some(path.to_string());
                }
                if let Some(url) = value.get("source_url").and_then(Value::as_str) {
                    source_url = Some(url.to_string());
                }
                if let Some(ts) = value.get("started_at").and_then(Value::as_str) {
                    started_at = Some(ts.to_string());
                }
                if let Some(ts) = value.get("updated_at").and_then(Value::as_str) {
                    updated_at = Some(ts.to_string());
                }
                if let Some(meta) = value.get("metadata") {
                    metadata = meta.clone();
                }
                facts.push(ParsedFact {
                    record_type: "session_meta".into(),
                    role: None,
                    is_meta: true,
                    content_text: None,
                    content_json: value,
                });
            }
            other => {
                let fact_ordinal = facts.len();
                facts.push(ParsedFact {
                    record_type: other.to_string(),
                    role: value
                        .get("role")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    is_meta: true,
                    content_text: value
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    content_json: value.clone(),
                });
                artifacts.push(ParsedArtifact {
                    artifact_type: other.to_string(),
                    message_ordinal: None,
                    fact_ordinal: Some(fact_ordinal),
                    text_preview: value
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    content_json: value,
                });
            }
        }
    }

    let external_session_id = candidate
        .external_session_id
        .clone()
        .unwrap_or_else(|| synthetic_session_id(candidate));
    let synthetic_identity = candidate.external_session_id.is_none();
    if synthetic_identity {
        metadata = json!({
            "synthetic_identity": true,
            "source_path": candidate.source_path,
        });
    }

    Ok(ParsedCapture {
        external_session_id,
        synthetic_identity,
        title,
        summary,
        project_path,
        source_url,
        started_at,
        updated_at,
        metadata,
        facts,
        messages,
        artifacts,
    })
}

/**
 * Derive a deterministic synthetic Session Identity from the candidate path.
 */
fn synthetic_session_id(candidate: &CaptureCandidate) -> String {
    let digest = Sha256::digest(candidate.source_path.as_bytes());
    format!("synthetic-{}", &hex::encode(digest)[..16])
}

/**
 * Compare dotted numeric versions; returns true when `current >= required`.
 */
fn version_at_least(current: &str, required: &str) -> bool {
    Version::parse(current)
        .and_then(|current| Version::parse(required).map(|required| current >= required))
        .unwrap_or(false)
}

/**
 * Canonicalize an existing path, mapping IO failures into strings.
 */
fn canonicalize_existing(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|err| format!("{}: {err}", path.display()))
}
