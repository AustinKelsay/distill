//! Shared privacy and hostile-input policy for the Distill Library.
//!
//! # v1 privacy boundary (code-comment contract for issue #32)
//!
//! Distill v1 treats conversation content as local operator data with an
//! **export-only** sensitivity policy:
//! - the `sensitive` modifier label blocks standard dataset export; it is not
//!   an encryption, ACL, or deletion control
//! - v1 provides **no** application-level encryption at rest
//! - v1 provides **no** per-session delete, retention purge, or secure-forget
//! - callers must rely on OS filesystem permissions, Distill home modes
//!   (`0o700` / `0o600`), path containment, subprocess bounds, and redacted
//!   diagnostics for the documented threat model
//!
//! This module is the shared implementation seam for those hardening rules.
//! Governed docs updates for the same boundary remain a follow-up pass.

use std::fs;

use serde_json::Value;

use crate::adapter::CaptureCandidate;
use crate::error::{LibraryError, LibraryResult};

/// Maximum nesting depth accepted for provider/manifest JSON values.
pub const MAX_JSON_DEPTH: usize = 64;

/// Maximum bytes accepted for a single JSON document (manifest/export object).
pub const MAX_JSON_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;

/// Maximum bytes accepted for one JSONL line before parse.
pub const MAX_JSON_LINE_BYTES: usize = 1024 * 1024;

/**
 * Reject a file-backed candidate whose on-disk size already exceeds the Capture limit.
 *
 * Called before adapter snapshot so hostile oversized files are not fully read
 * into memory. Virtual candidates are ignored here; their in-memory size is
 * still gated by [`crate::storage::store_capture_bytes`].
 *
 * Parameters:
 * - `candidate`: Capture Candidate about to be snapshotted.
 * - `max_capture_bytes`: Configured Capture acceptance limit.
 */
pub(crate) fn reject_oversized_candidate_file(
    candidate: &CaptureCandidate,
    max_capture_bytes: u64,
) -> LibraryResult<()> {
    if candidate.is_virtual {
        return Ok(());
    }
    let Some(path) = candidate.absolute_path.as_ref() else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    // Follow only after `enforce_configured_root` has accepted the candidate.
    // Symlink escapes are rejected there; this gate only avoids reading huge
    // in-root (or already-contained) files into memory before snapshot.
    let meta = fs::metadata(path)?;
    let byte_size = meta.len();
    if byte_size > max_capture_bytes {
        return Err(LibraryError::CaptureTooLarge {
            byte_size,
            limit: max_capture_bytes,
        });
    }
    Ok(())
}

/**
 * Parse a UTF-8 JSON document under shared size and depth bounds.
 *
 * Parameters:
 * - `raw`: Exact document bytes as text.
 */
pub(crate) fn parse_json_document_bounded(raw: &str) -> Result<Value, String> {
    if raw.len() > MAX_JSON_DOCUMENT_BYTES {
        return Err(format!(
            "json document exceeds {} byte limit",
            MAX_JSON_DOCUMENT_BYTES
        ));
    }
    let value: Value =
        serde_json::from_str(raw).map_err(|err| format!("invalid json document: {err}"))?;
    ensure_json_depth(&value, MAX_JSON_DEPTH)?;
    Ok(value)
}

/**
 * Parse one JSONL line under shared size and depth bounds.
 *
 * Parameters:
 * - `line`: Trimmed JSONL line text.
 * - `line_no`: 1-based line number for typed diagnostics.
 */
pub(crate) fn parse_json_line_bounded(line: &str, line_no: usize) -> Result<Value, String> {
    if line.len() > MAX_JSON_LINE_BYTES {
        return Err(format!(
            "line {line_no}: json exceeds {} byte limit",
            MAX_JSON_LINE_BYTES
        ));
    }
    let value: Value =
        serde_json::from_str(line).map_err(|err| format!("line {line_no}: invalid json: {err}"))?;
    ensure_json_depth(&value, MAX_JSON_DEPTH).map_err(|err| format!("line {line_no}: {err}"))?;
    Ok(value)
}

/**
 * Reject JSON values deeper than `max_depth`.
 *
 * Parameters:
 * - `value`: Parsed JSON value.
 * - `max_depth`: Inclusive maximum nesting depth (objects/arrays count).
 */
pub(crate) fn ensure_json_depth(value: &Value, max_depth: usize) -> Result<(), String> {
    fn walk(value: &Value, remaining: usize) -> Result<(), String> {
        match value {
            Value::Object(map) => {
                if remaining == 0 {
                    return Err(format!("json exceeds maximum depth {MAX_JSON_DEPTH}"));
                }
                for child in map.values() {
                    walk(child, remaining - 1)?;
                }
            }
            Value::Array(items) => {
                if remaining == 0 {
                    return Err(format!("json exceeds maximum depth {MAX_JSON_DEPTH}"));
                }
                for child in items {
                    walk(child, remaining - 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(value, max_depth)
}

/**
 * Redact Activity/export payload JSON for caller-facing and durable surfaces.
 *
 * Drops path/command/provider/secret-bearing keys and replaces absolute paths,
 * SQL-looking strings, and credential-shaped values with `[redacted]`.
 *
 * Parameters:
 * - `raw`: Raw JSON text; malformed input becomes `{}`.
 */
pub(crate) fn redact_payload_json(raw: &str) -> Value {
    match serde_json::from_str::<Value>(raw) {
        Ok(value) => redact_json_value(value),
        Err(_) => Value::Object(serde_json::Map::new()),
    }
}

/**
 * Recursively redact a JSON value.
 *
 * Parameters:
 * - `value`: Parsed JSON value.
 */
pub(crate) fn redact_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                let lowered = key.to_ascii_lowercase().replace(['-', '_'], "");
                if is_redacted_payload_key(&key.to_ascii_lowercase())
                    || is_secret_bearing_key(&lowered)
                {
                    continue;
                }
                out.insert(key, redact_json_value(child));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(redact_json_value).collect()),
        Value::String(text) => {
            if looks_like_filesystem_path(&text)
                || looks_like_sql(&text)
                || looks_like_secret_value(&text)
            {
                Value::String("[redacted]".into())
            } else {
                Value::String(text)
            }
        }
        other => other,
    }
}

/**
 * Redact path-like and secret-bearing fragments from operational diagnostic text.
 *
 * Parameters:
 * - `text`: Caller-facing diagnostic string.
 */
pub(crate) fn redact_diagnostic_text(text: &str) -> String {
    if looks_like_sql(text) || looks_like_secret_value(text) {
        return "[redacted]".into();
    }

    text.split_whitespace()
        .map(|token| {
            let trimmed = token.trim_matches(|character: char| {
                matches!(
                    character,
                    '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
                )
            });
            if looks_like_diagnostic_path(trimmed) || looks_like_secret_value(trimmed) {
                "[redacted]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/**
 * Caller-safe Library error message that never includes raw paths or payloads.
 *
 * Parameters:
 * - `err`: Library failure.
 */
pub fn safe_caller_message(err: &LibraryError) -> String {
    match err {
        LibraryError::PathOutsideConfiguredRoot { .. } => {
            "path escaped the configured Source root".into()
        }
        LibraryError::CaptureTooLarge { limit, .. } => {
            format!("capture exceeds configured size limit of {limit} bytes")
        }
        LibraryError::MigrationChecksumMismatch { version } => {
            format!("migration checksum mismatch for version {version}")
        }
        LibraryError::SourceAdapter(_) => "source adapter failed".into(),
        LibraryError::Io(_) => "io failure".into(),
        LibraryError::Sqlite(_) => "sqlite failure".into(),
        LibraryError::Json(_) => "json failure".into(),
        LibraryError::ContentIntegrity { .. } => "content integrity failure".into(),
        LibraryError::StagedContentIntegrity { .. } => "staged content integrity failure".into(),
        LibraryError::NotFound(_) => "not found".into(),
        LibraryError::InvalidArgument(detail) => {
            format!("invalid argument: {}", redact_diagnostic_text(detail))
        }
        LibraryError::SyncAlreadyRunning => "sync already running".into(),
        LibraryError::SyncNoEnabledSources => "sync selection has no enabled sources".into(),
        LibraryError::SyncLeaseLost => "sync lease lost".into(),
        LibraryError::InvalidConfiguredRoot { detail } => {
            format!(
                "invalid configured root: {}",
                redact_diagnostic_text(detail)
            )
        }
        LibraryError::ProviderProcessBoundExceeded { detail } => {
            format!(
                "provider process bound exceeded: {}",
                redact_diagnostic_text(detail)
            )
        }
        #[cfg(feature = "test-faults")]
        LibraryError::InjectedTestFault { point } => {
            format!("injected test fault: {point:?}")
        }
    }
}

fn is_redacted_payload_key(key: &str) -> bool {
    matches!(
        key,
        "sql"
            | "argv"
            | "command"
            | "stderr"
            | "stdout"
            | "provider_payload"
            | "providerpayload"
            | "raw_payload"
            | "rawpayload"
    ) || key.ends_with("path")
}

fn is_secret_bearing_key(normalized_key: &str) -> bool {
    matches!(
        normalized_key,
        "token"
            | "apitoken"
            | "accesstoken"
            | "refreshtoken"
            | "authorization"
            | "auth"
            | "password"
            | "secret"
            | "apikey"
            | "privatekey"
            | "clientsecret"
            | "bearer"
    ) || normalized_key.contains("apikey")
        || normalized_key.contains("secret")
        || normalized_key.ends_with("token")
}

fn looks_like_filesystem_path(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.starts_with('/')
        || trimmed.starts_with("~/")
        || (trimmed.len() >= 3
            && trimmed.as_bytes()[0].is_ascii_alphabetic()
            && trimmed.as_bytes()[1] == b':'
            && (trimmed.as_bytes()[2] == b'\\' || trimmed.as_bytes()[2] == b'/'))
}

fn looks_like_diagnostic_path(text: &str) -> bool {
    looks_like_filesystem_path(text) || text.contains('/') || text.contains('\\')
}

fn looks_like_sql(text: &str) -> bool {
    let lowered = text.trim().to_ascii_lowercase();
    lowered.starts_with("select ")
        || lowered.starts_with("insert ")
        || lowered.starts_with("update ")
        || lowered.starts_with("delete ")
        || lowered.starts_with("pragma ")
        || lowered.starts_with("create ")
        || lowered.starts_with("alter ")
        || lowered.starts_with("drop ")
}

fn looks_like_secret_value(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lowered = trimmed.to_ascii_lowercase();
    lowered.starts_with("bearer ")
        || lowered.starts_with("sk-")
        || lowered.starts_with("xox")
        || lowered.contains("api_key=")
        || lowered.contains("apikey=")
        || lowered.contains("secret=")
}
