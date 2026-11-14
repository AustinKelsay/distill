//! Hermes: `~/.hermes/sessions/<id>.jsonl` (a `session_meta` line, then one message per line)
//! and older `session_<id>.json` files holding a `messages` array.

use anyhow::Result;
use serde_json::Value;

use super::blocks::ts;
use super::chat::push_message;
use crate::model::{Event, EventKind, Parsed};

pub fn parse(origin: &str, bytes: &[u8]) -> Result<Parsed> {
    let mut out = Parsed::default();
    let text = String::from_utf8_lossy(bytes);
    if origin.ends_with(".json") {
        let v: Value = serde_json::from_str(&text)?;
        out.session_id = v
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(m) = v.get("model").and_then(Value::as_str) {
            out.add_model(m);
        }
        let start = v
            .get("session_start")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(sp) = v.get("system_prompt").and_then(Value::as_str) {
            out.push(Event::text(EventKind::System, sp).at(start.clone()));
        }
        for (i, m) in v
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let role = m.get("role").and_then(Value::as_str).unwrap_or("");
            // These files carry no per-message times; the session start stands in for the first.
            let at = ts(m).or_else(|| (i == 0).then(|| start.clone()).flatten());
            push_message(&mut out, role, m, at);
        }
    } else {
        out.session_id = Some(
            origin
                .rsplit('/')
                .next()
                .unwrap_or(origin)
                .trim_end_matches(".jsonl")
                .to_string(),
        );
        for line in text.lines() {
            let Ok(m) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let role = m.get("role").and_then(Value::as_str).unwrap_or("");
            if role == "session_meta" {
                if let Some(md) = m.get("model").and_then(Value::as_str) {
                    out.add_model(md);
                }
                continue;
            }
            push_message(&mut out, role, &m, ts(&m));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jsonl_and_json_sessions() {
        let jsonl = r#"{"role":"session_meta","model":"glm-5","platform":"telegram","timestamp":"2026-10-01T00:00:00Z"}
{"role":"user","content":"weather?","timestamp":"2026-10-01T00:00:01Z"}
{"role":"assistant","content":"","reasoning":"look it up","tool_calls":[{"id":"c1","function":{"name":"web","arguments":"{\"q\":\"weather\"}"}}],"timestamp":"2026-10-01T00:00:02Z"}
{"role":"tool","tool_call_id":"c1","content":"sunny","timestamp":"2026-10-01T00:00:03Z"}"#;
        let p = parse("/h/sessions/20261001_000000_ab12.jsonl", jsonl.as_bytes()).unwrap();
        assert_eq!(p.session_id.as_deref(), Some("20261001_000000_ab12"));
        let kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::User,
                EventKind::Reasoning,
                EventKind::ToolCall,
                EventKind::ToolResult
            ]
        );
        assert_eq!(p.events[2].input.as_ref().unwrap()["q"], "weather");
        assert_eq!(p.models, vec!["glm-5"]);

        let json = r#"{"session_id":"s9","model":"m","session_start":"2026-09-01T00:00:00Z","system_prompt":"be brief","messages":[{"role":"user","content":"hi"},{"role":"assistant","content":"hello"}]}"#;
        let p = parse("/h/sessions/session_s9.json", json.as_bytes()).unwrap();
        assert_eq!(p.session_id.as_deref(), Some("s9"));
        assert_eq!(p.events.len(), 3);
        assert_eq!(p.events[1].ts.as_deref(), Some("2026-09-01T00:00:00Z"));
    }
}
