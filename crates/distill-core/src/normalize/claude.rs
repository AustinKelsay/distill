//! Claude Code: `~/.claude/projects/<project>/<session>.jsonl`, one record per line.

use std::io::BufRead;

use anyhow::Result;
use serde_json::Value;

use super::blocks::{each_line, push_content, ts};
use crate::model::{EventKind, Parsed};

/// Record types the parser skips on purpose: the harness's bookkeeping (titles, summaries, file
/// snapshots, the prompt queue, status and progress notes, cost and mode, links and lineage), the
/// context it injects (`attachment`: environment, tool and skill listings, reminders), and the API
/// request log, whose blobs replay the conversation the parser already reads. Checked against
/// parsed sessions in a real archive (#104). Any other type it skips is reported as format drift.
pub const IGNORED: &[&str] = &[
    "custom-title",
    "summary",
    "file-history-snapshot",
    "queue-operation",
    "system",
    "progress",
    "ai-title",
    "agent-name",
    "api-request",
    "api-request-blob",
    "api-request-shape",
    "atis-latch",
    "attachment",
    "cost-state",
    "fork-context-ref",
    "frame-link",
    "last-prompt",
    "mode",
    "pr-link",
    "session-owner",
    "tools-used",
];

pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    parse_reader(bytes)
}

/// [`parse`] over a stream, one line at a time, so the capture is never held whole. (Every record is read, since
/// the ones the parser skips still carry `cwd`, `gitBranch` and `version`.)
pub fn parse_reader(reader: impl BufRead) -> Result<Parsed> {
    let mut out = Parsed::default();
    each_line(reader, |line| {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
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
        // A sub-agent's own file (`<session>/subagents/agent-*.jsonl`) marks its records as a
        // side chain of the session that started it, and names that session.
        if out.parent_session.is_none()
            && v.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            out.parent_session = v
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if let Some(ver) = v.get("version").and_then(Value::as_str) {
            out.harness_version = Some(ver.to_string());
        }
        match ty {
            "user" | "assistant" => {
                let Some(msg) = v.get("message") else {
                    return;
                };
                // Synthetic records: API error stand-ins and harness meta prompts.
                if v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) {
                    return;
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
            _ => out.skip(ty, line.len()),
        }
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sub_agent_names_the_session_that_started_it() {
        let src = r#"{"type":"user","isSidechain":true,"sessionId":"parent-1","agentId":"a1","message":{"role":"user","content":"look into it"}}
{"type":"assistant","isSidechain":true,"sessionId":"parent-1","agentId":"a1","message":{"role":"assistant","content":[{"type":"text","text":"Done."}]}}"#;
        assert_eq!(
            parse(src.as_bytes()).unwrap().parent_session.as_deref(),
            Some("parent-1")
        );
        let main = r#"{"type":"user","isSidechain":false,"sessionId":"s","message":{"role":"user","content":"hi"}}"#;
        assert_eq!(parse(main.as_bytes()).unwrap().parent_session, None);
    }

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
        assert_eq!(p.models, vec!["claude-opus-5-5"]);
    }
}
