use std::fs;

use distill_core::{Config, Home, SourceKind, normalize, sweep};
use serde_json::{Value, json};

fn fixture() -> (tempfile::TempDir, Home) {
    let tmp = tempfile::tempdir().unwrap();
    let pi = tmp.path().join("pi");
    fs::create_dir_all(&pi).unwrap();
    fs::write(
        pi.join("2026-11-01_p1.jsonl"),
        r#"{"type":"session","id":"p1","cwd":"/r","version":3}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"rename the widget"}],"timestamp":1794436930000}}
{"type":"message","message":{"role":"assistant","model":"glm-5","content":[{"type":"text","text":"Renamed."}],"timestamp":1794436931000}}
"#,
    )
    .unwrap();
    let home = Home::new(tmp.path().join("home"));
    home.ensure().unwrap();
    let mut c = Config {
        sources: vec![],
        ..Default::default()
    };
    let s = c.source_mut(SourceKind::Pi);
    s.enabled = true;
    s.roots = vec![pi];
    c.save(&home.config_path()).unwrap();
    sweep::sweep(&home, &c, None).unwrap();
    normalize::normalize(&home, false).unwrap();
    (tmp, home)
}

fn call(home: &Home, id: u64, method: &str, params: Value) -> Value {
    distill_mcp::handle(
        home,
        &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
    )
    .unwrap()
}

fn tool(home: &Home, name: &str, args: Value) -> Value {
    let r = call(
        home,
        9,
        "tools/call",
        json!({"name": name, "arguments": args}),
    );
    assert_eq!(r["result"]["isError"], false, "{r}");
    serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn initialize_list_and_call_tools() {
    let (_t, home) = fixture();
    let init = call(
        &home,
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "distill");
    assert!(
        distill_mcp::handle(
            &home,
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .is_none()
    );

    let tools = call(&home, 2, "tools/list", json!({}));
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "distill_stats",
            "distill_list_sessions",
            "distill_read_session",
            "distill_search"
        ]
    );

    let list = tool(&home, "distill_list_sessions", json!({}));
    assert_eq!(list[0]["session_id"], "p1");
    let read = tool(
        &home,
        "distill_read_session",
        json!({"source": "pi", "session_id": "p1"}),
    );
    assert_eq!(read["total_events"], 2);
    assert_eq!(read["events"][1]["text"], "Renamed.");
    let hits = tool(&home, "distill_search", json!({"query": "WIDGET"}));
    assert_eq!(hits[0]["session_id"], "p1");
    let stats = tool(&home, "distill_stats", json!({}));
    assert_eq!(stats["totals"]["sessions"], 1);

    let bad = call(
        &home,
        3,
        "tools/call",
        json!({"name": "distill_read_session", "arguments": {"source": "nope", "session_id": "x"}}),
    );
    assert_eq!(bad["result"]["isError"], true);
    let missing = call(&home, 4, "no/such", json!({}));
    assert_eq!(missing["error"]["code"], -32601);
}

#[test]
fn serves_over_newline_delimited_stdio() {
    let (_t, home) = fixture();
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\nnot json\n";
    let mut out = Vec::new();
    distill_mcp::serve(&home, &input[..], &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["result"], json!({}));
    assert_eq!(lines[1]["error"]["code"], -32700);
}
