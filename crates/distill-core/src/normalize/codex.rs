//! Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` with `session_meta`, `turn_context`,
//! `response_item` and `event_msg` records.

use anyhow::Result;
use serde_json::Value;

use super::blocks::{result_text, ts};
use crate::model::{Event, EventKind, Parsed};

pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    let mut out = Parsed::default();
    let mut model: Option<String> = None;
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        let Some(p) = v.get("payload") else { continue };
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
            _ => {}
        }
    }
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
        _ => {}
    }
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
}
