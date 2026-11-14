//! Antigravity: `.../brain/<conversation id>/.system_generated/logs/transcript_full.jsonl`, one
//! step per line: `USER_INPUT`, `PLANNER_RESPONSE` (the model's turn, with `tool_calls`), and
//! steps that ran tools (`RUN_COMMAND`, `VIEW_FILE`, …) or failed (`ERROR_MESSAGE`).

use anyhow::Result;
use serde_json::Value;

use crate::model::{Event, EventKind, Parsed};

fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

pub fn parse(origin: &str, bytes: &[u8]) -> Result<Parsed> {
    let mut out = Parsed {
        session_id: origin
            .split("/brain/")
            .nth(1)
            .and_then(|r| r.split('/').next())
            .map(str::to_string),
        ..Default::default()
    };
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        let at = v
            .get("created_at")
            .and_then(Value::as_str)
            .map(str::to_string);
        match ty {
            "USER_INPUT" => {
                out.push(Event::text(EventKind::User, text_of(v.get("content"))).at(at))
            }
            "PLANNER_RESPONSE" => {
                out.push(
                    Event::text(EventKind::Assistant, text_of(v.get("content"))).at(at.clone()),
                );
                for call in v
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let mut e = Event::new(EventKind::ToolCall).at(at.clone());
                    e.tool = call.get("name").and_then(Value::as_str).map(str::to_string);
                    e.input = call.get("args").cloned();
                    out.push(e);
                }
            }
            "" => {}
            step => {
                let mut e = Event::new(EventKind::ToolResult).at(at);
                e.tool = Some(step.to_ascii_lowercase());
                let content = text_of(v.get("content"));
                e.text = Some(if content.is_empty() {
                    text_of(v.get("error"))
                } else {
                    content
                });
                let failed = step == "ERROR_MESSAGE"
                    || v.get("error").is_some_and(|x| !x.is_null())
                    || v.get("exit_code")
                        .and_then(Value::as_i64)
                        .is_some_and(|c| c != 0);
                e.is_error = Some(failed);
                out.push(e);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_steps_to_events() {
        let src = r#"{"type":"USER_INPUT","content":"fix the build","created_at":"2026-10-01T00:00:00Z","source":"USER","status":"DONE","step_index":0}
{"type":"PLANNER_RESPONSE","content":"Running the build.","tool_calls":[{"name":"run_command","args":{"cmd":"make"}}],"created_at":"2026-10-01T00:00:01Z"}
{"type":"RUN_COMMAND","content":"error: x","exit_code":2,"created_at":"2026-10-01T00:00:02Z"}
{"type":"ERROR_MESSAGE","content":"","error":"quota","created_at":"2026-10-01T00:00:03Z"}"#;
        let p = parse(
            "/a/brain/conv-1/.system_generated/logs/transcript_full.jsonl",
            src.as_bytes(),
        )
        .unwrap();
        assert_eq!(p.session_id.as_deref(), Some("conv-1"));
        let kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::User,
                EventKind::Assistant,
                EventKind::ToolCall,
                EventKind::ToolResult,
                EventKind::ToolResult
            ]
        );
        assert_eq!(p.events[3].tool.as_deref(), Some("run_command"));
        assert_eq!(p.events[3].is_error, Some(true));
        assert_eq!(p.events[4].is_error, Some(true));
        assert_eq!(p.events[4].text.as_deref(), Some("quota"));
    }
}
