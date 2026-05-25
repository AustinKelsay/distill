use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub type JsonMap = Map<String, Value>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Codex,
    ClaudeCode,
    OpenCode,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::OpenCode => "opencode",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "OpenAI Codex CLI",
            Self::ClaudeCode => "Claude Code",
            Self::OpenCode => "OpenCode",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStatus {
    Installed,
    NotFound,
    Partial,
}

impl InstallStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::NotFound => "not_found",
            Self::Partial => "partial",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourcePathCheck {
    pub label: String,
    pub path: String,
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_count: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveredSource {
    pub kind: SourceKind,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_root: Option<PathBuf>,
    pub install_status: InstallStatus,
    pub checks: Vec<SourcePathCheck>,
    #[serde(default)]
    pub metadata: JsonMap,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveredCapture {
    pub source_kind: SourceKind,
    pub capture_kind: String,
    pub source_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_modified_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_size_bytes: Option<u64>,
    #[serde(default)]
    pub metadata: JsonMap,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureSnapshot {
    pub raw_text: String,
    pub raw_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_modified_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_size_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedCaptureRecord {
    pub line_no: usize,
    pub record_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_provider_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub is_meta: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_text: Option<String>,
    pub content_json: Value,
    #[serde(default)]
    pub metadata: JsonMap,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormalizedSession {
    pub source_kind: SourceKind,
    pub external_session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub metadata: JsonMap,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormalizedMessage {
    pub source_line_no: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_external_message_id: Option<String>,
    pub role: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    pub message_kind: String,
    #[serde(default)]
    pub metadata: JsonMap,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormalizedArtifact {
    pub source_line_no: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_message_id: Option<String>,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub payload: JsonMap,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedCapture {
    pub session: NormalizedSession,
    pub messages: Vec<NormalizedMessage>,
    pub artifacts: Vec<NormalizedArtifact>,
    pub raw_records: Vec<ParsedCaptureRecord>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ImportSourceSummary {
    pub kind: String,
    pub discovered_captures: usize,
    pub imported_captures: usize,
    pub skipped_captures: usize,
    pub failed_captures: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ImportFailureEntry {
    pub source_kind: String,
    pub source_path: String,
    pub error_text: String,
}

pub trait SourceConnector {
    fn kind(&self) -> SourceKind;
    fn detect(&self) -> Result<DiscoveredSource>;
    fn discover_captures(&self) -> Result<Vec<DiscoveredCapture>>;
    fn snapshot_capture(&self, capture: &DiscoveredCapture) -> Result<CaptureSnapshot>;
    fn parse_capture(
        &self,
        capture: &DiscoveredCapture,
        snapshot: &CaptureSnapshot,
    ) -> Result<ParsedCapture>;
}
