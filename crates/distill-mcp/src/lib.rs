//! An MCP server over stdio, so any harness can search and read the archive. Read-only: no tool
//! here writes anything.

use std::io::{BufRead, Write};

use anyhow::Result;
use distill_core::search::{self, ListFilter};
use distill_core::{Home, SourceKind, card, normalize, stats};
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

fn tools() -> Value {
    json!([
        {
            "name": "distill_stats",
            "description": "Totals, per-source health, sessions per week, recent steward runs and items that need the user.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "distill_list_sessions",
            "description": "List archived agent sessions, newest first. Filter by source, by text in the title or project path, or by start date.",
            "inputSchema": {"type": "object", "properties": {
                "source": {"type": "string", "enum": SourceKind::ALL.map(|k| k.id())},
                "text": {"type": "string"},
                "since": {"type": "string", "description": "YYYY-MM-DD"},
                "limit": {"type": "integer", "default": 25, "maximum": 200}
            }}
        },
        {
            "name": "distill_read_session",
            "description": "Read one session: its meta, its card if the steward wrote one, and a page of its events (prompts, replies, tool calls and results).",
            "inputSchema": {"type": "object", "required": ["source", "session_id"], "properties": {
                "source": {"type": "string"},
                "session_id": {"type": "string"},
                "offset": {"type": "integer", "default": 0},
                "limit": {"type": "integer", "default": 50, "maximum": 500},
                "max_chars": {"type": "integer", "default": 2000, "description": "Clip each event's text to this many characters"}
            }}
        },
        {
            "name": "distill_search",
            "description": "Find events whose text contains a query (case-insensitive) across all sessions.",
            "inputSchema": {"type": "object", "required": ["query"], "properties": {
                "query": {"type": "string"},
                "source": {"type": "string"},
                "limit": {"type": "integer", "default": 20, "maximum": 200}
            }}
        }
    ])
}

fn source_arg(args: &Value) -> Result<Option<SourceKind>, String> {
    match args.get("source").and_then(Value::as_str) {
        None | Some("") => Ok(None),
        Some(s) => SourceKind::parse(s)
            .map(Some)
            .ok_or_else(|| format!("unknown source {s:?}")),
    }
}

fn limit(args: &Value, default: u64, max: u64) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .min(max) as usize
}

pub fn call_tool(home: &Home, name: &str, args: &Value) -> Result<Value, String> {
    let err = |e: anyhow::Error| format!("{e:#}");
    match name {
        "distill_stats" => {
            serde_json::to_value(stats::dashboard(home).map_err(err)?).map_err(|e| e.to_string())
        }
        "distill_list_sessions" => {
            let f = ListFilter {
                source: source_arg(args)?,
                text: args.get("text").and_then(Value::as_str).map(str::to_string),
                since: args
                    .get("since")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                include_steward: false,
            };
            let list = search::list(home, &f, limit(args, 25, 200)).map_err(err)?;
            Ok(json!(list
                .iter()
                .map(|m| json!({
                    "source": m.source, "session_id": m.session_id, "title": m.title,
                    "cwd": m.cwd, "started_at": m.started_at, "models": m.models, "counts": m.counts
                }))
                .collect::<Vec<_>>()))
        }
        "distill_read_session" => {
            let source = source_arg(args)?.ok_or("source is required")?;
            let id = args
                .get("session_id")
                .and_then(Value::as_str)
                .ok_or("session_id is required")?;
            let meta = normalize::find(home, source, id)
                .map_err(err)?
                .ok_or("no such session")?;
            let events = normalize::read_events(home, &meta).map_err(err)?;
            let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let max_chars = args
                .get("max_chars")
                .and_then(Value::as_u64)
                .unwrap_or(2000) as usize;
            let page: Vec<Value> = events
                .iter()
                .skip(offset)
                .take(limit(args, 50, 500))
                .map(|e| {
                    let mut e = e.clone();
                    e.text = e.text.map(|t| distill_core::util::clip(&t, max_chars));
                    serde_json::to_value(e).unwrap_or(Value::Null)
                })
                .collect();
            Ok(json!({
                "meta": meta,
                "card": card::load(&normalize::session_dir(home, &meta)),
                "total_events": events.len(),
                "offset": offset,
                "events": page
            }))
        }
        "distill_search" => {
            let q = args
                .get("query")
                .and_then(Value::as_str)
                .ok_or("query is required")?;
            let hits =
                search::search(home, q, source_arg(args)?, limit(args, 20, 200)).map_err(err)?;
            serde_json::to_value(hits).map_err(|e| e.to_string())
        }
        _ => Err(format!("unknown tool {name:?}")),
    }
}

/// Handle one JSON-RPC message. Returns the response, or None for notifications.
pub fn handle(home: &Home, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let id = id?; // notifications get no response
    let result = match method {
        "initialize" => {
            let requested = msg
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            Ok(json!({
                "protocolVersion": requested,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "distill", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Read-only access to the user's archive of AI coding-agent sessions. Start with distill_stats or distill_list_sessions; read a session with distill_read_session."
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = msg
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = msg
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            Ok(match call_tool(home, name, &args) {
                Ok(v) => {
                    json!({"content": [{"type": "text", "text": serde_json::to_string(&v).unwrap_or_default()}], "isError": false})
                }
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            })
        }
        _ => Err(json!({"code": -32601, "message": format!("method not found: {method}")})),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    })
}

/// Serve newline-delimited JSON-RPC on stdin/stdout until stdin closes.
pub fn serve(home: &Home, input: impl BufRead, mut output: impl Write) -> Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(home, &msg),
            Err(e) => Some(
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}}),
            ),
        };
        if let Some(r) = response {
            writeln!(output, "{r}")?;
            output.flush()?;
        }
    }
    Ok(())
}
