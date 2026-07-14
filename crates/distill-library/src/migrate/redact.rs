//! Redaction helpers for legacy import Activity payloads and reports.

use serde_json::Value;

use crate::privacy::redact_payload_json;

/**
 * Redact a legacy Activity payload for durable destination storage.
 *
 * Drops path-bearing keys, SQL/command streams, provider/raw payload bodies,
 * and secret-bearing keys/values. Absolute filesystem path strings and
 * SQL-looking strings become `[redacted]`.
 */
pub fn redact_legacy_activity_payload(raw: &str) -> Value {
    redact_payload_json(raw)
}
