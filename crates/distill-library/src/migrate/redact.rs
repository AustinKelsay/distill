//! Redaction helpers for legacy import Activity payloads and reports.

use serde_json::Value;

/**
 * Redact a legacy Activity payload for durable destination storage.
 *
 * Drops path-bearing keys, SQL/command streams, and provider/raw payload bodies.
 * Absolute filesystem path strings and SQL-looking strings become `[redacted]`.
 */
pub fn redact_legacy_activity_payload(raw: &str) -> Value {
    match serde_json::from_str::<Value>(raw) {
        Ok(value) => redact_json_value(value),
        Err(_) => Value::Object(serde_json::Map::new()),
    }
}

fn redact_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                let lowered = key.to_ascii_lowercase();
                if is_redacted_payload_key(&lowered) {
                    continue;
                }
                out.insert(key, redact_json_value(child));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(redact_json_value).collect()),
        Value::String(text) => {
            if looks_like_filesystem_path(&text) || looks_like_sql(&text) {
                Value::String("[redacted]".into())
            } else {
                Value::String(text)
            }
        }
        other => other,
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
