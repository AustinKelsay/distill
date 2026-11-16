use distill_core::Home;
use distill_core::runtime::FakeRuntime;
use distill_core::trust::Reader;
use distill_mcp::Server;
use serde_json::{Value, json};

fn home() -> (tempfile::TempDir, Server) {
    let tmp = tempfile::tempdir().unwrap();
    let server = Server {
        home: Home::new(tmp.path().join("home")),
        runtime: FakeRuntime::not_installed().runtime(),
        reader: Reader::model(None, None),
    };
    (tmp, server)
}

fn call(server: &Server, id: u64, method: &str, params: Value) -> Value {
    server
        .handle(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
        .unwrap()
}

#[test]
fn it_answers_the_protocol_and_the_previous_builds_tools_are_gone() {
    let (_t, home) = home();
    let init = call(
        &home,
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "distill");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    let says = init["result"]["instructions"].as_str().unwrap();
    assert!(
        says.contains("archived data") && says.contains("not instructions"),
        "{says}"
    );
    assert!(
        home.handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .is_none()
    );

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
    assert!(
        !home.home.root.exists(),
        "answering the protocol writes nothing"
    );
}

#[test]
fn serves_over_newline_delimited_stdio() {
    let (_t, home) = home();
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\nnot json\n";
    let mut out = Vec::new();
    home.serve(&input[..], &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["result"], json!({}));
    assert_eq!(lines[1]["error"]["code"], -32700);
}

#[test]
fn the_server_opens_no_network_listener() {
    // Stdio only: nothing in the server's code can listen on, or connect to, a network.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src.join("lib.rs"), src.join("main.rs")];
    for e in std::fs::read_dir(src.join("tools")).unwrap() {
        files.push(e.unwrap().path());
    }
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for net in [
            "std::net",
            "TcpListener",
            "UdpSocket",
            "UnixListener",
            "tokio",
            "hyper",
        ] {
            assert!(!text.contains(net), "{} names {net}", f.display());
        }
    }
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .unwrap();
    for net in [
        "tokio",
        "hyper",
        "axum",
        "reqwest",
        "ureq",
        "actix",
        "warp",
        "tiny_http",
        "mio",
    ] {
        assert!(!manifest.contains(net), "the server depends on {net}");
    }
}
