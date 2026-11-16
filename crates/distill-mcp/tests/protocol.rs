use distill_core::Home;
use serde_json::{Value, json};

fn home() -> (tempfile::TempDir, Home) {
    let tmp = tempfile::tempdir().unwrap();
    let home = Home::new(tmp.path().join("home"));
    (tmp, home)
}

fn call(home: &Home, id: u64, method: &str, params: Value) -> Value {
    distill_mcp::handle(
        home,
        &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
    )
    .unwrap()
}

#[test]
fn the_shell_answers_the_protocol_and_exposes_nothing() {
    let (_t, home) = home();
    let init = call(
        &home,
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "distill");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(
        distill_mcp::handle(
            &home,
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .is_none()
    );

    let tools = call(&home, 2, "tools/list", json!({}));
    assert_eq!(tools["result"]["tools"], json!([]));

    // The previous build's tools are gone.
    for name in [
        "distill_stats",
        "distill_list_sessions",
        "distill_read_session",
        "distill_search",
    ] {
        let r = call(
            &home,
            3,
            "tools/call",
            json!({"name": name, "arguments": {}}),
        );
        assert_eq!(r["result"]["isError"], true, "{r}");
    }
    let missing = call(&home, 4, "no/such", json!({}));
    assert_eq!(missing["error"]["code"], -32601);
    assert!(!home.root.exists(), "the shell writes nothing");
}

#[test]
fn serves_over_newline_delimited_stdio() {
    let (_t, home) = home();
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
