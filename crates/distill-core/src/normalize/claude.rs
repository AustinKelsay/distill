//! Claude Code: `~/.claude/projects/<project>/<session>.jsonl`, one record per line.

use std::io::BufRead;

use anyhow::Result;
use serde_json::Value;

use super::blocks::{
    attach_stop_and_usage, each_line, normalize_stop_reason, parse_usage, push_compaction,
    push_content, ts,
};
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
    let mut last_assistant_msg: Option<(String, usize)> = None;
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
            "system" if v.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => {
                push_compaction(&mut out, v.get("content").and_then(Value::as_str), ts(&v));
                last_assistant_msg = None;
            }
            "user" | "assistant" => {
                let Some(msg) = v.get("message") else {
                    return;
                };
                // Synthetic records: API error stand-ins and harness meta prompts.
                if v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) {
                    return;
                }
                let is_compact = v.get("isCompactSummary").and_then(Value::as_bool) == Some(true);
                let mut role = if ty == "user" {
                    EventKind::User
                } else {
                    EventKind::Assistant
                };
                if v.get("isMeta").and_then(Value::as_bool) == Some(true) || is_compact {
                    role = EventKind::System;
                }
                let model = msg.get("model").and_then(Value::as_str);
                if let Some(m) = model
                    && m != "<synthetic>"
                {
                    out.add_model(m);
                }
                let start = out.events.len();
                push_content(
                    &mut out,
                    role,
                    msg.get("content").unwrap_or(&Value::Null),
                    ts(&v),
                    model,
                );
                if is_compact {
                    for e in &mut out.events[start..] {
                        e.compaction = Some(true);
                    }
                    last_assistant_msg = None;
                } else if ty == "assistant" {
                    let stop = msg
                        .get("stop_reason")
                        .and_then(Value::as_str)
                        .and_then(normalize_stop_reason);
                    let usage = msg.get("usage").and_then(parse_usage);
                    let msg_id = msg.get("id").and_then(Value::as_str);
                    if let Some(id) = msg_id {
                        let prev = last_assistant_msg
                            .as_ref()
                            .filter(|(prev_id, _)| prev_id == id)
                            .map(|(_, idx)| *idx);
                        if out.events.len() > start {
                            let (carried_stop, carried_usage) = if let Some(prev_idx) = prev {
                                (
                                    out.events[prev_idx].stop_reason.take(),
                                    out.events[prev_idx].usage.take(),
                                )
                            } else {
                                (None, None)
                            };
                            if let Some(idx) = attach_stop_and_usage(
                                &mut out,
                                start,
                                stop.or(carried_stop),
                                usage.or(carried_usage),
                            ) {
                                last_assistant_msg = Some((id.to_string(), idx));
                            }
                        } else if let Some(prev_idx) = prev {
                            if let Some(reason) = stop {
                                out.events[prev_idx].stop_reason = Some(reason);
                            }
                            if let Some(u) = usage {
                                out.events[prev_idx].usage = Some(u);
                            }
                        }
                    } else {
                        attach_stop_and_usage(&mut out, start, stop, usage);
                        last_assistant_msg = None;
                    }
                } else {
                    last_assistant_msg = None;
                }
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

    #[test]
    fn preserves_compaction_seams_stop_reasons_and_token_usage() {
        let src = r#"{"type":"user","timestamp":"2026-11-01T10:00:00Z","message":{"role":"user","content":"check"}}
{"type":"assistant","timestamp":"2026-11-01T10:00:01Z","message":{"id":"m1","role":"assistant","model":"claude-opus-5-5","stop_reason":null,"usage":{"input_tokens":100,"output_tokens":3,"cache_read_input_tokens":40,"cache_creation_input_tokens":10},"content":[{"type":"thinking","thinking":"plan"}]}}
{"type":"assistant","timestamp":"2026-11-01T10:00:02Z","message":{"id":"m1","role":"assistant","model":"claude-opus-5-5","stop_reason":"tool_use","usage":{"input_tokens":100,"output_tokens":25,"cache_read_input_tokens":40,"cache_creation_input_tokens":10},"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"path":"a"}}]}}
{"type":"system","subtype":"compact_boundary","timestamp":"2026-11-01T10:00:03Z","content":"Conversation compacted"}
{"type":"user","isCompactSummary":true,"timestamp":"2026-11-01T10:00:04Z","message":{"role":"user","content":"Summary of earlier turns."}}
{"type":"assistant","timestamp":"2026-11-01T10:00:05Z","message":{"role":"assistant","model":"claude-opus-5-5","stop_reason":"end_turn","usage":{"input_tokens":200,"output_tokens":50},"content":[{"type":"text","text":"Done."}]}}"#;
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.events.len(), 6);
        // Untouched user event omits all three optional fields.
        let user_json = serde_json::to_value(&p.events[0]).unwrap();
        assert!(user_json.get("compaction").is_none());
        assert!(user_json.get("stop_reason").is_none());
        assert!(user_json.get("usage").is_none());
        // Multi-frame assistant message moves usage to the final frame without double-counting.
        assert_eq!(p.events[1].usage, None);
        assert_eq!(p.events[1].stop_reason, None);
        assert_eq!(p.events[2].stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(
            p.events[2].usage,
            Some(crate::model::TokenUsage {
                input: Some(100),
                output: Some(25),
                cache_read: Some(40),
                cache_write: Some(10),
            })
        );
        // Both compact_boundary and isCompactSummary are marked compaction: Some(true).
        assert_eq!(p.events[3].kind, EventKind::System);
        assert_eq!(p.events[3].compaction, Some(true));
        assert_eq!(p.events[3].text.as_deref(), Some("Conversation compacted"));
        assert_eq!(p.events[4].kind, EventKind::System);
        assert_eq!(p.events[4].compaction, Some(true));
        assert_eq!(
            p.events[4].text.as_deref(),
            Some("Summary of earlier turns.")
        );
        assert_eq!(p.events[5].stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(p.events[5].usage.as_ref().unwrap().output, Some(50));
    }
}
