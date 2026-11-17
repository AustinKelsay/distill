//! Droid (Factory): `~/.factory/sessions/<project>/<id>.jsonl`, a `session_start` record then
//! `message` records with Anthropic-style content blocks.

use anyhow::Result;
use serde_json::Value;

use super::blocks::{PlanTracker, first_text, plan_line, push_content, ts};
use crate::model::{Event, EventKind, Parsed};

/// Record types the parser skips on purpose. Any other type it skips is reported as format drift.
///
/// `session_end` repeats the final assistant message (#104).
pub const IGNORED: &[&str] = &["session_end"];

pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    let mut out = Parsed::default();
    let mut plan = PlanTracker::default();
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            "session_start" => {
                out.session_id = v.get("id").and_then(Value::as_str).map(str::to_string);
                out.cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
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
            // The summary the model wrote when the context was compacted.
            "compaction_state" => {
                let body = nested(&v, "compaction_state");
                let summary = first_text(
                    body,
                    &["summaryText", "summary", "compactionSummary", "text"],
                )
                .or_else(|| first_text(&v, &["summaryText", "summary", "text"]));
                if let Some(s) = summary {
                    out.push(Event::text(EventKind::System, s).at(ts(&v)));
                }
            }
            // The model's plan, repeated in full on every update: kept when it changes.
            "todo_state" => {
                let body = nested(&v, "todo_state");
                let items = ["todos", "items", "list"]
                    .iter()
                    .find_map(|k| body.get(*k).or_else(|| v.get(*k)));
                let lines = match items {
                    Some(Value::Array(items)) => items
                        .iter()
                        .filter_map(|t| {
                            let step = first_text(t, &["content", "text", "title", "description"])?;
                            let status = t.get("status").and_then(Value::as_str).unwrap_or("");
                            Some(plan_line(status, step))
                        })
                        .collect(),
                    // A list written as text, one step per line.
                    Some(Value::String(s)) => s.lines().map(|l| l.to_string()).collect(),
                    _ => Vec::new(),
                };
                if let Some(plan) = plan.changed(lines) {
                    out.push(Event::text(EventKind::System, plan).at(ts(&v)));
                }
            }
            _ => out.skip(ty, line.len()),
        }
    }
    Ok(out)
}

/// The object a state record keeps its fields in: under a key named for the record (or
/// `state`, or `data`) when it has one, else the record itself.
fn nested<'a>(v: &'a Value, ty: &str) -> &'a Value {
    let camel: String = ty
        .split('_')
        .enumerate()
        .map(|(i, w)| match (i, w.chars().next()) {
            (0, _) | (_, None) => w.to_string(),
            (_, Some(c)) => c.to_ascii_uppercase().to_string() + &w[c.len_utf8()..],
        })
        .collect();
    [ty, camel.as_str(), "state", "data"]
        .iter()
        .find_map(|k| v.get(*k).filter(|x| x.is_object()))
        .unwrap_or(v)
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
        assert_eq!(p.events.len(), 3);
        assert_eq!(p.events[2].kind, EventKind::ToolResult);
    }

    #[test]
    fn parses_compaction_summaries_and_plans_once_per_change() {
        let src = r#"{"type":"session_start","id":"d2","cwd":"/r"}
{"type":"compaction_state","timestamp":"2026-10-01T00:00:00Z","summaryText":"We fixed the parser."}
{"type":"todo_state","timestamp":"2026-10-01T00:00:01Z","todos":[{"content":"write test","status":"completed"},{"content":"ship","status":"in_progress"}]}
{"type":"todo_state","timestamp":"2026-10-01T00:00:02Z","todos":[{"content":"write test","status":"completed"},{"content":"ship","status":"in_progress"}]}
{"type":"todo_state","timestamp":"2026-10-01T00:00:03Z","todos":[{"content":"write test","status":"completed"},{"content":"ship","status":"completed"}]}"#;
        let p = parse(src.as_bytes()).unwrap();
        assert!(p.skipped.is_empty(), "{:?}", p.skipped);
        assert_eq!(p.events.len(), 3);
        assert!(p.events.iter().all(|e| e.kind == EventKind::System));
        assert_eq!(p.events[0].text.as_deref(), Some("We fixed the parser."));
        assert_eq!(
            p.events[1].text.as_deref(),
            Some("[done] write test\n[in progress] ship")
        );
        assert_eq!(
            p.events[2].text.as_deref(),
            Some("[done] write test\n[done] ship")
        );
    }
}
