//! Pi: `~/.pi/agent/sessions/**/<time>_<id>.jsonl`, a `session` header then `message` records
//! whose roles are user, assistant, toolResult and system.

use anyhow::Result;
use serde_json::Value;

use super::blocks::{push_content, result_text, ts};
use crate::model::{Event, EventKind, Parsed};

pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    let mut out = Parsed::default();
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
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
                match msg.get("role").and_then(Value::as_str).unwrap_or("") {
                    "user" => push_content(
                        &mut out,
                        EventKind::User,
                        msg.get("content").unwrap_or(&Value::Null),
                        at,
                        None,
                    ),
                    "assistant" => push_content(
                        &mut out,
                        EventKind::Assistant,
                        msg.get("content").unwrap_or(&Value::Null),
                        at,
                        model,
                    ),
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
                    _ => {}
                }
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
}
