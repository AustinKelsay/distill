//! Grok: `~/.grok/sessions/<percent-encoded cwd>/<session id>/chat_history.jsonl`, one entry
//! per line typed `system`, `user`, `assistant`, `reasoning`, `backend_tool_call` or
//! `tool_result`. The cwd and session id come from the path.

use anyhow::Result;
use serde_json::Value;

use super::chat::push_message;
use crate::model::{Event, EventKind, Parsed};
use crate::source::percent_decode;

pub fn parse(origin: &str, bytes: &[u8]) -> Result<Parsed> {
    let mut out = Parsed::default();
    let parts: Vec<&str> = origin.rsplit('/').collect();
    // [chat_history.jsonl, <session id>, <encoded cwd>, ...]
    out.session_id = parts.get(1).map(|s| s.to_string());
    out.cwd = parts.get(2).map(|s| percent_decode(s));
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Ok(m) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ty = m.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            "reasoning" => {
                let summary = m
                    .get("summary")
                    .and_then(Value::as_array)
                    .map(|s| {
                        s.iter()
                            .filter_map(|x| x.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                out.push(Event::text(EventKind::Reasoning, summary));
            }
            // Server-side tools (search, browsing) run by the backend.
            "backend_tool_call" => {
                let k = m.get("kind").unwrap_or(&Value::Null);
                let mut e = Event::new(EventKind::ToolCall);
                e.tool = k
                    .get("tool_type")
                    .or_else(|| k.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                e.call_id = k.get("id").and_then(Value::as_str).map(str::to_string);
                e.input = k.get("action").or_else(|| k.get("input")).cloned();
                out.push(e);
            }
            _ => push_message(&mut out, ty, &m, None),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_ids_from_the_path_and_parses_entries() {
        let src = r#"{"type":"system","content":"You are Grok"}
{"type":"user","content":[{"type":"text","text":"find docs"}]}
{"type":"reasoning","id":"r","summary":[{"text":"search first"}],"encrypted_content":"x"}
{"type":"backend_tool_call","kind":{"id":"b1","tool_type":"web_search","action":{"query":"docs"}}}
{"type":"assistant","model_id":"grok-4.7","content":"","tool_calls":[{"id":"t1","name":"read","arguments":"{\"path\":\"a\"}"}]}
{"type":"tool_result","tool_call_id":"t1","content":[{"type":"text","text":"ok"}]}"#;
        let p = parse(
            "/u/.grok/sessions/%2FUsers%2Fme%2Frepo/01a0/chat_history.jsonl",
            src.as_bytes(),
        )
        .unwrap();
        assert_eq!(p.session_id.as_deref(), Some("01a0"));
        assert_eq!(p.cwd.as_deref(), Some("/Users/me/repo"));
        let kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::System,
                EventKind::User,
                EventKind::Reasoning,
                EventKind::ToolCall,
                EventKind::ToolCall,
                EventKind::ToolResult
            ]
        );
        assert_eq!(p.events[3].tool.as_deref(), Some("web_search"));
        assert_eq!(p.events[4].input.as_ref().unwrap()["path"], "a");
        assert_eq!(p.models, vec!["grok-4.7"]);
    }
}
