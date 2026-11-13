//! Claude Code: `~/.claude/projects/<project>/<session>.jsonl`, one record per line.

use anyhow::Result;
use serde_json::Value;

use super::blocks::{push_content, ts};
use crate::model::{EventKind, Parsed};

pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    let mut out = Parsed::default();
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        if out.cwd.is_none() {
            out.cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
        }
        if out.git_branch.is_none() {
            out.git_branch = v
                .get("gitBranch")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if let Some(ver) = v.get("version").and_then(Value::as_str) {
            out.harness_version = Some(ver.to_string());
        }
        match ty {
            "custom-title" | "ai-title" => {
                if let Some(t) = v
                    .get("customTitle")
                    .or_else(|| v.get("title"))
                    .and_then(Value::as_str)
                {
                    out.title = Some(t.to_string());
                }
            }
            "summary" if out.title.is_none() => {
                out.title = v.get("summary").and_then(Value::as_str).map(str::to_string);
            }
            "user" | "assistant" => {
                let Some(msg) = v.get("message") else {
                    continue;
                };
                // Synthetic records: API error stand-ins and harness meta prompts.
                if v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                let mut role = if ty == "user" {
                    EventKind::User
                } else {
                    EventKind::Assistant
                };
                if v.get("isMeta").and_then(Value::as_bool) == Some(true)
                    || v.get("isCompactSummary").and_then(Value::as_bool) == Some(true)
                {
                    role = EventKind::System;
                }
                let model = msg.get("model").and_then(Value::as_str);
                if let Some(m) = model
                    && m != "<synthetic>"
                {
                    out.add_model(m);
                }
                push_content(
                    &mut out,
                    role,
                    msg.get("content").unwrap_or(&Value::Null),
                    ts(&v),
                    model,
                );
            }
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_tool_calls_results_and_reasoning() {
        let src = r#"{"type":"user","cwd":"/r","gitBranch":"main","version":"2.1.0","timestamp":"2026-11-01T10:00:00Z","message":{"role":"user","content":"fix the test"}}
{"type":"assistant","timestamp":"2026-11-01T10:00:05Z","message":{"role":"assistant","model":"claude-opus-5-5","content":[{"type":"thinking","thinking":"look first"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}
{"type":"user","timestamp":"2026-11-01T10:00:09Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"1 failed","is_error":true}]}}
{"type":"assistant","timestamp":"2026-11-01T10:00:20Z","message":{"role":"assistant","model":"claude-opus-5-5","content":[{"type":"text","text":"Fixed."}]}}
{"type":"custom-title","customTitle":"Fix test","sessionId":"s"}
not json"#;
        let p = parse(src.as_bytes()).unwrap();
        let kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::User,
                EventKind::Reasoning,
                EventKind::ToolCall,
                EventKind::ToolResult,
                EventKind::Assistant
            ]
        );
        assert_eq!(p.events[2].tool.as_deref(), Some("Bash"));
        assert_eq!(p.events[3].call_id.as_deref(), Some("t1"));
        assert_eq!(p.events[3].is_error, Some(true));
        assert_eq!(p.cwd.as_deref(), Some("/r"));
        assert_eq!(p.title.as_deref(), Some("Fix test"));
        assert_eq!(p.models, vec!["claude-opus-5-5"]);
    }
}
