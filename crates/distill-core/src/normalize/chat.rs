//! OpenAI-style chat messages (`role`, `content`, `tool_calls`, `tool_call_id`), shared by
//! harnesses that log the messages they send to the model: Hermes and Grok.

use serde_json::Value;

use super::blocks::{push_content, result_text};
use crate::model::{Event, EventKind, Parsed};

/// Push one message. `role` is the message's role (or Grok's `type`); `ts` its timestamp.
pub fn push_message(out: &mut Parsed, role: &str, m: &Value, ts: Option<String>) {
    let model = m
        .get("model")
        .or_else(|| m.get("model_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(md) = &model {
        out.add_model(md);
    }
    let content = m.get("content").unwrap_or(&Value::Null);
    match role {
        "system" | "developer" => push_content(out, EventKind::System, content, ts, None),
        "user" => push_content(out, EventKind::User, content, ts, None),
        "assistant" => {
            if let Some(r) = m.get("reasoning").and_then(Value::as_str) {
                out.push(Event::text(EventKind::Reasoning, r).at(ts.clone()));
            }
            push_content(
                out,
                EventKind::Assistant,
                content,
                ts.clone(),
                model.as_deref(),
            );
            for call in m
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let f = call.get("function").unwrap_or(call);
                let mut e = Event::new(EventKind::ToolCall).at(ts.clone());
                e.tool = f.get("name").and_then(Value::as_str).map(str::to_string);
                e.call_id = call.get("id").and_then(Value::as_str).map(str::to_string);
                e.input = match f.get("arguments").or_else(|| f.get("input")) {
                    Some(Value::String(s)) => {
                        Some(serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.clone())))
                    }
                    other => other.cloned(),
                };
                e.model = model.clone();
                out.push(e);
            }
        }
        "tool" | "tool_result" => {
            let mut e = Event::new(EventKind::ToolResult).at(ts);
            e.call_id = m
                .get("tool_call_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            e.tool = m.get("name").and_then(Value::as_str).map(str::to_string);
            e.text = Some(result_text(content));
            out.push(e);
        }
        _ => {}
    }
}
