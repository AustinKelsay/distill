//! An MCP server over stdio. Stdio only: it opens no network listener. Each tool is its own
//! module under `tools/`; for now there are none, so the server answers the protocol and
//! exposes nothing until the read path and search land.

pub mod tools;

use std::io::{BufRead, Write};

use anyhow::Result;
use distill_core::Home;
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

fn tool_list() -> Value {
    json!(
        tools::all()
            .iter()
            .map(|t| json!({
                "name": t.name(),
                "description": t.description(),
                "inputSchema": t.input_schema(),
            }))
            .collect::<Vec<_>>()
    )
}

pub fn call_tool(home: &Home, name: &str, args: &Value) -> Result<Value, String> {
    match tools::all().into_iter().find(|t| t.name() == name) {
        Some(t) => t.call(home, args),
        None => Err(format!("unknown tool {name:?}")),
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
                "instructions": "The owner's archive of agent sessions. It exposes no tools yet."
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tool_list()})),
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
