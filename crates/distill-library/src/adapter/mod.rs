//! SourceAdapter seam shared by Fixture, Codex, Claude Code, OpenCode, and Droid adapters.

mod claude;
mod codex;
mod droid;
mod fixture;
mod opencode;
mod registry;

pub use claude::{ClaudeAdapter, CLAUDE_PARSER_ID, CLAUDE_PARSER_VERSION};
pub(crate) use codex::find_executable;
pub use codex::{CodexAdapter, CODEX_PARSER_ID, CODEX_PARSER_VERSION};
pub use droid::{default_droid_sessions_root, DroidAdapter, DROID_PARSER_ID, DROID_PARSER_VERSION};
pub use fixture::{parse_fixture_bytes, FixtureAdapter, FIXTURE_PARSER_ID, FIXTURE_PARSER_VERSION};
pub use opencode::{OpenCodeAdapter, OPENCODE_PARSER_ID, OPENCODE_PARSER_VERSION};
pub use registry::ParserRegistry;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Closed Source kind identifiers for v1.
///
/// [`SourceKind::Fixture`], [`SourceKind::Codex`], [`SourceKind::ClaudeCode`],
/// [`SourceKind::OpenCode`], and [`SourceKind::Droid`] have concrete adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Synthetic Fixture Source used by contract tests and smoke harnesses.
    Fixture,
    /// Codex Source adapter (#26).
    Codex,
    /// Claude Code Source adapter (#27).
    ClaudeCode,
    /// OpenCode Source adapter (#28).
    OpenCode,
    /// Factory Droid Source adapter (#29).
    Droid,
}

impl SourceKind {
    /// Stable string form persisted in SQLite.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fixture => "fixture",
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::OpenCode => "opencode",
            Self::Droid => "droid",
        }
    }

    /// Parse a stable Source kind string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "fixture" => Some(Self::Fixture),
            "codex" => Some(Self::Codex),
            "claude_code" => Some(Self::ClaudeCode),
            "opencode" => Some(Self::OpenCode),
            "droid" => Some(Self::Droid),
            _ => None,
        }
    }

    /// Every closed v1 Source kind in stable order.
    pub fn all() -> &'static [Self] {
        &[
            Self::Fixture,
            Self::Codex,
            Self::ClaudeCode,
            Self::OpenCode,
            Self::Droid,
        ]
    }
}

/// Detected Source installation or configured root.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveredSource {
    /// Source kind.
    pub kind: SourceKind,
    /// Human-readable label.
    pub display_name: String,
    /// Configured or detected data root.
    pub data_root: PathBuf,
    /// Parser identity and version used for Normalization Attempts.
    pub parser: ParserIdentity,
}

/// Versioned parser identity owned by a SourceAdapter implementation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParserIdentity {
    /// Stable parser identifier.
    pub id: String,
    /// Parser contract version.
    pub version: String,
}

/// Capture Candidate discovered before snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureCandidate {
    /// Source kind.
    pub source_kind: SourceKind,
    /// Stable source or virtual path used for dedupe.
    pub source_path: String,
    /// Absolute filesystem path for file-backed candidates.
    pub absolute_path: Option<PathBuf>,
    /// Provider or Fixture Session Identity when known.
    pub external_session_id: Option<String>,
    /// Optional title hint from the Fixture manifest.
    pub title: Option<String>,
    /// True when content is supplied virtually rather than from a file path.
    pub is_virtual: bool,
    /// Virtual payload bytes when `is_virtual` is true.
    pub virtual_bytes: Option<Vec<u8>>,
    /// Media type for the eventual Capture.
    pub media_type: String,
}

/// Raw snapshot bytes plus checksum metadata.
#[derive(Clone, Debug)]
pub struct CaptureSnapshot {
    /// Exact source bytes.
    pub bytes: Vec<u8>,
    /// SHA-256 hex digest of `bytes`.
    pub sha256: String,
    /// Byte length.
    pub byte_size: u64,
    /// Media type.
    pub media_type: String,
    /// Optional source modified timestamp (RFC3339).
    pub source_modified_at: Option<String>,
}

/// Immutable provider-shaped Capture Fact input from parse.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedFact {
    /// Source record type.
    pub record_type: String,
    /// Optional role.
    pub role: Option<String>,
    /// Meta marker.
    pub is_meta: bool,
    /// Optional free-text preview.
    pub content_text: Option<String>,
    /// Structured payload.
    pub content_json: Value,
}

/// Normalized Transcript Message input from parse.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedMessage {
    /// Role.
    pub role: String,
    /// `text` or `meta`.
    pub message_kind: String,
    /// Visible text.
    pub text: String,
    /// Optional external message id.
    pub external_message_id: Option<String>,
}

/// Normalized Artifact input from parse.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedArtifact {
    /// Artifact type.
    pub artifact_type: String,
    /// Optional index into parsed messages for placement.
    pub message_ordinal: Option<usize>,
    /// Optional index into parsed facts for provenance.
    pub fact_ordinal: Option<usize>,
    /// Optional preview.
    pub text_preview: Option<String>,
    /// Structured content.
    pub content_json: Value,
}

/// Complete parse result for one Capture.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedCapture {
    /// Session Identity external id.
    pub external_session_id: String,
    /// True when the id was synthesized.
    pub synthetic_identity: bool,
    /// Optional title.
    pub title: Option<String>,
    /// Optional summary.
    pub summary: Option<String>,
    /// Optional project path from session metadata.
    #[serde(default)]
    pub project_path: Option<String>,
    /// Optional source URL from session metadata.
    #[serde(default)]
    pub source_url: Option<String>,
    /// Optional session started_at timestamp from session metadata.
    #[serde(default)]
    pub started_at: Option<String>,
    /// Optional session updated_at timestamp from session metadata.
    #[serde(default)]
    pub updated_at: Option<String>,
    /// Session metadata object.
    pub metadata: Value,
    /// Immutable Capture Facts.
    pub facts: Vec<ParsedFact>,
    /// Projected Transcript Messages.
    pub messages: Vec<ParsedMessage>,
    /// Projected Artifacts.
    pub artifacts: Vec<ParsedArtifact>,
}

/// Typed SourceAdapter stage errors.
#[derive(Debug, Error)]
pub enum SourceStageError {
    /// Detection failed.
    #[error("detect failed: {0}")]
    Detect(String),
    /// Discovery failed.
    #[error("discover failed: {0}")]
    Discover(String),
    /// Snapshot failed.
    #[error("snapshot failed: {0}")]
    Snapshot(String),
    /// Parse failed.
    #[error("parse failed: {0}")]
    Parse(String),
}

impl SourceStageError {
    /**
     * Stable public-seam class for redacted SourceAdapter diagnostics.
     *
     * Stage details remain internal and are never copied into caller-facing
     * detection messages.
     */
    pub(crate) fn code(&self) -> &'static str {
        "source_adapter"
    }
}

/// Production SourceAdapter seam.
pub trait SourceAdapter {
    /// Detect whether this Source is present for the configured root.
    fn detect(&self) -> Result<DiscoveredSource, SourceStageError>;

    /// Discover Capture Candidates under the detected Source.
    fn discover(
        &self,
        source: &DiscoveredSource,
    ) -> Result<Vec<CaptureCandidate>, SourceStageError>;

    /// Snapshot exact bytes for a candidate.
    fn snapshot(&self, candidate: &CaptureCandidate) -> Result<CaptureSnapshot, SourceStageError>;

    /// Parse snapshot bytes into facts, messages, and artifacts.
    fn parse(
        &self,
        candidate: &CaptureCandidate,
        snapshot: &CaptureSnapshot,
    ) -> Result<ParsedCapture, SourceStageError>;
}

/**
 * Parse Distill-owned Capture bytes for renormalization without rereading a Source root.
 *
 * OpenCode replay never invokes the provider executable. File-backed Sources never
 * reopen original paths; auxiliary root indexes are omitted during replay.
 *
 * Parameters:
 * - `kind`: Persisted Capture Source kind.
 * - `candidate`: Replay Candidate rebuilt from Capture identity (no absolute path).
 * - `bytes`: Checksum-verified Distill-owned Capture bytes.
 * - `parser_version`: Registered parser version executing this Attempt.
 */
pub(crate) fn parse_replay_bytes(
    kind: SourceKind,
    candidate: &CaptureCandidate,
    bytes: &[u8],
    parser_version: &str,
) -> Result<ParsedCapture, SourceStageError> {
    match kind {
        SourceKind::Fixture => parse_fixture_bytes(candidate, bytes, parser_version),
        SourceKind::Codex => codex::parse_codex_bytes(candidate, bytes),
        SourceKind::ClaudeCode => claude::parse_claude_bytes(candidate, bytes),
        SourceKind::OpenCode => opencode::parse_opencode_bytes(candidate, bytes),
        SourceKind::Droid => droid::parse_droid_bytes(candidate, bytes),
    }
}
