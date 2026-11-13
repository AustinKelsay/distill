//! Helpers for the content-block shapes most harnesses share.

use serde_json::Value;

use crate::model::{Event, EventKind, Parsed};

pub fn ts(v: &Value) -> Option<String> {
    match v.get("timestamp")? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => n.as_i64().map(millis_to_rfc3339),
        _ => None,
    }
}

pub fn millis_to_rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

/// Flatten a tool result's content (string, or blocks of text and images) to text.
pub fn result_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|b| match b.get("type").and_then(Value::as_str) {
                Some("text") | Some("input_text") | Some("output_text") => b
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                Some("image") | Some("input_image") => "[image]".to_string(),
                _ => b.as_str().map(str::to_string).unwrap_or_default(),
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Push events for one message's content, which may be a string or an array of blocks in the
/// Anthropic style (text, thinking, tool_use, tool_result) or close variants (toolCall, image).
pub fn push_content(
    out: &mut Parsed,
    role: EventKind,
    content: &Value,
    ts: Option<String>,
    model: Option<&str>,
) {
    let with_model = |mut e: Event| {
        if role == EventKind::Assistant || e.kind == EventKind::ToolCall {
            e.model = model.map(str::to_string);
        }
        e
    };
    match content {
        Value::String(s) => out.push(with_model(Event::text(role, s.clone()).at(ts))),
        Value::Array(blocks) => {
            for b in blocks {
                let ty = b.get("type").and_then(Value::as_str).unwrap_or("");
                let e = match ty {
                    "text" | "input_text" | "output_text" => {
                        let t = b.get("text").and_then(Value::as_str).unwrap_or("");
                        Event::text(role, t)
                    }
                    "thinking" | "reasoning" => {
                        let t = b
                            .get("thinking")
                            .or_else(|| b.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        Event::text(EventKind::Reasoning, t)
                    }
                    "redacted_thinking" => Event::text(EventKind::Reasoning, "[redacted]"),
                    "tool_use" | "toolCall" | "tool_call" | "server_tool_use" => {
                        let mut e = Event::new(EventKind::ToolCall);
                        e.tool = b.get("name").and_then(Value::as_str).map(str::to_string);
                        e.call_id = b.get("id").and_then(Value::as_str).map(str::to_string);
                        e.input = b.get("input").or_else(|| b.get("arguments")).cloned();
                        e
                    }
                    "tool_result" | "toolResult" => {
                        let mut e = Event::new(EventKind::ToolResult);
                        e.call_id = b
                            .get("tool_use_id")
                            .or_else(|| b.get("toolCallId"))
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        e.text = Some(result_text(b.get("content").unwrap_or(&Value::Null)));
                        e.is_error = b.get("is_error").and_then(Value::as_bool);
                        e
                    }
                    "image" | "input_image" => Event::text(role, "[image]"),
                    _ => continue,
                };
                out.push(with_model(e.at(ts.clone())));
            }
        }
        _ => {}
    }
}
