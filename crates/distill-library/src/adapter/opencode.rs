//! OpenCode SourceAdapter: virtual session discovery via bounded `opencode` CLI calls.
//!
//! Detection uses the configured data root. Discovery and snapshot invoke the
//! `opencode` executable through hard duration and stdout/stderr caps. Snapshot
//! preserves the complete export stdout payload (including any leading non-JSON
//! line) as Distill-owned bytes. The adapter never opens or writes SQLite.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::error::LibraryError;
use crate::ops::process::{run_bounded_command, ProviderProcessLimits};

use super::{
    find_executable, CaptureCandidate, CaptureSnapshot, DiscoveredSource, ParsedArtifact,
    ParsedCapture, ParsedFact, ParsedMessage, ParserIdentity, SourceAdapter, SourceKind,
    SourceStageError,
};

/// Default OpenCode parser identity for Normalization Attempts.
pub const OPENCODE_PARSER_ID: &str = "opencode";

/// Default OpenCode parser contract version.
pub const OPENCODE_PARSER_VERSION: &str = "1.0.0";

/// Media type for OpenCode virtual session-export Captures.
pub const OPENCODE_MEDIA_TYPE: &str = "application/x-distill-opencode+json";

/// Production discovery/export wall-clock budget.
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);

/// Production stdout cap for discovery and export payloads.
const DEFAULT_MAX_STDOUT_BYTES: usize = 16 * 1024 * 1024;

/// Production stderr cap for redacted failure classification.
const DEFAULT_MAX_STDERR_BYTES: usize = 1024 * 1024;

/// Session discovery SQL matching the legacy connector contract.
const SESSION_LIST_QUERY: &str = "SELECT id, title, directory, version, time_created, time_updated, time_archived, share_url FROM session ORDER BY time_updated ASC;";

/// OpenCode-specific alias for the shared bounded provider-process policy.
pub type OpenCodeProcessLimits = ProviderProcessLimits;

fn default_process_limits() -> OpenCodeProcessLimits {
    OpenCodeProcessLimits {
        max_duration: DEFAULT_COMMAND_TIMEOUT,
        max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
        max_stderr_bytes: DEFAULT_MAX_STDERR_BYTES,
    }
}

/// Captured bounded subprocess output.
#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundedOutput {
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Classified bounded-command failure without paths or provider payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandFailureKind {
    ExecutableNotFound,
    TimedOut,
    OutputOverflow,
    Failed,
}

/// OpenCode SourceAdapter bound to one configured data root.
pub struct OpenCodeAdapter {
    root: PathBuf,
    parser: ParserIdentity,
    limits: OpenCodeProcessLimits,
}

impl OpenCodeAdapter {
    /// Create an adapter that detects only the supplied OpenCode data root.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_parser(
            root,
            ParserIdentity {
                id: OPENCODE_PARSER_ID.to_string(),
                version: OPENCODE_PARSER_VERSION.to_string(),
            },
        )
    }

    /**
     * Create an adapter with an explicit OpenCode parser identity.
     *
     * Parameters:
     * - `root`: Configured OpenCode data root.
     * - `parser`: Parser identity/version recorded on Normalization Attempts.
     */
    pub fn with_parser(root: impl Into<PathBuf>, parser: ParserIdentity) -> Self {
        Self {
            root: root.into(),
            parser,
            limits: process_limits_from_env(default_process_limits()),
        }
    }

    /**
     * Create an adapter with explicit process bounds (tests and harnesses).
     *
     * Parameters:
     * - `root`: Configured OpenCode data root.
     * - `limits`: Duration and stdout/stderr caps for CLI helpers.
     */
    #[allow(dead_code)]
    pub fn with_limits(root: impl Into<PathBuf>, limits: OpenCodeProcessLimits) -> Self {
        Self {
            root: root.into(),
            parser: ParserIdentity {
                id: OPENCODE_PARSER_ID.to_string(),
                version: OPENCODE_PARSER_VERSION.to_string(),
            },
            limits: process_limits_from_env(limits),
        }
    }
}

impl SourceAdapter for OpenCodeAdapter {
    fn detect(&self) -> Result<DiscoveredSource, SourceStageError> {
        let root = canonicalize_existing(&self.root).map_err(|_| {
            SourceStageError::Detect("configured root is missing or inaccessible".into())
        })?;
        if !root.is_dir() {
            return Err(SourceStageError::Detect(
                "configured root is not a directory".into(),
            ));
        }
        Ok(DiscoveredSource {
            kind: SourceKind::OpenCode,
            display_name: "OpenCode".to_string(),
            data_root: root,
            parser: self.parser.clone(),
        })
    }

    fn discover(
        &self,
        source: &DiscoveredSource,
    ) -> Result<Vec<CaptureCandidate>, SourceStageError> {
        let executable = resolve_executable(&source.data_root)
            .map_err(|kind| stage_command_error(SourceStageError::Discover, kind))?;
        let output = run_opencode_command(
            &executable,
            &["db", SESSION_LIST_QUERY, "--format", "json"],
            self.limits,
        )
        .map_err(|kind| stage_command_error(SourceStageError::Discover, kind))?;

        if output.exit_code != Some(0) {
            if is_no_rows_message(&output.stderr) || is_no_rows_message(&output.stdout) {
                return Ok(Vec::new());
            }
            return Err(SourceStageError::Discover("command failed".into()));
        }

        let rows = parse_session_rows(&output.stdout)?;
        let mut candidates = Vec::with_capacity(rows.len());
        for row in rows {
            let session_id = match text_value(row.get("id")) {
                Some(id) => id,
                None => continue,
            };
            let title = text_value(row.get("title"));
            let source_modified_at = timestamp_to_iso(row.get("time_updated"));
            let discovery_meta = json!({
                "title": title,
                "directory": text_value(row.get("directory")),
                "version": text_value(row.get("version")),
                "time_created": row.get("time_created").cloned().unwrap_or(Value::Null),
                "time_updated": row.get("time_updated").cloned().unwrap_or(Value::Null),
                "time_archived": row.get("time_archived").cloned().unwrap_or(Value::Null),
                "share_url": text_value(row.get("share_url")),
                "source_modified_at": source_modified_at,
            });
            candidates.push(CaptureCandidate {
                source_kind: SourceKind::OpenCode,
                source_path: format!("opencode://session/{session_id}"),
                absolute_path: None,
                external_session_id: Some(session_id),
                title,
                is_virtual: true,
                virtual_bytes: Some(discovery_meta.to_string().into_bytes()),
                media_type: OPENCODE_MEDIA_TYPE.to_string(),
            });
        }
        candidates.sort_by(|left, right| left.source_path.cmp(&right.source_path));
        Ok(candidates)
    }

    fn snapshot(&self, candidate: &CaptureCandidate) -> Result<CaptureSnapshot, SourceStageError> {
        let session_id = candidate
            .external_session_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| SourceStageError::Snapshot("missing session identity".into()))?;
        let executable = resolve_executable(&self.root)
            .map_err(|kind| stage_command_error(SourceStageError::Snapshot, kind))?;
        let output = run_opencode_command(&executable, &["export", session_id], self.limits)
            .map_err(|kind| stage_command_error(SourceStageError::Snapshot, kind))?;
        if output.exit_code != Some(0) {
            return Err(SourceStageError::Snapshot("command failed".into()));
        }
        // Preserve the complete stdout payload, including any leading non-JSON line.
        let bytes = output.stdout;
        if extract_json_bytes(&bytes).is_err() {
            return Err(SourceStageError::Snapshot(
                "malformed export payload".into(),
            ));
        }
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let source_modified_at = candidate
            .virtual_bytes
            .as_ref()
            .and_then(|meta| serde_json::from_slice::<Value>(meta).ok())
            .and_then(|value| {
                value
                    .get("source_modified_at")
                    .and_then(Value::as_str)
                    .map(str::to_string)
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
        parse_opencode_export(candidate, &snapshot.bytes)
    }
}

/**
 * Resolve the OpenCode executable without leaking absolute paths into errors.
 *
 * Prefers `{root}/bin/opencode` so hermetic tests can install a fake beside the
 * configured root, then falls back to PATH.
 */
fn resolve_executable(root: &Path) -> Result<PathBuf, CommandFailureKind> {
    let local = root.join("bin").join("opencode");
    if local.is_file() {
        return Ok(local);
    }
    find_executable("opencode").ok_or(CommandFailureKind::ExecutableNotFound)
}

/**
 * Run `opencode` with hard duration and output caps.
 *
 * Parameters:
 * - `executable`: Resolved opencode binary path.
 * - `args`: CLI arguments after the executable.
 * - `limits`: Duration and byte caps.
 */
fn run_opencode_command(
    executable: &Path,
    args: &[&str],
    limits: OpenCodeProcessLimits,
) -> Result<BoundedOutput, CommandFailureKind> {
    let mut command = Command::new(executable);
    command.args(args);
    let process_limits = ProviderProcessLimits {
        max_duration: limits.max_duration,
        max_stdout_bytes: limits.max_stdout_bytes,
        max_stderr_bytes: limits.max_stderr_bytes,
    };
    match run_bounded_command(command, process_limits, None) {
        Ok(output) => Ok(BoundedOutput {
            exit_code: output.exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
        }),
        Err(LibraryError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            Err(CommandFailureKind::ExecutableNotFound)
        }
        Err(LibraryError::ProviderProcessBoundExceeded { detail }) => {
            if detail.contains("duration") {
                Err(CommandFailureKind::TimedOut)
            } else {
                Err(CommandFailureKind::OutputOverflow)
            }
        }
        Err(_) => Err(CommandFailureKind::Failed),
    }
}

fn stage_command_error<F>(stage: F, kind: CommandFailureKind) -> SourceStageError
where
    F: FnOnce(String) -> SourceStageError,
{
    let message = match kind {
        CommandFailureKind::ExecutableNotFound => "executable not found",
        CommandFailureKind::TimedOut => "command timed out",
        CommandFailureKind::OutputOverflow => "command output exceeded limit",
        CommandFailureKind::Failed => "command failed",
    };
    stage(message.into())
}

/**
 * Allow integration tests to tighten bounds without new public Library APIs.
 *
 * `DISTILL_TEST_OPENCODE_TIMEOUT_MS` and `DISTILL_TEST_OPENCODE_MAX_STDOUT_BYTES`
 * override the supplied defaults when present and valid.
 */
fn process_limits_from_env(mut limits: OpenCodeProcessLimits) -> OpenCodeProcessLimits {
    if let Ok(ms) = std::env::var("DISTILL_TEST_OPENCODE_TIMEOUT_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            limits.max_duration = Duration::from_millis(ms.max(1));
        }
    }
    if let Ok(bytes) = std::env::var("DISTILL_TEST_OPENCODE_MAX_STDOUT_BYTES") {
        if let Ok(bytes) = bytes.parse::<usize>() {
            limits.max_stdout_bytes = bytes.max(1);
        }
    }
    limits
}

fn parse_session_rows(stdout: &[u8]) -> Result<Vec<Map<String, Value>>, SourceStageError> {
    let trimmed = trim_utf8(stdout)
        .map_err(|_| SourceStageError::Discover("invalid discovery output".into()))?;
    if trimmed.is_empty() || trimmed == "null" {
        return Ok(Vec::new());
    }
    let value = crate::privacy::parse_json_document_bounded(trimmed)
        .map_err(|_| SourceStageError::Discover("invalid discovery output".into()))?;
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => {
            let mut rows = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::Object(map) => rows.push(map),
                    _ => {
                        return Err(SourceStageError::Discover(
                            "invalid discovery output".into(),
                        ))
                    }
                }
            }
            Ok(rows)
        }
        _ => Err(SourceStageError::Discover(
            "invalid discovery output".into(),
        )),
    }
}

/**
 * Parse OpenCode export bytes into Capture Facts, Messages, and Artifacts.
 *
 * Parameters:
 * - `candidate`: Capture Candidate providing identity and discovery metadata.
 * - `bytes`: Exact Distill-owned snapshot bytes (complete export stdout).
 */
fn parse_opencode_export(
    candidate: &CaptureCandidate,
    bytes: &[u8],
) -> Result<ParsedCapture, SourceStageError> {
    let json_bytes = extract_json_bytes(bytes)?;
    let export_text = std::str::from_utf8(json_bytes)
        .map_err(|_| SourceStageError::Parse("malformed export payload".into()))?;
    let export = crate::privacy::parse_json_document_bounded(export_text)
        .map_err(|_| SourceStageError::Parse("malformed export payload".into()))?;
    let session_info = export
        .get("info")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let export_messages = export
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let discovery = candidate
        .virtual_bytes
        .as_ref()
        .and_then(|meta| serde_json::from_slice::<Value>(meta).ok())
        .unwrap_or(Value::Null);

    let mut facts = Vec::new();
    let mut messages = Vec::new();
    let mut artifacts = Vec::new();
    let (model, model_provider) = normalize_model_info(&export_messages);

    for export_message in &export_messages {
        let info = export_message
            .get("info")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let message_role = text_value(info.get("role")).unwrap_or_else(|| "assistant".into());
        let parent_id = text_value(info.get("parentID"));
        let parts = export_message
            .get("parts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        for part_value in parts {
            let part = match part_value.as_object() {
                Some(map) => map.clone(),
                None => continue,
            };
            let part_type = text_value(part.get("type")).unwrap_or_else(|| "unknown".into());
            let part_id = text_value(part.get("id"));
            let text = build_meta_message_text(&part, &message_role);
            let role = normalize_part_role(&part_type, &message_role);
            let is_meta = part_type != "text";
            let fact_ordinal = facts.len();
            let content_json = json!({
                "info": Value::Object(info.clone()),
                "part": Value::Object(part.clone()),
                "parent_id": parent_id,
            });
            facts.push(ParsedFact {
                record_type: format!("message:{message_role}:{part_type}"),
                role: Some(role.clone()),
                is_meta,
                content_text: text.clone(),
                content_json: content_json.clone(),
            });

            let message_ordinal = if let Some(text) = text.clone() {
                let ordinal = messages.len();
                messages.push(ParsedMessage {
                    role: role.clone(),
                    message_kind: if part_type == "text" {
                        "text".into()
                    } else {
                        "meta".into()
                    },
                    text,
                    external_message_id: part_id.clone(),
                });
                Some(ordinal)
            } else {
                None
            };

            if part_type == "tool" {
                push_tool_artifacts(
                    &mut artifacts,
                    &part,
                    part_id.as_deref(),
                    message_ordinal,
                    fact_ordinal,
                );
            } else if part_type == "file" {
                artifacts.push(ParsedArtifact {
                    artifact_type: "file".into(),
                    message_ordinal,
                    fact_ordinal: Some(fact_ordinal),
                    text_preview: text,
                    content_json: Value::Object(part),
                });
            } else if !matches!(
                part_type.as_str(),
                "text" | "reasoning" | "step-start" | "step-finish"
            ) {
                artifacts.push(ParsedArtifact {
                    artifact_type: "raw_json".into(),
                    message_ordinal,
                    fact_ordinal: Some(fact_ordinal),
                    text_preview: text,
                    content_json: Value::Object(part),
                });
            }
        }
    }

    let generated_title = text_value(session_info.get("title"));
    let fallback_title = first_user_text(&messages);
    let title = match generated_title.as_deref() {
        Some(value) if !is_generated_title(value) => Some(value.to_string()),
        _ => fallback_title.or(candidate.title.clone()),
    };

    let source_session_id = text_value(session_info.get("id"))
        .or_else(|| candidate.external_session_id.clone())
        .filter(|value| !value.is_empty());
    let synthetic_identity = source_session_id.is_none();
    let external_session_id = source_session_id.unwrap_or_else(|| synthetic_session_id(candidate));

    let provenance = if synthetic_identity {
        json!({ "kind": "synthetic", "strategy": "source_path_sha256" })
    } else {
        json!({ "kind": "source" })
    };

    let mut metadata = Map::new();
    metadata.insert("external_session_id_provenance".into(), provenance);
    if synthetic_identity {
        metadata.insert("synthetic_identity".into(), json!(true));
        metadata.insert("source_path".into(), json!(candidate.source_path.clone()));
    }
    if let Some(model) = model.clone() {
        metadata.insert("model".into(), json!(model));
    }
    if let Some(model_provider) = model_provider.clone() {
        metadata.insert("model_provider".into(), json!(model_provider));
    }
    if let Some(version) = text_value(session_info.get("version")) {
        metadata.insert("cli_version".into(), json!(version));
    }
    if let Some(slug) = text_value(session_info.get("slug")) {
        metadata.insert("slug".into(), json!(slug));
    }
    if let Some(project_id) = text_value(session_info.get("projectID")) {
        metadata.insert("project_id".into(), json!(project_id));
    }
    metadata.insert("export_source".into(), json!("opencode export"));
    if let Some(original) = generated_title.clone() {
        if title.as_deref() != Some(original.as_str()) {
            metadata.insert("original_title".into(), json!(original));
        }
    }

    let started_at = session_info
        .get("time")
        .and_then(Value::as_object)
        .and_then(|time| timestamp_to_iso(time.get("created")));
    let updated_at = session_info
        .get("time")
        .and_then(Value::as_object)
        .and_then(|time| timestamp_to_iso(time.get("updated")))
        .or_else(|| timestamp_to_iso(discovery.get("time_updated")))
        .or_else(|| {
            discovery
                .get("source_modified_at")
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    Ok(ParsedCapture {
        external_session_id,
        synthetic_identity,
        title,
        summary: None,
        project_path: text_value(session_info.get("directory"))
            .or_else(|| text_value(discovery.get("directory"))),
        source_url: text_value(discovery.get("share_url")),
        started_at,
        updated_at,
        metadata: Value::Object(metadata),
        facts,
        messages,
        artifacts,
    })
}

fn extract_json_bytes(bytes: &[u8]) -> Result<&[u8], SourceStageError> {
    let start = bytes
        .iter()
        .position(|byte| *byte == b'{')
        .ok_or_else(|| SourceStageError::Parse("malformed export payload".into()))?;
    Ok(&bytes[start..])
}

fn push_tool_artifacts(
    artifacts: &mut Vec<ParsedArtifact>,
    part: &Map<String, Value>,
    _part_id: Option<&str>,
    message_ordinal: Option<usize>,
    fact_ordinal: usize,
) {
    let state = part
        .get("state")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let name = text_value(part.get("tool")).unwrap_or_else(|| "unknown".into());
    let call_id = text_value(part.get("callID"));
    let payload = json!({
        "name": name,
        "callId": call_id,
        "state": Value::Object(state.clone()),
        "metadata": part.get("metadata").cloned().unwrap_or(Value::Null),
    });
    artifacts.push(ParsedArtifact {
        artifact_type: "tool_call".into(),
        message_ordinal,
        fact_ordinal: Some(fact_ordinal),
        text_preview: Some(format!("[tool] {name}")),
        content_json: payload,
    });

    let status = text_value(state.get("status"));
    if matches!(status.as_deref(), Some("completed") | Some("error")) {
        artifacts.push(ParsedArtifact {
            artifact_type: "tool_result".into(),
            message_ordinal,
            fact_ordinal: Some(fact_ordinal),
            text_preview: status.clone(),
            content_json: json!({
                "name": name,
                "status": status,
                "output": state.get("output").cloned().unwrap_or(Value::Null),
                "error": state.get("error").cloned().unwrap_or(Value::Null),
                "title": state.get("title").cloned().unwrap_or(Value::Null),
                "attachments": state.get("attachments").cloned().unwrap_or(Value::Null),
            }),
        });
        if let Some(attachments) = state.get("attachments").and_then(Value::as_array) {
            for attachment in attachments {
                artifacts.push(ParsedArtifact {
                    artifact_type: "file".into(),
                    message_ordinal,
                    fact_ordinal: Some(fact_ordinal),
                    text_preview: text_value(
                        attachment.as_object().and_then(|map| map.get("filename")),
                    ),
                    content_json: attachment.clone(),
                });
            }
        }
    }
}

fn build_meta_message_text(part: &Map<String, Value>, message_role: &str) -> Option<String> {
    let part_type = text_value(part.get("type")).unwrap_or_else(|| "unknown".into());
    match part_type.as_str() {
        "text" | "reasoning" => text_value(part.get("text")),
        "step-start" => Some("[step-start]".into()),
        "step-finish" => {
            let tokens = part
                .get("tokens")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let input = tokens.get("input").and_then(Value::as_i64).unwrap_or(0);
            let output = tokens.get("output").and_then(Value::as_i64).unwrap_or(0);
            let reason = text_value(part.get("reason")).unwrap_or_else(|| "unknown".into());
            Some(format!(
                "[step-finish] reason={reason} input={input} output={output}"
            ))
        }
        "tool" => Some(build_tool_message_text(part)),
        "file" => {
            let file_name = text_value(part.get("filename"));
            let source_path = part
                .get("source")
                .and_then(Value::as_object)
                .and_then(|source| text_value(source.get("path")));
            Some(format!(
                "[file] {}",
                file_name
                    .or(source_path)
                    .or_else(|| text_value(part.get("url")))
                    .unwrap_or_else(|| "attachment".into())
            ))
        }
        "subtask" => Some(format!(
            "[subtask] {}",
            text_value(part.get("description"))
                .or_else(|| text_value(part.get("prompt")))
                .unwrap_or_else(|| "subtask".into())
        )),
        "agent" => Some(format!(
            "[agent] {}",
            text_value(part.get("name")).unwrap_or_else(|| "agent".into())
        )),
        "patch" => {
            let files = part
                .get("files")
                .and_then(Value::as_array)
                .map(|files| files.len())
                .unwrap_or(0);
            Some(format!("[patch] {files} files"))
        }
        "snapshot" => Some("[snapshot]".into()),
        "retry" => {
            let attempt = part.get("attempt").and_then(Value::as_i64).unwrap_or(0);
            let error_text = part
                .get("error")
                .and_then(Value::as_object)
                .and_then(|error| text_value(error.get("message")))
                .map(|text| format!(" {}", trim_text(&text, 120)))
                .unwrap_or_default();
            Some(format!("[retry] attempt {attempt}{error_text}"))
        }
        "compaction" => Some(format!(
            "[compaction] {}",
            if part.get("auto").and_then(Value::as_bool) == Some(true) {
                "auto"
            } else {
                "manual"
            }
        )),
        other => Some(format!("[{other}] {message_role}")),
    }
}

fn build_tool_message_text(part: &Map<String, Value>) -> String {
    let tool_name = text_value(part.get("tool")).unwrap_or_else(|| "unknown".into());
    let state = part
        .get("state")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let status = text_value(state.get("status")).unwrap_or_else(|| "pending".into());
    match status.as_str() {
        "completed" => {
            let title = text_value(state.get("title"));
            let output = text_value(state.get("output")).map(|text| trim_text(&text, 400));
            let body = [title, output]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n");
            if body.is_empty() {
                format!("[tool:completed] {tool_name}")
            } else {
                format!("[tool:completed] {tool_name}\n{body}")
            }
        }
        "error" => {
            let error_text = text_value(state.get("error")).map(|text| trim_text(&text, 400));
            match error_text {
                Some(text) => format!("[tool:error] {tool_name}\n{text}"),
                None => format!("[tool:error] {tool_name}"),
            }
        }
        other => format!("[tool:{other}] {tool_name}"),
    }
}

fn normalize_part_role(part_type: &str, message_role: &str) -> String {
    if part_type == "tool" {
        return "tool".into();
    }
    match message_role {
        "user" | "assistant" | "system" => message_role.into(),
        _ => "assistant".into(),
    }
}

fn normalize_model_info(messages: &[Value]) -> (Option<String>, Option<String>) {
    let mut first_user: Option<(Option<String>, Option<String>)> = None;
    let mut last_assistant: Option<(Option<String>, Option<String>)> = None;
    for message in messages {
        let info = match message.get("info").and_then(Value::as_object) {
            Some(info) => info,
            None => continue,
        };
        let role = text_value(info.get("role"));
        if role.as_deref() == Some("user") && first_user.is_none() {
            let model = info
                .get("model")
                .and_then(Value::as_object)
                .map(|model| {
                    (
                        text_value(model.get("modelID")),
                        text_value(model.get("providerID")),
                    )
                })
                .unwrap_or((None, None));
            if model.0.is_some() || model.1.is_some() {
                first_user = Some(model);
            }
        }
        if role.as_deref() == Some("assistant") {
            let model = (
                text_value(info.get("modelID")),
                text_value(info.get("providerID")),
            );
            if model.0.is_some() || model.1.is_some() {
                last_assistant = Some(model);
            }
        }
    }
    let chosen = last_assistant.or(first_user);
    match chosen {
        Some((model, provider)) => (model, provider),
        None => (None, None),
    }
}

fn first_user_text(messages: &[ParsedMessage]) -> Option<String> {
    messages
        .iter()
        .find(|message| message.role == "user" && message.message_kind == "text")
        .map(|message| {
            message
                .text
                .lines()
                .next()
                .unwrap_or(message.text.as_str())
                .trim()
                .chars()
                .take(160)
                .collect()
        })
        .filter(|value: &String| !value.is_empty())
}

fn is_generated_title(title: &str) -> bool {
    title.starts_with("New session - ")
        && title
            .get("New session - ".len()..)
            .is_some_and(|rest| rest.contains('T'))
}

fn synthetic_session_id(candidate: &CaptureCandidate) -> String {
    let digest = Sha256::digest(candidate.source_path.as_bytes());
    format!("synthetic-{}", &hex::encode(digest)[..16])
}

fn timestamp_to_iso(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let millis = match value {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    if !millis.is_finite() {
        return None;
    }
    chrono::DateTime::from_timestamp_millis(millis as i64)
        .map(|datetime| datetime.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn text_value(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn trim_text(text: &str, max_length: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_length {
        return normalized;
    }
    let mut truncated: String = normalized
        .chars()
        .take(max_length.saturating_sub(1))
        .collect();
    while truncated.ends_with(char::is_whitespace) {
        truncated.pop();
    }
    truncated.push('…');
    truncated
}

fn trim_utf8(bytes: &[u8]) -> Result<&str, ()> {
    std::str::from_utf8(bytes).map(str::trim).map_err(|_| ())
}

fn is_no_rows_message(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    lower.contains("no row") || lower.contains("no data")
}

fn canonicalize_existing(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(external_session_id: Option<&str>) -> CaptureCandidate {
        CaptureCandidate {
            source_kind: SourceKind::OpenCode,
            source_path: "opencode://session/test-session".into(),
            absolute_path: None,
            external_session_id: external_session_id.map(str::to_string),
            title: None,
            is_virtual: true,
            virtual_bytes: None,
            media_type: OPENCODE_MEDIA_TYPE.into(),
        }
    }

    #[test]
    fn parse_preserves_metadata_unknown_roles_and_structured_artifacts() {
        let body = br#"{"info":{"title":"New session - 2026-03-26T19:15:49.354Z","directory":"/tmp/project","time":{"created":1774543194067,"updated":1774543475213}},"messages":[{"info":{"role":"user","model":{"providerID":"ollama","modelID":"nemotron"}},"parts":[{"type":"text","text":"first question"}]},{"info":{"role":"mystery"},"parts":[{"type":"reasoning","text":"hidden reasoning"},{"type":"tool","tool":"search","state":{"status":"completed","output":"done","attachments":[{"filename":"result.txt"}]}},{"type":"file","filename":"input.txt"},{"type":"custom","value":true}]}]}"#;
        let parsed = parse_opencode_export(&candidate(Some("ses_1")), body).expect("parse");

        assert_eq!(parsed.external_session_id, "ses_1");
        assert_eq!(parsed.title.as_deref(), Some("first question"));
        assert_eq!(parsed.project_path.as_deref(), Some("/tmp/project"));
        assert_eq!(
            parsed.started_at.as_deref(),
            Some("2026-03-26T16:39:54.067Z")
        );
        assert_eq!(
            parsed.updated_at.as_deref(),
            Some("2026-03-26T16:44:35.213Z")
        );
        assert!(parsed
            .metadata
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|model| model == "nemotron"));
        assert!(parsed
            .facts
            .iter()
            .any(|fact| fact.content_json["info"]["role"] == "mystery"));
        assert!(parsed
            .artifacts
            .iter()
            .any(|artifact| artifact.artifact_type == "tool_call"));
        assert!(parsed
            .artifacts
            .iter()
            .any(|artifact| artifact.artifact_type == "tool_result"));
        assert!(parsed
            .artifacts
            .iter()
            .any(|artifact| artifact.artifact_type == "file"));
        assert!(parsed
            .artifacts
            .iter()
            .any(|artifact| artifact.artifact_type == "raw_json"));
    }

    #[test]
    fn parse_synthesizes_deterministic_identity_and_rejects_invalid_timestamp() {
        let body = br#"{"info":{"time":{"created":999999999999999999999999}},"messages":[]}"#;
        let candidate = candidate(None);
        let first = parse_opencode_export(&candidate, body).expect("parse");
        let second = parse_opencode_export(&candidate, body).expect("parse again");

        assert!(first.synthetic_identity);
        assert!(first.external_session_id.starts_with("synthetic-"));
        assert_eq!(first.external_session_id, second.external_session_id);
        assert!(first.started_at.is_none());
        assert_eq!(
            first.metadata["external_session_id_provenance"]["kind"],
            "synthetic"
        );
    }

    #[test]
    fn malformed_export_and_missing_identity_are_typed() {
        let candidate = candidate(Some("ses_1"));
        assert!(matches!(
            parse_opencode_export(&candidate, b"not-json"),
            Err(SourceStageError::Parse(_))
        ));
        assert!(matches!(
            run_opencode_command(
                Path::new("/tmp/distill-opencode-missing-executable"),
                &["export", "ses_1"],
                OpenCodeProcessLimits {
                    max_duration: Duration::from_millis(10),
                    max_stdout_bytes: 32,
                    max_stderr_bytes: 32,
                },
            ),
            Err(CommandFailureKind::ExecutableNotFound)
        ));
    }
}
