//! Droid (Factory): `~/.factory/sessions/<project>/<id>.jsonl`, a `session_start` record then
//! `message` records with Anthropic-style content blocks.

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
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "session_start" => {
                out.session_id = v.get("id").and_then(Value::as_str).map(str::to_string);
                out.cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
                out.title = v
                    .get("sessionTitle")
                    .or_else(|| v.get("title"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                out.parent_session = v
                    .get("callingSessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                out.harness_version = v
                    .get("version")
                    .map(|x| x.to_string().trim_matches('"').to_string());
            }
            "message" => {
                let Some(msg) = v.get("message") else {
                    continue;
                };
                let role = match msg.get("role").and_then(Value::as_str) {
                    Some("assistant") => EventKind::Assistant,
                    Some("system") => EventKind::System,
                    _ => EventKind::User,
                };
                let model = msg.get("model").and_then(Value::as_str);
                if let Some(m) = model {
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
    fn parses_session_start_and_blocks() {
        let src = r#"{"type":"session_start","id":"d1","cwd":"/r","sessionTitle":"Refactor","version":2}
{"type":"message","timestamp":"2026-10-01T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":"go"}]}}
{"type":"message","timestamp":"2026-10-01T00:00:01Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"u1","name":"Read","input":{"path":"a"}}]}}
{"type":"message","timestamp":"2026-10-01T00:00:02Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"u1","content":"ok"}]}}"#;
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(p.session_id.as_deref(), Some("d1"));
        assert_eq!(p.title.as_deref(), Some("Refactor"));
        assert_eq!(p.events.len(), 3);
        assert_eq!(p.events[2].kind, EventKind::ToolResult);
    }
}
