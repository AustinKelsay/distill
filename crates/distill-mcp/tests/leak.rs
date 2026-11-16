//! The leak test's MCP half (issue #72, "Testing Decisions"): a session with planted secrets is
//! indexed, then every MCP tool the server lists is called by trusted readers, local and remote,
//! and no tool's output holds a value; each reads a placeholder instead. A new tool must say here
//! what it is called with, or this test fails. The secrets are fake, and built from
//! pieces at run time so that no secret scanner mistakes this file for a leak.

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
    let mut c = home.load_config().unwrap_or_default();
    distill_core::trust::add(&mut c, "example-cloud", true).unwrap();
    c.save(&home.config_path()).unwrap();

    distill_core::index::build(&home).unwrap();

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
        let listed = s
            .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .unwrap();
        let mut texts = std::collections::BTreeMap::new();
        for tool in listed["result"]["tools"].as_array().unwrap() {
            let tool = tool["name"].as_str().unwrap();
            let calls = match tool {
                "list_sessions" => vec![json!({})],
                "read_session" => vec![json!({"session": "claude-code/s1"})],
                "search_sessions" => vec![
                    json!({"query": "deploy"}),
                    json!({"query": "anthropic_key"}),
                    json!({"query": key}),
                    json!({"query": password}),
                ],
                other => panic!("the leak test does not call {other}: add what it is called with"),
            };
            for args in calls {
                let r: Value = s
                    .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                        "params": {"name": tool, "arguments": args}}))
                    .unwrap();
                let whole = r.to_string();
                assert!(
                    !whole.contains(&key) && !whole.contains(&password),
                    "{reader:?} {tool} {args}: {whole}"
                );
                let text = r["result"]["content"][0]["text"].as_str().unwrap();
                texts.insert(format!("{tool} {args}"), text.to_string());
            }
        }
        let text = |k: &str| texts[k].as_str();
        assert!(
            text("list_sessions {}").contains("deploy with [secret: anthropic_key #"),
            "{reader:?}: {texts:#?}"
        );
        assert!(
            text(r#"read_session {"session":"claude-code/s1"}"#)
                .contains("DB_PASSWORD=[secret: assigned_secret #"),
            "{reader:?}: {texts:#?}"
        );
        for q in ["deploy", "anthropic_key"] {
            let t = text(&format!(r#"search_sessions {{"query":"{q}"}}"#));
            assert!(
                t.contains("[secret: anthropic_key #"),
                "{reader:?} {q}: {t}"
            );
        }
        // The values are not in the index, so searching for one finds nothing.
        for v in [&key, &password] {
            let t = text(&format!(r#"search_sessions {{"query":"{v}"}}"#));
            assert!(t.starts_with("No sessions match."), "{reader:?}: {t}");
        }
    }
}
