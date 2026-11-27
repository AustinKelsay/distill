//! Helpers for the content-block shapes most harnesses share.

use std::borrow::Cow;
use std::io::BufRead;

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::model::{Event, EventKind, Parsed, TokenUsage};

/// Fallback label when a harness records a context-compaction boundary with no summary text.
pub const COMPACTION_MARKER: &str = "[context compacted]";

/// Call `f` on each line of a stream, as `str::lines` would on the whole text (invalid UTF-8
/// becomes U+FFFD), holding one line at a time.
pub fn each_line(mut reader: impl BufRead, mut f: impl FnMut(&str)) -> Result<()> {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf)? == 0 {
            return Ok(());
        }
        if buf.last() == Some(&b'\n') {
            buf.pop();
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
        }
        f(&String::from_utf8_lossy(&buf));
    }
}

/// Whether a JSONL record's top-level `type` is one of `ignored`, read without building the
/// record's tree (a record can be megabytes). A line it cannot read this way is not ignored:
/// the caller parses it in full.
pub fn is_ignored_record(line: &str, ignored: &[&str]) -> bool {
    #[derive(Deserialize)]
    struct Head<'a> {
        #[serde(borrow)]
        r#type: Option<Cow<'a, str>>,
    }
    serde_json::from_str::<Head>(line)
        .ok()
        .and_then(|h| h.r#type)
        .is_some_and(|t| ignored.contains(&&*t))
}

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
                    _ => {
                        out.skip_in("block", ty, b);
                        continue;
                    }
                };
                out.push(with_model(e.at(ts.clone())));
            }
        }
        _ => {}
    }
}

/// Normalize a harness's stop/finish reason to a short lowercase token (`end_turn`, `tool_use`,
/// `max_tokens`, `aborted`, or a lowercase snake_case fallback).
pub fn normalize_stop_reason(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut snake = String::with_capacity(trimmed.len());
    let mut prev_sep = true;
    let mut prev_lower = false;
    for ch in trimmed.chars() {
        if ch == '-' || ch == '_' || ch.is_whitespace() {
            if !prev_sep {
                snake.push('_');
                prev_sep = true;
            }
            prev_lower = false;
        } else if ch.is_ascii_uppercase() {
            if prev_lower && !prev_sep {
                snake.push('_');
            }
            snake.push(ch.to_ascii_lowercase());
            prev_sep = false;
            prev_lower = false;
        } else {
            snake.push(ch.to_ascii_lowercase());
            prev_sep = false;
            prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        }
    }
    while snake.ends_with('_') {
        snake.pop();
    }
    if snake.is_empty() {
        return None;
    }
    let canonical = match snake.as_str() {
        "end_turn" | "stop" => "end_turn",
        "tool_use" | "tool_calls" | "tool_call" | "function_call" => "tool_use",
        "max_tokens" | "length" | "max_output_tokens" => "max_tokens",
        "aborted" | "abort" | "cancelled" | "canceled" | "interrupted" => "aborted",
        other => other,
    };
    Some(canonical.to_string())
}

/// Extract token usage from a harness's usage/tokens JSON object without inventing missing fields.
pub fn parse_usage(v: &Value) -> Option<TokenUsage> {
    if !v.is_object() {
        return None;
    }
    let u64_of = |keys: &[&str]| keys.iter().find_map(|k| v.get(*k).and_then(Value::as_u64));
    let input = u64_of(&[
        "input_tokens",
        "inputTokens",
        "prompt_tokens",
        "promptTokens",
        "input",
    ]);
    let output = u64_of(&[
        "output_tokens",
        "outputTokens",
        "completion_tokens",
        "completionTokens",
        "output",
    ]);
    let cache_read = u64_of(&[
        "cache_read_input_tokens",
        "cacheReadInputTokens",
        "cached_input_tokens",
        "cachedInputTokens",
        "cache_read_tokens",
        "cacheReadTokens",
        "cacheRead",
        "cache_read",
    ])
    .or_else(|| v.pointer("/cache/read").and_then(Value::as_u64))
    .or_else(|| {
        v.pointer("/input_tokens_details/cached_tokens")
            .and_then(Value::as_u64)
    })
    .or_else(|| {
        v.pointer("/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_u64)
    });
    let cache_write = u64_of(&[
        "cache_creation_input_tokens",
        "cacheCreationInputTokens",
        "cache_write_input_tokens",
        "cacheWriteInputTokens",
        "cache_write_tokens",
        "cacheWriteTokens",
        "cacheWrite",
        "cache_write",
    ])
    .or_else(|| v.pointer("/cache/write").and_then(Value::as_u64));
    TokenUsage {
        input,
        output,
        cache_read,
        cache_write,
    }
    .non_empty()
}

/// Emit a context-compaction seam as a `System` event, using [`COMPACTION_MARKER`] when the
/// harness recorded no summary text.
pub fn push_compaction(out: &mut Parsed, summary: Option<&str>, ts: Option<String>) {
    let text = summary
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(COMPACTION_MARKER);
    let mut e = Event::text(EventKind::System, text).at(ts);
    e.compaction = Some(true);
    out.push(e);
}

/// Attach per-turn `stop_reason` and `usage` to the last event emitted since `start_len` so a
/// multi-block assistant turn does not duplicate token counts across every block.
pub fn attach_stop_and_usage(
    out: &mut Parsed,
    start_len: usize,
    stop_reason: Option<String>,
    usage: Option<TokenUsage>,
) -> Option<usize> {
    if out.events.len() <= start_len {
        return None;
    }
    let idx = out.events.len() - 1;
    if let Some(reason) = stop_reason {
        out.events[idx].stop_reason = Some(reason);
    }
    if let Some(u) = usage {
        out.events[idx].usage = Some(u);
    }
    Some(idx)
}

/// The first of `keys` in `v` that holds non-blank text.
pub fn first_text<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .filter_map(|k| v.get(*k).and_then(Value::as_str))
        .find(|s| !s.trim().is_empty())
}

/// One plan step as a line: `[done] step`, `[in progress] step`, `[pending] step`. A status
/// the harness spells another way is kept as it is.
pub fn plan_line(status: &str, step: &str) -> String {
    let s = status.trim().to_ascii_lowercase().replace(['_', '-'], " ");
    let s = match s.as_str() {
        "completed" | "complete" | "done" => "done",
        "pending" | "todo" | "" => "pending",
        "inprogress" | "in progress" | "active" => "in progress",
        other => other,
    };
    format!("[{s}] {}", step.trim())
}

/// Remembers the last plan a session showed, so a plan record that repeats on every update
/// adds an event only when the plan changed.
#[derive(Default)]
pub struct PlanTracker(Option<String>);

impl PlanTracker {
    /// The plan as an event's text, unless it is empty or the same as the one before.
    pub fn changed(&mut self, lines: Vec<String>) -> Option<String> {
        let text = lines.join("\n");
        if text.trim().is_empty() || self.0.as_deref() == Some(text.as_str()) {
            return None;
        }
        self.0 = Some(text.clone());
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_ignored_by_its_head_alone() {
        let ig = ["compacted", "event_msg"];
        assert!(is_ignored_record(
            r#"{"type":"compacted","payload":{"a":[1,2]}}"#,
            &ig
        ));
        assert!(is_ignored_record(
            r#"{"payload":{},"type":"event_msg"}"#,
            &ig
        ));
        // An escaped name still reads; a name that is not a string, another type, no type, and
        // a line that is not JSON are left for the full parse.
        assert!(is_ignored_record(r#"{"type":"compacted"}"#, &ig));
        assert!(!is_ignored_record(r#"{"type":7}"#, &ig));
        assert!(!is_ignored_record(r#"{"type":"response_item"}"#, &ig));
        assert!(!is_ignored_record(r#"{"payload":{}}"#, &ig));
        assert!(!is_ignored_record(r#"{"type":"compacted""#, &ig));
        assert!(!is_ignored_record("not json", &ig));
    }

    #[test]
    fn lines_split_as_str_lines_does() {
        let mut got = Vec::new();
        let bytes = b"a\r\nb\n\nc\xff\nlast";
        each_line(&bytes[..], |l| got.push(l.to_string())).unwrap();
        let want: Vec<String> = String::from_utf8_lossy(bytes)
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(got, want);
        assert_eq!(got.len(), 5);
    }
}
