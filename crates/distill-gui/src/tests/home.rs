//! A temporary home with secrets in it, for the Findings, Approvals and Read log tests: two
//! synthetic Claude Code sessions, scanned by a tick with the fake runtime, and a trusted
//! provider whose model files reveal requests. Every value is fake and built at run time.

use std::sync::Arc;

use distill_core::detect::Kind;
use distill_core::read::{self, Answer, FindingFilter, Kinds, Order, Page};
use distill_core::reveal::{self, Filed, Request};
use distill_core::runtime::FakeRuntime;
use distill_core::trust::Reader;
use distill_core::{Config, Harness, Home};

use super::*;
use crate::jobs::{Core, Timer};

pub fn fake_key() -> String {
    ["sk-", "ant-", "api03-", &"Rq7".repeat(9)].concat()
}

pub fn fake_password() -> String {
    ["synthetic", "-gui-", "pass-87"].concat()
}

pub fn fake_email() -> String {
    ["robin.example", "@", "example.com"].concat()
}

/// Every fake value in the home.
pub fn values() -> [String; 3] {
    [fake_key(), fake_password(), fake_email()]
}

pub fn cloud() -> Reader {
    Reader::model(Some("example-cloud"), Some("m"))
}

fn session(cwd: &str, at: &str, said: &str, command: &str, result: &str) -> String {
    let said = serde_json::to_string(said).unwrap();
    let input = serde_json::json!({ "command": command });
    let result = serde_json::to_string(result).unwrap();
    format!(
        r#"{{"type":"user","cwd":"{cwd}","version":"2.1.0","timestamp":"{at}","message":{{"role":"user","content":{said}}}}}
{{"type":"assistant","timestamp":"{at}","message":{{"role":"assistant","model":"claude-opus-5-5","content":[{{"type":"tool_use","id":"t1","name":"Bash","input":{input}}}]}}}}
{{"type":"user","timestamp":"{at}","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":{result}}}]}}}}
"#
    )
}

/// `claude-code/aaaa` holds the key (what the owner wrote), a password (a tool call) and an
/// email (a tool result); `claude-code/bbbb` holds the same key again.
pub fn home(dir: &std::path::Path) -> Core {
    let root = dir.join("harness/claude");
    std::fs::create_dir_all(root.join("-w")).unwrap();
    std::fs::write(
        root.join("-w/aaaa.jsonl"),
        session(
            "/work/app",
            "2026-11-10T10:00:00Z",
            &format!("deploy the app with {} please", fake_key()),
            &format!("export DB_PASSWORD={} && deploy", fake_password()),
            &format!("deployed; mail {} about it", fake_email()),
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("-w/bbbb.jsonl"),
        session(
            "/work/ops",
            "2026-11-12T10:00:00Z",
            &format!("rotate {} today", fake_key()),
            "ls",
            "ok",
        ),
    )
    .unwrap();
    let mut config = Config::default();
    for h in &mut config.harnesses {
        h.enabled = false;
        h.roots = vec![dir.join("harness/none")];
    }
    let h = config.harness_mut(Harness::ClaudeCode);
    h.enabled = true;
    h.roots = vec![root];
    distill_core::trust::add(&mut config, "example-cloud", true).unwrap();
    let home = Home::new(dir.join("home"));
    home.ensure().unwrap();
    config.save(&home.config_path()).unwrap();
    let runtime = FakeRuntime::stopped().runtime();
    distill_core::tick::tick_with(&home, &config, &runtime).unwrap();
    Core {
        home,
        runtime,
        timer: Timer {
            install: Arc::new(|_, _| panic!("no test installs the timer here")),
            installed: Arc::new(|| false),
        },
    }
}

/// The short fingerprint of the one value of `kind` in the archive.
pub fn short(core: &Core, kind: Kind) -> String {
    let filter = FindingFilter {
        kinds: Kinds::Only(kind),
        ..FindingFilter::default()
    };
    let page = Page {
        offset: 0,
        limit: 10,
    };
    let Answer::Served(f) = read::fingerprints(
        &core.home,
        &core.runtime,
        &Reader::Owner,
        &filter,
        Order::Spread,
        page,
    )
    .unwrap() else {
        panic!("refused");
    };
    assert_eq!(f.rows.len(), 1, "one {kind:?}");
    f.rows[0].fingerprint.short().to_string()
}

/// File a request as `reader` for these kinds' values in `claude-code/aaaa`.
pub fn file(core: &Core, reader: &Reader, kinds: &[Kind], reason: &str) -> Request {
    let fps: Vec<String> = kinds.iter().map(|k| short(core, *k)).collect();
    match reveal::request(
        &core.home,
        &core.runtime,
        reader,
        "claude-code/aaaa",
        &fps,
        reason,
    )
    .unwrap()
    {
        Filed::Filed(r) => *r,
        Filed::Refused(why) => panic!("refused: {why}"),
    }
}

/// A window over the home, on the overview with its report loaded.
pub fn window(core: &Core) -> Runtime<State> {
    let config = core.home.load_config().unwrap();
    let mut s = State::new(core.home.clone(), Some(config));
    s.fixture = true;
    let mut rt = runtime_sized(s, (1080.0, 1400.0));
    rt.update(|s, _| crate::nav::go(s, crate::nav::Screen::Overview));
    run_jobs(&mut rt, core);
    rt
}

/// Every string the window shows or names, joined.
pub fn everything(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .flat_map(|e| [e.text, e.label])
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
        // The pads beside a chip are drawn, not read: `crate::cover::PAD`.
        .replace(crate::cover::PAD, "")
}

/// Fails if any fake value is anywhere in the window.
pub fn assert_no_value(rt: &Runtime<State>, on: &str) {
    let all = everything(rt);
    for v in values() {
        assert!(!all.contains(&v), "{on}: a value is on screen");
    }
}
