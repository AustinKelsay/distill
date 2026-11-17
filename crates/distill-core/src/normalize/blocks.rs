//! Helpers for the content-block shapes most harnesses share.

use std::borrow::Cow;
use std::io::BufRead;

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::model::{Event, EventKind, Parsed};

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
