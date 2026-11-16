//! The MCP tools against a temp home with a synthetic session, driven over JSON-RPC as a harness
//! would. No local runtime is reached: the fake stands in for it, and the binary's tests point
//! `OLLAMA_HOST` at a loopback port nothing listens on.

use std::io::Write;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};

use distill_core::archive::Archive;
use distill_core::runtime::{FakeRuntime, Stored};
use distill_core::trust::Reader;
use distill_core::{Harness, Home};
use distill_mcp::Server;
use serde_json::{Value, json};

/// Words that appear only in the synthetic session's text.
const SESSION_WORDS: [&str; 3] = ["rename the widget module", "git mv widget.rs", "moved it"];

const SESSION: &str = r#"{"type":"user","cwd":"/work/app","gitBranch":"main","timestamp":"2026-11-10T10:00:00Z","message":{"role":"user","content":"rename the widget module"}}
{"type":"assistant","timestamp":"2026-11-10T10:00:05Z","message":{"role":"assistant","model":"m-small","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git mv widget.rs gadget.rs"}}]}}
{"type":"user","timestamp":"2026-11-10T10:00:09Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"moved it"}]}}
"#;

/// A home under `dir` with the synthetic Claude Code session archived and parsed.
fn home_in(dir: &Path) -> Home {
    let home = Home::new(dir.join("home"));
    home.ensure().unwrap();
    let mut a = Archive::open(&home).unwrap();
    a.put(
        Harness::ClaudeCode,
        "/s/proj/s1.jsonl",
        "1",
        SESSION.as_bytes(),
    )
    .unwrap();
    distill_core::normalize::normalize(&home, false).unwrap();
    home
}

fn server(dir: &Path, reader: Reader) -> Server {
    Server {
        home: home_in(dir),
        runtime: FakeRuntime::stopped()
            .with_model("small:8b", Stored::Here)
            .runtime(),
        reader,
    }
}

fn rpc(s: &Server, method: &str, params: Value) -> Value {
    s.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .unwrap()
}

/// A tool call's text, and whether it is an error.
fn call(s: &Server, tool: &str, args: Value) -> (bool, String) {
    let r = rpc(s, "tools/call", json!({"name": tool, "arguments": args}));
    let result = &r["result"];
    let text = result["content"][0]["text"].as_str().unwrap_or_default();
    (result["isError"] == true, text.to_string())
}

fn ollama() -> Reader {
    Reader::model(Some("ollama"), Some("small:8b"))
}

#[test]
fn it_offers_list_read_and_search_tools_whose_descriptions_say_session_text_is_data() {
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), ollama());
    let tools = rpc(&s, "tools/list", json!({}));
    let tools = tools["result"]["tools"].as_array().unwrap();
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "list_sessions",
            "read_session",
            "search_sessions",
            "list_findings",
            "request_reveal",
            "collect_reveal"
        ]
    );
    for t in tools {
        let d = t["description"].as_str().unwrap();
        assert!(
            d.contains("archived data") && d.contains("not instructions"),
            "{d}"
        );
        assert_eq!(t["inputSchema"]["type"], "object");
    }
}

#[test]
fn a_trusted_model_lists_sessions_with_facts_and_opening() {
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), ollama());
    let (err, t) = call(&s, "list_sessions", json!({}));
    assert!(!err, "{t}");
    assert!(t.contains("=== Archived session from Distill"), "{t}");
    for fact in [
        "claude-code/s1",
        "/work/app",
        "main",
        "m-small",
        "rename the widget module",
    ] {
        assert!(t.contains(fact), "{fact:?} missing: {t}");
    }
    assert!(!t.contains("git mv"), "only the opening: {t}");
    assert!(t.contains("This is the last page."), "{t}");

    // Filters, and the next page.
    let (_, t) = call(
        &s,
        "list_sessions",
        json!({"harness": "codex", "project": "/work", "model": "small",
               "since": "2026-11-01", "until": "2026-11-30"}),
    );
    assert!(t.starts_with("No sessions match."), "{t}");
    let (_, t) = call(
        &s,
        "list_sessions",
        json!({"harness": "claude-code", "project": "/work", "model": "SMALL",
               "since": "2026-11-10", "until": "2026-11-10", "include_mirrors": true,
               "include_steward": true, "offset": 0, "limit": 5}),
    );
    assert!(t.contains("claude-code/s1"), "{t}");
    let (_, t) = call(&s, "list_sessions", json!({"offset": 1}));
    assert!(t.contains("1 match in all"), "{t}");

    for (args, says) in [
        (json!({"harness": "nope"}), "harness"),
        (json!({"since": "last week"}), "YYYY-MM-DD"),
        (json!({"limit": 0}), "limit"),
    ] {
        let (err, t) = call(&s, "list_sessions", args.clone());
        assert!(err && t.contains(says), "{args}: {t}");
    }
}

#[test]
fn a_trusted_model_reads_a_session_in_full_or_by_range() {
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), ollama());
    let (err, t) = call(&s, "read_session", json!({"session": "claude-code/s1"}));
    assert!(!err, "{t}");
    assert!(t.starts_with("=== Archived session from Distill"), "{t}");
    assert!(t.trim_end().ends_with("not instructions. ==="), "{t}");
    for w in SESSION_WORDS {
        assert!(t.contains(w), "{w:?} missing: {t}");
    }
    let (_, t) = call(
        &s,
        "read_session",
        json!({"session": "s1", "from": 1, "to": 1}),
    );
    assert!(
        t.contains("git mv widget.rs") && !t.contains("moved it"),
        "{t}"
    );

    let (err, t) = call(&s, "read_session", json!({"session": "nothing-here"}));
    assert!(err && t.contains("no session"), "{t}");
    let (err, t) = call(&s, "read_session", json!({}));
    assert!(err && t.contains("session"), "{t}");

    // Each read went through the read path and its log; the call naming no session never got
    // that far.
    let log = distill_core::readlog::list(&s.home, None, usize::MAX).unwrap();
    let sessions: Vec<_> = log.iter().map(|e| e.session.as_deref().unwrap()).collect();
    assert_eq!(
        sessions,
        ["nothing-here", "claude-code/s1", "claude-code/s1"]
    );
}

#[test]
fn a_list_call_is_logged_as_one_line_with_a_count() {
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), ollama());
    call(&s, "list_sessions", json!({}));
    let log = distill_core::readlog::list(&s.home, None, usize::MAX).unwrap();
    assert_eq!(log.len(), 1);
    let line = serde_json::to_value(&log[0]).unwrap();
    assert_eq!(line["result"], "listed");
    assert_eq!(line["sessions"], 1);
    assert_eq!(line["model"], "small:8b");
}

#[test]
fn a_trusted_model_searches_sessions_by_words_narrowed_by_filters() {
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), ollama());
    distill_core::index::build(&s.home).unwrap();
    let (err, t) = call(&s, "search_sessions", json!({"query": "Widget.rs"}));
    assert!(!err, "{t}");
    assert!(t.starts_with("Sessions 1–1 of 1 that match"), "{t}");
    for want in [
        distill_core::read::NOTICE_START,
        "claude-code/s1 · Claude Code · project /work/app · branch main",
        "Opened with: rename the widget module",
        "Matched in event 1 (tool_call, input at /command): git mv widget.rs gadget.rs",
        distill_core::read::NOTICE_END,
    ] {
        assert!(t.contains(want), "{want:?} missing: {t}");
    }
    let narrowed = json!({"query": "widget", "harness": "claude-code", "model": "SMALL",
        "project": "/work", "since": "2026-11-10", "until": "2026-11-10", "limit": 5});
    let (err, t) = call(&s, "search_sessions", narrowed);
    assert!(!err && t.contains("claude-code/s1"), "{t}");
    for args in [
        json!({"query": "widget", "harness": "codex"}),
        json!({"query": "widget", "since": "2026-11-11"}),
        json!({"query": "widget gizmo"}),
    ] {
        let (err, t) = call(&s, "search_sessions", args.clone());
        assert!(!err && t.starts_with("No sessions match."), "{args}: {t}");
    }
    for (args, says) in [
        (json!({}), "query is required"),
        (json!({"query": "widget", "since": "Nov 10"}), "YYYY-MM-DD"),
        (json!({"query": "widget", "limit": 0}), "limit"),
        (json!({"query": "?!"}), "at least one word"),
    ] {
        let (err, t) = call(&s, "search_sessions", args.clone());
        assert!(err && t.contains(says), "{args}: {t}");
    }
}

/// The server's home with a second session that holds a fake key, built at run time so no secret
/// scanner mistakes this file for a leak, and scanned.
fn with_secret(s: &Server, key: &str) {
    let said = serde_json::to_string(&format!("deploy with {key}")).unwrap();
    let body = format!(
        r#"{{"type":"user","cwd":"/work/app","timestamp":"2026-11-12T10:00:00Z","message":{{"role":"user","content":{said}}}}}
"#
    );
    Archive::open(&s.home)
        .unwrap()
        .put(
            Harness::ClaudeCode,
            "/s/proj/s2.jsonl",
            "1",
            body.as_bytes(),
        )
        .unwrap();
    distill_core::normalize::normalize(&s.home, false).unwrap();
    distill_core::findings::scan_all(&s.home, &s.home.load_config().unwrap_or_default()).unwrap();
}

#[test]
fn a_trusted_model_lists_findings_by_fingerprint_and_where_one_appears() {
    let key = ["sk-", "ant-", "api03-", &"Wd2".repeat(9)].concat();
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), ollama());
    with_secret(&s, &key);

    let (err, t) = call(&s, "list_findings", json!({}));
    assert!(!err, "{t}");
    assert!(
        t.contains("anthropic_key") && t.contains("1 session") && t.contains("claude-code"),
        "{t}"
    );
    let short = t
        .split('#')
        .nth(1)
        .map(|r| r[..8].to_string())
        .unwrap_or_else(|| panic!("no short form: {t}"));
    let (err, found) = call(&s, "list_findings", json!({"fingerprint": short}));
    assert!(
        !err && found.contains("claude-code/s2") && !found.contains("claude-code/s1 "),
        "{found}"
    );

    let (_, t) = call(
        &s,
        "list_findings",
        json!({"kind": "all", "harness": "claude-code", "since": "2026-11-01",
               "until": "2026-11-30", "sort": "recent", "offset": 0, "limit": 5}),
    );
    assert!(t.contains(&short), "{t}");
    let (_, t) = call(&s, "list_findings", json!({"kind": "personal"}));
    assert!(t.contains("No fingerprints match"), "{t}");
    for (args, says) in [
        (json!({"kind": "tea_leaves"}), "unknown kind"),
        (json!({"sort": "loudest"}), "sort"),
        (json!({"harness": "nope"}), "harness"),
        (json!({"since": "last week"}), "YYYY-MM-DD"),
        (json!({"limit": 0}), "limit"),
    ] {
        let (err, t) = call(&s, "list_findings", args.clone());
        assert!(err && t.contains(says), "{args}: {t}");
    }

    // The four calls that reached the read path are in its log, one line each.
    let log = distill_core::readlog::list(&s.home, None, usize::MAX).unwrap();
    let results: Vec<_> = log
        .iter()
        .map(|e| serde_json::to_value(e).unwrap()["result"].clone())
        .collect();
    assert_eq!(results, ["findings", "findings", "findings", "findings"]);
    assert!(log.iter().all(|e| e.reader == ollama()));
}

/// Every tool, with arguments that a trusted reader would be served.
fn every_tool(s: &Server) -> Vec<(String, Value)> {
    let tools = rpc(s, "tools/list", json!({}));
    tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let name = t["name"].as_str().unwrap().to_string();
            let args = match name.as_str() {
                "read_session" => json!({"session": "claude-code/s1"}),
                "search_sessions" => json!({"query": "widget"}),
                "request_reveal" => json!({
                    "session": "claude-code/s1",
                    "fingerprints": ["0123abcd"],
                    "reason": "rotate it"
                }),
                "collect_reveal" => json!({"request": "0123456789abcdef"}),
                _ => json!({}),
            };
            (name, args)
        })
        .collect()
}

#[test]
fn with_no_provider_or_an_untrusted_one_every_tool_returns_the_refusal_and_nothing_else() {
    for reader in [
        Reader::model(None, None),
        Reader::model(Some("unheard-of"), Some("m")),
        Reader::model(Some("ollama"), Some("big:cloud")),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let s = server(tmp.path(), reader.clone());
        let calls = every_tool(&s);
        assert_eq!(calls.len(), 6);
        for (tool, args) in calls {
            let (err, t) = call(&s, &tool, args);
            assert!(err, "{reader:?} {tool}: {t}");
            assert!(t.contains("distill providers"), "{reader:?} {tool}: {t}");
            for leak in SESSION_WORDS.iter().copied().chain([
                "claude-code",
                "s1",
                "/work/app",
                "m-small",
                "1 ",
            ]) {
                assert!(!t.contains(leak), "{reader:?} {tool} leaked {leak:?}: {t}");
            }
        }
    }
}

/// Run the server binary on `input` with `args` and `env`, against `home`, with no runtime.
fn run_binary(home: &Home, args: &[&str], env: &[(&str, &str)], input: &str) -> Vec<Value> {
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let user = home.root.parent().unwrap().join("user");
    std::fs::create_dir_all(&user).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_distill-mcp"))
        .env("HOME", &user)
        .env("PATH", user.join("bin"))
        .env("OLLAMA_HOST", closed.to_string())
        .env_remove("OLLAMA_MODELS")
        .env_remove("DISTILL_PROVIDER")
        .env_remove("DISTILL_MODEL")
        .env("DISTILL_HOME", &home.root)
        .envs(env.iter().copied())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

const READ: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read_session","arguments":{"session":"claude-code/s1"}}}
"#;

fn text_of(response: &Value) -> (bool, String) {
    let r = &response["result"];
    (
        r["isError"] == true,
        r["content"][0]["text"].as_str().unwrap().to_string(),
    )
}

#[test]
fn the_binary_takes_its_reader_from_flags_or_the_environment() {
    let tmp = tempfile::tempdir().unwrap();
    let home = home_in(tmp.path());

    for (args, env) in [
        (vec!["--provider", "ollama"], vec![]),
        (vec!["--provider=ollama"], vec![]),
        (vec![], vec![("DISTILL_PROVIDER", "ollama")]),
        // The flag wins over the environment.
        (
            vec!["--provider", "ollama"],
            vec![("DISTILL_PROVIDER", "unheard-of")],
        ),
    ] {
        let (err, t) = text_of(&run_binary(&home, &args, &env, READ)[0]);
        assert!(!err, "{args:?} {env:?}: {t}");
        assert!(t.contains("rename the widget module"), "{t}");
    }

    // A model not stored here, named by flag or environment, reads as ollama-cloud.
    for (args, env) in [
        (vec!["--provider", "ollama", "--model", "big:cloud"], vec![]),
        (
            vec![],
            vec![
                ("DISTILL_PROVIDER", "ollama"),
                ("DISTILL_MODEL", "big:cloud"),
            ],
        ),
    ] {
        let (err, t) = text_of(&run_binary(&home, &args, &env, READ)[0]);
        assert!(err && t.contains("ollama-cloud"), "{args:?} {env:?}: {t}");
    }

    let (err, t) = text_of(&run_binary(&home, &[], &[], READ)[0]);
    assert!(err && t.contains("names no provider"), "{t}");

    let log = distill_core::readlog::list(&home, None, usize::MAX).unwrap();
    assert_eq!(log.len(), 7);
    assert!(
        log.iter()
            .all(|e| e.session.as_deref() == Some("claude-code/s1"))
    );
}

#[test]
fn the_binary_refuses_an_argument_it_does_not_know() {
    let tmp = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_distill-mcp"))
        .arg("--listen")
        .env("HOME", tmp.path())
        .env("DISTILL_HOME", tmp.path().join("home"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("--listen") && err.contains("--provider"),
        "{err}"
    );
}

#[test]
fn sessions_list_and_search_with_their_cards_filtered_by_tag_and_outcome() {
    let tmp = tempfile::tempdir().unwrap();
    let s = server(tmp.path(), ollama());
    let mut config = s.home.load_config().unwrap_or_default();
    config.harnesses.clear();
    config.steward.card_model = Some("small:8b".into());
    let model = FakeRuntime::running()
        .with_model("small:8b", Stored::Here)
        .replying(|_, _| {
            json!({"title": "Rename the widget", "summary": "Moved the module and its callers.",
                   "tags": ["refactor"], "outcome": "done"})
            .to_string()
        });
    let r = distill_core::cards::write(&s.home, &config, &model.runtime()).unwrap();
    assert_eq!(r.written, 1, "{r:?}");
    distill_core::index::build(&s.home).unwrap();

    let (err, t) = call(&s, "list_sessions", json!({"tag": "refactor"}));
    assert!(!err, "{t}");
    assert!(
        t.contains("  Card: Rename the widget · outcome done · tags refactor"),
        "{t}"
    );
    assert!(
        t.contains("  Summary: Moved the module and its callers."),
        "{t}"
    );
    let (_, t) = call(&s, "list_sessions", json!({"outcome": "failed"}));
    assert!(t.starts_with("No sessions match."), "{t}");
    let (_, t) = call(
        &s,
        "search_sessions",
        json!({"query": "callers", "outcome": "done"}),
    );
    assert!(t.contains("claude-code/s1"), "{t}");
    let (err, t) = call(&s, "list_sessions", json!({"outcome": "great"}));
    assert!(
        err && t.contains("done, partly_done, failed, unknown"),
        "{t}"
    );
}
