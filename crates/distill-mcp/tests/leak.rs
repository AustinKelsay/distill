//! The leak test's MCP half (issue #72, "Testing Decisions"): a session with planted secrets is
//! listed and read through every MCP tool by trusted readers, local and remote, and no tool's
//! output holds a value; each reads a placeholder instead. A reveal request that the owner
//! approves releases the values in the collection's output, once, and nowhere else. The secrets
//! are fake, and built from pieces at run time so that no secret scanner mistakes this file for
//! a leak.

use distill_core::archive::Archive;
use distill_core::runtime::{FakeRuntime, Stored};
use distill_core::trust::Reader;
use distill_core::{Harness, Home};
use distill_mcp::Server;
use serde_json::{Value, json};

#[test]
fn no_tool_serves_a_planted_secret() {
    let key = ["sk-", "ant-", "api03-", &"Mq4".repeat(9)].concat();
    let password = ["synthetic", "-mcp-", "pass-2208"].concat();
    let said = serde_json::to_string(&format!("deploy with {key} now")).unwrap();
    let input = json!({"command": format!("export DB_PASSWORD={password} && deploy")});
    let session = format!(
        r#"{{"type":"user","cwd":"/work/app","timestamp":"2026-11-10T10:00:00Z","message":{{"role":"user","content":{said}}}}}
{{"type":"assistant","timestamp":"2026-11-10T10:00:05Z","message":{{"role":"assistant","model":"m","content":[{{"type":"tool_use","id":"t1","name":"Bash","input":{input}}}]}}}}
"#
    );

    let tmp = tempfile::tempdir().unwrap();
    let home = Home::new(tmp.path().join("home"));
    home.ensure().unwrap();
    Archive::open(&home)
        .unwrap()
        .put(
            Harness::ClaudeCode,
            "/s/proj/s1.jsonl",
            "1",
            session.as_bytes(),
        )
        .unwrap();
    distill_core::normalize::normalize(&home, false).unwrap();
    distill_core::findings::scan_all(&home).unwrap();
    let mut c = home.load_config().unwrap_or_default();
    distill_core::trust::add(&mut c, "example-cloud", true).unwrap();
    c.save(&home.config_path()).unwrap();

    let mut last_read = String::new();
    for reader in [
        Reader::model(Some("ollama"), Some("small:8b")),
        Reader::model(Some("example-cloud"), Some("m")),
    ] {
        let s = Server {
            home: home.clone(),
            runtime: FakeRuntime::stopped()
                .with_model("small:8b", Stored::Here)
                .runtime(),
            reader: reader.clone(),
        };
        let mut texts = Vec::new();
        // Each fingerprint's short form, as `list_findings` shows it, looked up in turn.
        let shorts: Vec<String> = {
            let r = s
                .call_tool("list_findings", &json!({"kind": "all"}))
                .unwrap();
            r.split('#').skip(1).map(|p| p[..8].to_string()).collect()
        };
        assert_eq!(shorts.len(), 2, "{reader:?}");
        let mut calls = vec![
            ("list_sessions", json!({})),
            ("read_session", json!({"session": "claude-code/s1"})),
            ("list_findings", json!({"kind": "all"})),
        ];
        for short in &shorts {
            calls.push(("list_findings", json!({"fingerprint": short})));
        }
        for (tool, args) in calls {
            let r: Value = s
                .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": tool, "arguments": args}}))
                .unwrap();
            let whole = r.to_string();
            assert!(
                !whole.contains(&key) && !whole.contains(&password),
                "{reader:?} {tool}: {whole}"
            );
            texts.push(
                r["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
        }
        assert!(
            texts[0].contains("deploy with [secret: anthropic_key #"),
            "{reader:?}: {}",
            texts[0]
        );
        assert!(
            texts[1].contains("DB_PASSWORD=[secret: assigned_secret #"),
            "{reader:?}: {}",
            texts[1]
        );
        last_read = texts[1].clone();
    }

    // A reveal: requested by the remote reader, approved by the owner, collected once.
    let s = Server {
        home: home.clone(),
        runtime: FakeRuntime::stopped().runtime(),
        reader: Reader::model(Some("example-cloud"), Some("m")),
    };
    let call = |tool: &str, args: Value| -> (bool, String, String) {
        let r = s
            .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": tool, "arguments": args}}))
            .unwrap();
        let text = r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        (r["result"]["isError"] == true, text, r.to_string())
    };
    let fingerprints: Vec<String> = fingerprints_in(&last_read);
    assert_eq!(fingerprints.len(), 2, "{last_read}");
    let (err, receipt, whole) = call(
        "request_reveal",
        json!({"session": "claude-code/s1", "fingerprints": fingerprints, "reason": "rotate them"}),
    );
    assert!(!err, "{receipt}");
    assert!(
        !whole.contains(&key) && !whole.contains(&password),
        "{whole}"
    );
    let id = receipt.split_whitespace().nth(2).unwrap().to_string();
    let (err, early, whole) = call("collect_reveal", json!({"request": id}));
    assert!(err && early.contains("not approved yet"), "{early}");
    assert!(
        !whole.contains(&key) && !whole.contains(&password),
        "{whole}"
    );

    // The owner approves, in the GUI; no tool can.
    distill_core::reveal::approve(&home, &id).unwrap();
    let (err, values, _) = call("collect_reveal", json!({"request": id}));
    assert!(!err, "{values}");
    assert!(
        values.contains(&key) && values.contains(&password),
        "{values}"
    );
    let (err, again, whole) = call("collect_reveal", json!({"request": id}));
    assert!(err && again.contains("already collected"), "{again}");
    assert!(
        !whole.contains(&key) && !whole.contains(&password),
        "{whole}"
    );
}

/// The fingerprints of the placeholders in `text`, in order, each once.
fn fingerprints_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split("[secret: ").skip(1) {
        let fp = part.split('#').nth(1).unwrap_or_default();
        let fp: String = fp.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if !out.contains(&fp) {
            out.push(fp);
        }
    }
    out
}
