//! An MCP server over stdio. Stdio only: it opens no network listener. Each tool is its own
//! module under `tools/`, and every tool that returns session text, session facts or counts gets
//! them from the read path (`distill_core::read`) as the server's reader, so trust, placeholders,
//! the archived-data notice and the read log all apply.

pub mod tools;

use std::io::{BufRead, Write};

use anyhow::{Result, bail};
use distill_core::Home;
use distill_core::runtime::LocalRuntime;
use distill_core::trust::Reader;
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// How to run the server, for its usage line.
pub const USAGE: &str = "usage: distill-mcp [--provider <name>] [--model <name>]\n\
    The provider the harness's model runs at (default: $DISTILL_PROVIDER), and the model when \
    known (default: $DISTILL_MODEL). Name a provider only if every provider used in that harness \
    is trusted.";

/// The server: whose archive, the local runtime, and the one reader every call reads as.
pub struct Server {
    pub home: Home,
    pub runtime: LocalRuntime,
    pub reader: Reader,
}

/// The reader named on the command line (`--provider`, `--model`, each also as `--flag=value`),
/// each falling back to `DISTILL_PROVIDER` and `DISTILL_MODEL`.
pub fn reader_from_args(args: impl IntoIterator<Item = String>) -> Result<Reader> {
    let (mut provider, mut model) = (None, None);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) => (f.to_string(), Some(v.to_string())),
            None => (arg.clone(), None),
        };
        let slot = match flag.as_str() {
            "--provider" => &mut provider,
            "--model" => &mut model,
            _ => bail!("unknown argument {arg:?}\n{USAGE}"),
        };
        match inline.or_else(|| args.next()) {
            Some(v) => *slot = Some(v),
            None => bail!("{flag} needs a value\n{USAGE}"),
        }
    }
    Ok(Reader::from_flags_or_env(
        provider.as_deref(),
        model.as_deref(),
    ))
}

impl Server {
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

    /// Run one tool: its text, or the error the client sees.
    pub fn call_tool(&self, name: &str, args: &Value) -> Result<String, String> {
        match tools::all().into_iter().find(|t| t.name() == name) {
            Some(t) => t.call(self, args),
            None => Err(format!("unknown tool {name:?}")),
        }
    }

    /// Handle one JSON-RPC message. Returns the response, or None for notifications.
    pub fn handle(&self, msg: &Value) -> Option<Value> {
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
                    "instructions": tools::INSTRUCTIONS,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": Self::tool_list()})),
            "tools/call" => {
                let name = msg
                    .pointer("/params/name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let args = msg
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or(json!({}));
                let (text, is_error) = match self.call_tool(name, &args) {
                    Ok(text) => (text, false),
                    Err(e) => (e, true),
                };
                Ok(json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
            }
            _ => Err(json!({"code": -32601, "message": format!("method not found: {method}")})),
        };
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
        })
    }

    /// Serve newline-delimited JSON-RPC on stdin/stdout until stdin closes.
    pub fn serve(&self, input: impl BufRead, mut output: impl Write) -> Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<Value>(&line) {
                Ok(msg) => self.handle(&msg),
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
}
