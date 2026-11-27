//! Pi: `~/.pi/agent/sessions/**/<time>_<id>.jsonl`, a `session` header then `message` records
//! whose roles are user, assistant, toolResult and system.

use anyhow::Result;
use serde_json::Value;

use super::blocks::{
    attach_stop_and_usage, first_text, normalize_stop_reason, parse_usage, push_compaction,
    push_content, result_text, ts,
};
use crate::model::{Event, EventKind, Parsed};

/// Record types the parser skips on purpose: a change of thinking level holds no part of the
/// conversation. Any other type it skips is reported as format drift.
pub const IGNORED: &[&str] = &["thinking_level_change", "session_info", "context_edit"];

pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    let mut out = Parsed::default();
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            "session" => {
                out.session_id = v.get("id").and_then(Value::as_str).map(str::to_string);
                out.cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
                out.harness_version = v
                    .get("version")
                    .map(|x| x.to_string().trim_matches('"').to_string());
            }
            "model_change" => {
                if let Some(m) = v.get("modelId").and_then(Value::as_str) {
                    out.add_model(m);
                }
            }
            "message" => {
                let Some(msg) = v.get("message") else {
                    continue;
                };
                let at = ts(msg).or_else(|| ts(&v));
                let model = msg.get("model").and_then(Value::as_str);
                if let Some(m) = model {
                    out.add_model(m);
                }
                let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
                match role {
                    "user" => push_content(
                        &mut out,
                        EventKind::User,
                        msg.get("content").unwrap_or(&Value::Null),
                        at,
                        None,
                    ),
                    "assistant" => {
                        let start = out.events.len();
                        push_content(
                            &mut out,
                            EventKind::Assistant,
                            msg.get("content").unwrap_or(&Value::Null),
                            at.clone(),
                            model,
                        );
                        let stop = msg
                            .get("stopReason")
                            .or_else(|| msg.get("stop_reason"))
                            .and_then(Value::as_str)
                            .and_then(normalize_stop_reason);
                        let usage = msg.get("usage").and_then(parse_usage);
                        if attach_stop_and_usage(&mut out, start, stop.clone(), usage.clone())
                            .is_none()
                            && (stop.is_some() || usage.is_some())
                        {
                            let mut e = Event::new(EventKind::Assistant).at(at);
                            e.model = model.map(str::to_string);
                            e.stop_reason = stop;
                            e.usage = usage;
                            out.push(e);
                        }
                    }
                    "system" => push_content(
                        &mut out,
                        EventKind::System,
                        msg.get("content").unwrap_or(&Value::Null),
                        at,
                        None,
                    ),
                    "toolResult" => {
                        let mut e = Event::new(EventKind::ToolResult).at(at);
                        e.call_id = msg
                            .get("toolCallId")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        e.tool = msg
                            .get("toolName")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        e.is_error = msg.get("isError").and_then(Value::as_bool);
                        e.text = Some(result_text(msg.get("content").unwrap_or(&Value::Null)));
                        out.push(e);
                    }
                    _ => out.skip_in("message", role, msg),
                }
            }
            // The summary that replaces earlier turns in the model's context.
            "compaction" => {
                push_compaction(&mut out, v.get("summary").and_then(Value::as_str), ts(&v));
            }
            // An extension's own entry; only the kinds known to hold text are read.
            "custom" => {
                let kind = v.get("customType").and_then(Value::as_str).unwrap_or("");
                let data = v.get("data").unwrap_or(&Value::Null);
                match kind {
                    "summary-recap" => {
                        let mut parts = Vec::new();
                        if let Some(r) = first_text(data, &["recap"]) {
                            parts.push(r.to_string());
                        }
                        match data.get("next") {
                            Some(Value::String(n)) if !n.trim().is_empty() => {
                                parts.push(format!("Next: {n}"))
                            }
                            Some(Value::Array(items)) => {
                                let lines: Vec<&str> =
                                    items.iter().filter_map(Value::as_str).collect();
                                if !lines.is_empty() {
                                    parts.push(format!("Next:\n{}", lines.join("\n")));
                                }
                            }
                            _ => {}
                        }
                        out.push(Event::text(EventKind::System, parts.join("\n\n")).at(ts(&v)));
                    }
                    _ => out.skip_in("custom", kind, &v),
                }
            }
            // Text an extension put into the model's context.
            "custom_message" => {
                let kind = v.get("customType").and_then(Value::as_str).unwrap_or("");
                match kind {
                    "background-terminal-result" => {
                        let mut e = Event::new(EventKind::ToolResult).at(ts(&v));
                        e.tool = Some("background-terminal".into());
                        e.text = Some(result_text(v.get("content").unwrap_or(&Value::Null)));
                        out.push(e);
                    }
                    _ => out.skip_in("custom_message", kind, &v),
                }
            }
            _ => out.skip(ty, line.len()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_calls_and_results() {
        let src = r#"{"type":"session","id":"p1","cwd":"/r","version":3,"timestamp":"2026-08-01T20:24:49Z"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"search"}],"timestamp":1794436930000}}
{"type":"message","message":{"role":"assistant","model":"glm-5","content":[{"type":"toolCall","id":"c1","name":"web_search","arguments":{"q":"x"}}],"timestamp":1794436931000}}
{"type":"message","message":{"role":"toolResult","toolCallId":"c1","toolName":"web_search","isError":true,"content":[{"type":"text","text":"boom"}],"timestamp":1794436936342}}"#;
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.session_id.as_deref(), Some("p1"));
        assert_eq!(p.events.len(), 3);
        assert_eq!(p.events[1].input.as_ref().unwrap()["q"], "x");
        assert_eq!(p.events[2].is_error, Some(true));
        assert!(p.events[0].ts.as_deref().unwrap().starts_with("2026-"));
    }

    #[test]
    fn parses_compactions_recaps_and_background_results() {
        let src = r#"{"type":"session","id":"p2","cwd":"/r","version":3}
{"type":"compaction","id":"c1","timestamp":"2026-08-01T20:30:00Z","summary":"Earlier: fixed the parser.","firstKeptEntryId":"e9","tokensBefore":9000}
{"type":"custom","customType":"summary-recap","timestamp":"2026-08-01T20:31:00Z","data":{"recap":"Wired the index.","next":"Add tests."}}
{"type":"custom","customType":"some-ext-state","data":{"x":1}}
{"type":"custom_message","customType":"background-terminal-result","timestamp":"2026-08-01T20:32:00Z","content":"build ok","display":true}
{"type":"custom_message","customType":"other-note","content":"hm","display":true}"#;
        let p = parse(src.as_bytes()).unwrap();
        let kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![EventKind::System, EventKind::System, EventKind::ToolResult]
        );
        assert_eq!(
            p.events[0].text.as_deref(),
            Some("Earlier: fixed the parser.")
        );
        assert_eq!(p.events[0].compaction, Some(true));
        assert_eq!(p.events[1].compaction, None);
        assert_eq!(
            p.events[1].text.as_deref(),
            Some("Wired the index.\n\nNext: Add tests.")
        );
        assert_eq!(p.events[2].text.as_deref(), Some("build ok"));
        let names: Vec<_> = p.skipped.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            vec!["custom:some-ext-state", "custom_message:other-note"]
        );
    }

    #[test]
    fn extracts_assistant_stop_reason_and_usage() {
        let src = r#"{"type":"session","id":"p3","cwd":"/r","version":3}
{"type":"message","message":{"role":"user","content":"go","timestamp":1794436930000}}
{"type":"message","message":{"role":"assistant","model":"glm-5","stopReason":"toolUse","usage":{"input":400,"output":60,"cacheRead":120,"cacheWrite":15},"content":[{"type":"thinking","thinking":"check"},{"type":"toolCall","id":"c1","name":"bash","arguments":{"cmd":"ls"}}],"timestamp":1794436931000}}
{"type":"message","message":{"role":"assistant","model":"glm-5","stopReason":"aborted","usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},"content":[{"type":"text","text":""}],"timestamp":1794436931500}}
{"type":"message","message":{"role":"assistant","model":"glm-5","stopReason":"stop","usage":{"input":500,"output":40},"content":[{"type":"text","text":"done"}],"timestamp":1794436932000}}"#;
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.events.len(), 5);
        let user_json = serde_json::to_value(&p.events[0]).unwrap();
        assert!(user_json.get("compaction").is_none());
        assert!(user_json.get("stop_reason").is_none());
        assert!(user_json.get("usage").is_none());
        assert_eq!(p.events[1].usage, None);
        assert_eq!(p.events[2].stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(
            p.events[2].usage,
            Some(crate::model::TokenUsage {
                input: Some(400),
                output: Some(60),
                cache_read: Some(120),
                cache_write: Some(15),
            })
        );
        // Empty-content aborted assistant turn is preserved with its stop_reason and usage.
        assert_eq!(p.events[3].kind, EventKind::Assistant);
        assert_eq!(p.events[3].stop_reason.as_deref(), Some("aborted"));
        assert_eq!(p.events[3].usage.as_ref().unwrap().input, Some(0));
        assert_eq!(p.events[4].stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(p.events[4].usage.as_ref().unwrap().output, Some(40));
    }
}
