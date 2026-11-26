//! The leak test's MCP half (issue #72, "Testing Decisions"): a session with planted secrets is
//! indexed, then listed, searched and read through every MCP tool by trusted readers, local and
//! remote, and no tool's output holds a value; each reads a placeholder instead. A tool the
//! server lists that this test does not call fails it. A reveal request that the owner
//! approves releases the values in the collection's output, once, and nowhere else. The session
//! has a card: a model whose reply echoes a secret's value has that card discarded, and the card
//! written after it holds only the placeholder. The session's vectors are built, and searches
//! go by meaning too: no text sent to the embedding model, the stored session's or a query's,
//! holds a value. The local model's pass finds a made-up name, and its answers that name a value
//! or a placeholder are dropped. Afterwards no file under the home but the raw archive and the
//! session's events holds a value, and no findings file, state file or log holds the name. The
//! secrets are fake, and built from pieces at
//! run time so that no secret scanner mistakes this file for a leak.

use distill_core::archive::Archive;
use distill_core::runtime::{FakeRuntime, Stored};
use distill_core::trust::Reader;
use distill_core::{Harness, Home};
use distill_mcp::Server;
use serde_json::{Value, json};

#[test]
fn no_tool_serves_a_planted_secret() {
    let key = ["sk-", "ant-", "api03-", &"Mq4".repeat(9)].concat();
    let password = ["plaintext", "-mcp-", "pass-2208"].concat();
    // A token written as a JSON object key, and a number under a credential-named key (eight
    // digits: a shorter one is a counter, not a secret).
    let key_token = ["ghp_", &"Wd3".repeat(12)].concat();
    let pin: u64 = 90_000_000 + 271_828;
    let secrets = [
        key.clone(),
        password.clone(),
        key_token.clone(),
        pin.to_string(),
    ];
    let holds = move |text: &str| {
        let text = text.to_lowercase();
        secrets.iter().any(|s| text.contains(&s.to_lowercase()))
    };
    // A made-up name, for the model detection pass to find.
    let name = "Jane Roe";
    let said = serde_json::to_string(&format!("deploy with {key} now and tell {name}")).unwrap();
    let mut input = serde_json::Map::new();
    input.insert(
        "command".into(),
        format!("export DB_PASSWORD={password} && deploy").into(),
    );
    input.insert(key_token.clone(), "on".into());
    input.insert("api_secret".into(), pin.into());
    let input = Value::Object(input);
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
    distill_core::findings::scan_all(&home, &home.load_config().unwrap_or_default()).unwrap();
    let mut c = home.load_config().unwrap_or_default();
    distill_core::trust::add(&mut c, "example-cloud", true).unwrap();
    c.save(&home.config_path()).unwrap();

    // Cards: a reply that echoes a value is discarded; the next, with the placeholder, is kept.
    let mut config = home.load_config().unwrap();
    config.harnesses.clear();
    config.steward.card_model = Some("small:8b".into());
    let card_reply = |title: &str, summary: &str| {
        json!({"title": title, "summary": summary, "tags": ["deploy"], "outcome": "done"})
            .to_string()
    };
    let (echo, held) = (key.clone(), holds.clone());
    let leaky = FakeRuntime::running()
        .with_model("small:8b", Stored::Here)
        .replying(move |_, prompt| {
            assert!(!held(prompt), "{prompt}");
            card_reply(&format!("Deploy with {echo}"), "Deployed.")
        });
    let r = distill_core::cards::write(
        &home,
        &config,
        &leaky.runtime(),
        distill_core::tick::Deadline::NONE,
    )
    .unwrap();
    assert_eq!((r.written, r.discarded), (0, 1), "{r:?}");
    assert!(!home.card_path(None, Harness::ClaudeCode, "s1").exists());
    let careful = FakeRuntime::running()
        .with_model("small:8b", Stored::Here)
        .replying(move |_, prompt| {
            let at = prompt.find("[secret: anthropic_key #").unwrap();
            let placeholder = &prompt[at..at + prompt[at..].find(']').unwrap() + 1];
            card_reply(
                "Deploy the app",
                &format!("The owner asked to deploy with {placeholder} and it was done."),
            )
        });
    let r = distill_core::cards::write(
        &home,
        &config,
        &careful.runtime(),
        distill_core::tick::Deadline::NONE,
    )
    .unwrap();
    assert_eq!(r.written, 1, "{r:?}");

    // Personal details: the model is never served a value, and an answer that names one (it
    // cannot know it, but may guess) or a placeholder is never a finding. Only the name is.
    let (echo, served, held) = (key.clone(), password.clone(), holds.clone());
    let finder = FakeRuntime::running()
        .with_model("small:8b", Stored::Here)
        .replying(move |_, prompt| {
            assert!(!held(prompt), "{prompt}");
            assert!(
                !prompt.contains("DB_PASSWORD"),
                "tool input was sent: {prompt}"
            );
            json!({"details": [
                {"kind": "person_name", "text": name},
                {"kind": "organization", "text": echo},
                {"kind": "organization", "text": served},
                {"kind": "organization", "text": "[secret: anthropic_key"},
            ]})
            .to_string()
        });
    let r = distill_core::details::find(
        &home,
        &config,
        &finder.runtime(),
        distill_core::tick::Deadline::NONE,
    )
    .unwrap();
    assert_eq!((r.written, r.found), (1, 1), "{r:?}");

    distill_core::index::build(&home).unwrap();

    // Vectors: nothing sent to the embedding model holds a value, from the session or a query.
    config.steward.embedding_model = Some("embed:small".into());
    let mut saved = home.load_config().unwrap();
    saved.steward.embedding_model = config.steward.embedding_model.clone();
    saved.save(&home.config_path()).unwrap();
    let held = holds.clone();
    let embedder = FakeRuntime::running()
        .with_model("small:8b", Stored::Here)
        .with_model("embed:small", Stored::Here)
        .embedding(move |text| {
            assert!(!held(text), "embedded: {text}");
            vec![1.0, text.len() as f32]
        });
    let r = distill_core::index::vectors::build(
        &home,
        &config,
        &embedder.runtime(),
        distill_core::tick::Deadline::NONE,
    )
    .unwrap();
    assert_eq!(r.embedded, 1, "{r:?}");

    let mut last_read = String::new();
    for reader in [
        Reader::model(Some("ollama"), Some("small:8b")),
        Reader::model(Some("example-cloud"), Some("m")),
    ] {
        let s = Server {
            home: home.clone(),
            runtime: embedder.runtime(),
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
        // The four secrets and the name.
        assert_eq!(shorts.len(), 5, "{reader:?}");
        let mut calls = vec![
            ("list_sessions", json!({})),
            ("read_session", json!({"session": "claude-code/s1"})),
            ("list_findings", json!({"kind": "all"})),
        ];
        for short in &shorts {
            calls.push(("list_findings", json!({"fingerprint": short})));
        }
        calls.push(("list_tags", json!({})));
        // Searches for the words around the secrets, a placeholder's kind, and each value.
        for q in ["deploy", "anthropic_key", key.as_str(), password.as_str()] {
            calls.push(("search_sessions", json!({ "query": q })));
        }
        // The reveal tools are called below.
        let listed = s
            .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .unwrap();
        for t in listed["result"]["tools"].as_array().unwrap() {
            let name = t["name"].as_str().unwrap();
            assert!(
                calls.iter().any(|(c, _)| *c == name)
                    || ["request_reveal", "collect_reveal"].contains(&name),
                "the leak test does not call {name}: add what it is called with"
            );
        }
        for (tool, args) in calls {
            let r: Value = s
                .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": tool, "arguments": args}}))
                .unwrap();
            let whole = r.to_string();
            assert!(!holds(&whole), "{reader:?} {tool}: {whole}");
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
        for kind in ["github_token", "credential_field"] {
            assert!(
                texts[1].contains(&format!("[secret: {kind} #")),
                "{reader:?}: {}",
                texts[1]
            );
        }
        for t in &texts[texts.len() - 4..texts.len() - 2] {
            assert!(
                t.contains("deploy with [secret: anthropic_key #"),
                "{reader:?}: {t}"
            );
        }
        // The values are not in the index, so a search for one finds nothing, and a query that
        // holds one is not embedded.
        for t in &texts[texts.len() - 2..] {
            assert!(t.starts_with("No sessions match."), "{reader:?}: {t}");
            assert!(
                t.contains("the query holds a secret, which is never embedded"),
                "{reader:?}: {t}"
            );
        }
        // The other searches went by meaning too.
        assert!(
            !texts[texts.len() - 4].contains("Search by meaning is off"),
            "{reader:?}: {}",
            texts[texts.len() - 4]
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
    assert_eq!(fingerprints.len(), 4, "{last_read}");
    let (err, receipt, whole) = call(
        "request_reveal",
        json!({"session": "claude-code/s1", "fingerprints": fingerprints, "reason": "rotate them"}),
    );
    assert!(!err, "{receipt}");
    assert!(!holds(&whole), "{whole}");
    let id = receipt.split_whitespace().nth(2).unwrap().to_string();
    let (err, early, whole) = call("collect_reveal", json!({"request": id}));
    assert!(err && early.contains("not approved yet"), "{early}");
    assert!(!holds(&whole), "{whole}");

    // The owner approves, in the GUI; no tool can.
    distill_core::reveal::approve(&home, &id).unwrap();
    let (err, values, _) = call("collect_reveal", json!({"request": id}));
    assert!(!err, "{values}");
    for s in [&key, &password, &key_token, &pin.to_string()] {
        assert!(values.contains(s.as_str()), "{values}");
    }
    let (err, again, whole) = call("collect_reveal", json!({"request": id}));
    assert!(err && again.contains("already collected"), "{again}");
    assert!(!holds(&whole), "{whole}");

    let embeds = embedder
        .calls()
        .iter()
        .filter(|c| matches!(c, distill_core::runtime::Call::Embed { .. }))
        .count();
    assert_eq!(
        embeds,
        1 + 2 * 2,
        "the session, then two searches per reader"
    );
    assert!(home.vectors_path().exists());

    // On disk, only the raw archive and the session's events hold a value: not its card, its
    // findings, the index, the vectors, the logs or the reveal request.
    let session = home.session_dir(None, Harness::ClaudeCode, "s1");
    let (mut stack, mut in_events) = (vec![home.root.clone()], false);
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().filter_map(Result::ok) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let text = String::from_utf8_lossy(&std::fs::read(&p).unwrap()).to_lowercase();
            // Findings, state and logs never hold a personal detail's value either.
            if p.ends_with("findings.json")
                || p.starts_with(home.state_dir())
                || p.starts_with(home.runs_dir())
                || p.starts_with(home.log_dir())
            {
                assert!(
                    !text.contains(&name.to_lowercase()),
                    "{} holds the name",
                    p.display()
                );
            }
            if !holds(&text) {
                continue;
            }
            let events = p.parent() == Some(session.as_path())
                && !["card.json", "findings.json", "meta.json"]
                    .iter()
                    .any(|n| p.ends_with(n));
            assert!(
                p.starts_with(home.raw_dir()) || events,
                "{} holds a value",
                p.display()
            );
            in_events |= events;
        }
    }
    assert!(in_events, "the walk found the session's events");
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
