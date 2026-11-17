//! Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` with `session_meta`, `turn_context`,
//! `response_item` and `event_msg` records.

use std::io::BufRead;

use anyhow::Result;
use serde_json::{Value, json};

use super::blocks::{each_line, is_ignored_record, result_text, ts};
use crate::model::{Event, EventKind, Parsed};

/// Record types the parser skips on purpose: `event_msg` records repeat what the response items
/// hold, for the harness's own display; a ghost snapshot is the git state Codex keeps to undo a
/// turn; `compacted` replays the history the parser already read; and the rest are environment
/// snapshots, token usage, tool manifests and settings. Checked against parsed sessions in a
/// real archive (#104). Any other type it skips is reported as format drift.
pub const IGNORED: &[&str] = &[
    "event_msg",
    "response_item:ghost_snapshot",
    "compacted",
    "world_state",
    "token_usage_record",
    "inter_agent_communication_metadata",
    "realtime_item",
    "response_item:additional_tools",
    "response_item:configuration_update",
];

pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    parse_reader(bytes)
}

/// [`parse`] over a stream, one line at a time: the capture is never held whole, and a record
/// of a type the parser skips on purpose is recognised from its head, without building a tree.
pub fn parse_reader(reader: impl BufRead) -> Result<Parsed> {
    let mut out = Parsed::default();
    let mut model: Option<String> = None;
    each_line(reader, |line| {
        if is_ignored_record(line, IGNORED) {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        let Some(p) = v.get("payload") else { return };
        let at = ts(&v);
        match ty {
            "session_meta" => {
                out.session_id = p.get("id").and_then(Value::as_str).map(str::to_string);
                out.cwd = p.get("cwd").and_then(Value::as_str).map(str::to_string);
                out.harness_version = p
                    .get("cli_version")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(b) = p.pointer("/git/branch").and_then(Value::as_str) {
                    out.git_branch = Some(b.to_string());
                }
                if let Some(instr) = p
                    .get("base_instructions")
                    .and_then(|b| b.get("text").or(Some(b)))
                    .and_then(Value::as_str)
                {
                    out.push(Event::text(EventKind::System, instr).at(at));
                }
            }
            "turn_context" => {
                if let Some(m) = p.get("model").and_then(Value::as_str) {
                    model = Some(m.to_string());
                    out.add_model(m);
                }
                if out.cwd.is_none() {
                    out.cwd = p.get("cwd").and_then(Value::as_str).map(str::to_string);
                }
            }
            "response_item" => push_item(&mut out, p, at, model.as_deref()),
            _ => out.skip(ty, line.len()),
        }
    })?;
    Ok(out)
}

fn push_item(out: &mut Parsed, p: &Value, at: Option<String>, model: Option<&str>) {
    let ptype = p.get("type").and_then(Value::as_str).unwrap_or("");
    match ptype {
        "message" => {
            let role = match p.get("role").and_then(Value::as_str) {
                Some("user") => EventKind::User,
                Some("assistant") => EventKind::Assistant,
                _ => EventKind::System,
            };
            let text = result_text(p.get("content").unwrap_or(&Value::Null));
            let mut e = Event::text(role, text).at(at);
            if role == EventKind::Assistant {
                e.model = model.map(str::to_string);
            }
            out.push(e);
        }
        "reasoning" => {
            // Codex stores reasoning encrypted; only the summary is readable.
            let summary = p
                .get("summary")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|s| s.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            out.push(Event::text(EventKind::Reasoning, summary).at(at));
        }
        "function_call" | "custom_tool_call" | "local_shell_call" | "web_search_call" => {
            let mut e = Event::new(EventKind::ToolCall).at(at);
            e.tool = p
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| Some(ptype.trim_end_matches("_call").to_string()));
            e.call_id = p.get("call_id").and_then(Value::as_str).map(str::to_string);
            e.input = match p
                .get("arguments")
                .or_else(|| p.get("input"))
                .or_else(|| p.get("action"))
            {
                // Function arguments are a JSON string; keep them structured when they parse.
                Some(Value::String(s)) => {
                    Some(serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.clone())))
                }
                other => other.cloned(),
            };
            e.model = model.map(str::to_string);
            out.push(e);
        }
        "function_call_output" | "custom_tool_call_output" | "local_shell_call_output" => {
            let mut e = Event::new(EventKind::ToolResult).at(at);
            e.call_id = p.get("call_id").and_then(Value::as_str).map(str::to_string);
            let output = p.get("output").unwrap_or(&Value::Null);
            e.text = Some(match output {
                Value::String(s) => match serde_json::from_str::<Value>(s) {
                    Ok(Value::Object(o)) if o.contains_key("output") => o
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or(s)
                        .to_string(),
                    _ => s.clone(),
                },
                Value::Object(o) => o
                    .get("content")
                    .map(result_text)
                    .unwrap_or_else(|| output.to_string()),
                other => result_text(other),
            });
            out.push(e);
        }
        "agent_message" => {
            // Between agents (their ids): the text is the assistant's, so the ids stay in it.
            let text = p
                .get("content")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|b| b.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let id = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("?").to_string();
            if !text.trim().is_empty() {
                let mut e = Event::text(
                    EventKind::Assistant,
                    format!("[{} \u{2192} {}] {text}", id("author"), id("recipient")),
                )
                .at(at);
                e.model = model.map(str::to_string);
                out.push(e);
            }
        }
        "tool_search_call" => {
            let mut e = Event::new(EventKind::ToolCall).at(at);
            e.tool = Some("tool_search".into());
            e.call_id = item_call_id(p);
            e.input = p.get("arguments").cloned();
            e.model = model.map(str::to_string);
            out.push(e);
        }
        "tool_search_output" => {
            let mut e = Event::new(EventKind::ToolResult).at(at);
            e.tool = Some("tool_search".into());
            e.call_id = item_call_id(p);
            let mut lines = Vec::new();
            if let Some(tools) = p.get("tools").and_then(Value::as_array) {
                list_tools(tools, &mut lines);
            }
            e.text = Some(bounded_tool_list(lines));
            e.is_error =
                (p.get("status").and_then(Value::as_str) == Some("failed")).then_some(true);
            out.push(e);
        }
        "image_generation_call" => {
            // The image itself (`result`, base64) is never kept: a placeholder stands in.
            let call_id = item_call_id(p);
            let mut call = Event::new(EventKind::ToolCall).at(at.clone());
            call.tool = Some("image_generation".into());
            call.call_id = call_id.clone();
            call.input = Some(json!({"prompt": p.get("revised_prompt")}));
            call.model = model.map(str::to_string);
            out.push(call);
            let mut res = Event::text(EventKind::ToolResult, "[image generated]").at(at);
            res.tool = Some("image_generation".into());
            res.call_id = call_id;
            res.is_error =
                (p.get("status").and_then(Value::as_str) == Some("failed")).then_some(true);
            out.push(res);
        }
        _ => out.skip_in("response_item", ptype, p),
    }
}

/// How a call and its result are paired: the `call_id`, else the item's own id. Codex writes a
/// null `call_id` for a tool search the server ran, which counts as none.
fn item_call_id(p: &Value) -> Option<String> {
    p.get("call_id")
        .and_then(Value::as_str)
        .or_else(|| p.get("id").and_then(Value::as_str))
        .map(str::to_string)
}

/// The most tools a tool search's result lists, and the longest description kept for each.
const TOOLS_LISTED: usize = 40;
const DESCRIPTION_MAX: usize = 160;

/// A tool search's result as one line per tool (`name: short description`), going into the
/// tools a namespace holds. The schemas are left out.
fn list_tools(tools: &[Value], lines: &mut Vec<String>) {
    for t in tools {
        if let Some(inner) = t.get("tools").and_then(Value::as_array) {
            list_tools(inner, lines);
            continue;
        }
        let name = t
            .get("name")
            .or_else(|| t.pointer("/function/name"))
            .and_then(Value::as_str);
        let Some(name) = name else { continue };
        let desc = t
            .get("description")
            .or_else(|| t.pointer("/function/description"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let short: String = desc
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(DESCRIPTION_MAX)
            .collect();
        lines.push(if short.is_empty() {
            name.to_string()
        } else {
            format!("{name}: {short}")
        });
    }
}

fn bounded_tool_list(mut lines: Vec<String>) -> String {
    if lines.is_empty() {
        return "[no tools found]".into();
    }
    let more = lines.len().saturating_sub(TOOLS_LISTED);
    lines.truncate(TOOLS_LISTED);
    if more > 0 {
        lines.push(format!("... and {more} more"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_meta_calls_and_outputs() {
        let src = r#"{"timestamp":"2026-11-03T08:30:53Z","type":"session_meta","payload":{"id":"abc","cwd":"/r","cli_version":"0.169.0"}}
{"timestamp":"2026-11-03T08:30:54Z","type":"turn_context","payload":{"model":"gpt-6","cwd":"/r"}}
{"timestamp":"2026-11-03T08:30:55Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"list files"}]}}
{"timestamp":"2026-11-03T08:30:56Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"cmd\":[\"ls\"]}","call_id":"c1"}}
{"timestamp":"2026-11-03T08:30:57Z","type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"{\"output\":\"a.txt\\n\",\"metadata\":{}}"}}
{"timestamp":"2026-11-03T08:30:58Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"One file."}]}}"#;
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.session_id.as_deref(), Some("abc"));
        assert_eq!(p.models, vec!["gpt-6"]);
        assert_eq!(p.events.len(), 4);
        assert_eq!(p.events[1].input.as_ref().unwrap()["cmd"][0], "ls");
        assert_eq!(p.events[2].text.as_deref(), Some("a.txt\n"));
        assert_eq!(p.events[3].model.as_deref(), Some("gpt-6"));
    }

    #[test]
    fn a_null_call_id_falls_back_to_the_items_id() {
        let id = |v: serde_json::Value| item_call_id(&v);
        assert_eq!(
            id(serde_json::json!({"call_id": null, "id": "tsc_1"})).as_deref(),
            Some("tsc_1")
        );
        assert_eq!(
            id(serde_json::json!({"id": "tsc_2"})).as_deref(),
            Some("tsc_2")
        );
        assert_eq!(
            id(serde_json::json!({"call_id": "c1", "id": "tsc_3"})).as_deref(),
            Some("c1")
        );
        assert_eq!(id(serde_json::json!({"call_id": null})), None);
    }

    #[test]
    fn parses_agent_messages_tool_search_and_image_generation() {
        let src = r#"{"timestamp":"2026-11-03T08:30:54Z","type":"turn_context","payload":{"model":"gpt-6"}}
{"timestamp":"2026-11-03T08:31:00Z","type":"response_item","payload":{"type":"agent_message","author":"/root","recipient":"/root/worker","content":[{"type":"input_text","text":"check the build"}]}}
{"timestamp":"2026-11-03T08:31:01Z","type":"response_item","payload":{"type":"tool_search_call","call_id":"s1","execution":"client","arguments":{"query":"calendar","limit":2}}}
{"timestamp":"2026-11-03T08:31:02Z","type":"response_item","payload":{"type":"tool_search_output","call_id":"s1","status":"completed","execution":"client","tools":[{"type":"function","name":"cal_list","description":"List events.\nLong schema notes","parameters":{"type":"object"}},{"type":"namespace","name":"mail","tools":[{"type":"function","name":"mail_send"}]}]}}
{"timestamp":"2026-11-03T08:31:03Z","type":"response_item","payload":{"type":"image_generation_call","id":"ig_1","status":"completed","revised_prompt":"a gray cat","result":"QUJDREVGRw=="}}"#;
        let p = parse(src.as_bytes()).unwrap();
        assert!(p.skipped.is_empty(), "{:?}", p.skipped);
        let e = &p.events;
        assert_eq!(e.len(), 5);
        assert_eq!(e[0].kind, EventKind::Assistant);
        assert_eq!(
            e[0].text.as_deref(),
            Some("[/root \u{2192} /root/worker] check the build")
        );
        assert_eq!(e[1].input.as_ref().unwrap()["query"], "calendar");
        assert_eq!(e[2].call_id, e[1].call_id);
        assert_eq!(
            e[2].text.as_deref(),
            Some("cal_list: List events.\nmail_send")
        );
        assert_eq!(e[3].input.as_ref().unwrap()["prompt"], "a gray cat");
        assert_eq!(e[4].text.as_deref(), Some("[image generated]"));
        assert_eq!(e[4].call_id.as_deref(), Some("ig_1"));
        assert!(!serde_json::to_string(e).unwrap().contains("QUJD"));
    }
}
