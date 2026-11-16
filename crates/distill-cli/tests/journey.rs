//! The kept loop through the real binary, against synthetic harness files in a temp dir: init,
//! harnesses, sweep with append detection and exclusions, verify, parse, scan, tick (which also
//! builds the word index), status and runs.
//! Every command runs with `HOME` set to the temp dir, and none may print session text.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// Run `distill` against `home`, with `HOME` set to `user_home` so nothing reads the real one.
fn distill(user_home: &Path, home: &Path, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_distill"))
        .env("HOME", user_home)
        .env_remove("DISTILL_HOME")
        .args(["--home", home.to_str().unwrap()])
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Words that appear only in the synthetic sessions' text.
const SESSION_WORDS: [&str; 4] = [
    "fix the failing test",
    "All tests pass",
    "list files",
    "a.txt",
];

fn no_session_text(args: &[&str], out: &str) {
    for w in SESSION_WORDS {
        assert!(
            !out.contains(w),
            "distill {args:?} printed session text {w:?}:\n{out}"
        );
    }
}

fn json(user_home: &Path, home: &Path, args: &[&str]) -> Value {
    let mut a = vec!["--json"];
    a.extend_from_slice(args);
    let (ok, out, err) = distill(user_home, home, &a);
    assert!(ok, "distill {args:?} failed: {err}");
    no_session_text(args, &out);
    no_session_text(args, &err);
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("bad json from {args:?}: {e}\n{out}"))
}

fn human(user_home: &Path, home: &Path, args: &[&str]) -> String {
    let (ok, out, err) = distill(user_home, home, args);
    assert!(ok, "distill {args:?} failed: {err}");
    no_session_text(args, &out);
    no_session_text(args, &err);
    out
}

fn claude_session(cwd: &str) -> String {
    format!(
        r#"{{"type":"user","cwd":"{cwd}","version":"2.1.0","timestamp":"2026-11-10T10:00:00Z","message":{{"role":"user","content":"fix the failing test"}}}}
{{"type":"assistant","timestamp":"2026-11-10T10:00:05Z","message":{{"role":"assistant","model":"claude-opus-5-5","content":[{{"type":"tool_use","id":"t1","name":"Bash","input":{{"command":"cargo test"}}}}]}}}}
{{"type":"user","timestamp":"2026-11-10T10:00:09Z","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":"ok"}}]}}}}
"#
    )
}

fn touch_later(path: &Path, secs: u64) {
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(secs);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(later)
        .unwrap();
}

#[test]
fn the_kept_loop_through_the_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let user = root.join("user");
    fs::create_dir_all(&user).unwrap();
    let home = root.join("home");
    let claude = root.join("claude");
    let codex = root.join("codex");
    fs::create_dir_all(claude.join("proj")).unwrap();
    fs::create_dir_all(codex.join("2026/11/10")).unwrap();

    let session_a = claude.join("proj/aaaa.jsonl");
    fs::write(&session_a, claude_session("/work/app")).unwrap();
    // Excluded by privacy settings: never archived.
    fs::write(
        claude.join("proj/bbbb.jsonl"),
        claude_session("/work/secret-client/repo"),
    )
    .unwrap();
    // Run by the previous build's steward (cwd inside the home): archived and marked.
    fs::write(
        claude.join("proj/cccc.jsonl"),
        claude_session(&format!("{}/steward", home.display())),
    )
    .unwrap();
    fs::write(
        codex.join("2026/11/10/rollout-x.jsonl"),
        r#"{"timestamp":"2026-11-10T11:00:00Z","type":"session_meta","payload":{"id":"cx1","cwd":"/work/app","cli_version":"0.169.0"}}
{"timestamp":"2026-11-10T11:00:01Z","type":"turn_context","payload":{"model":"gpt-6"}}
{"timestamp":"2026-11-10T11:00:02Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"list files"}]}}
{"timestamp":"2026-11-10T11:00:03Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"cmd\":[\"ls\"]}","call_id":"c1"}}
{"timestamp":"2026-11-10T11:00:04Z","type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"a.txt"}}
"#,
    )
    .unwrap();

    // Init writes the spec's names; nothing of the temp user's is found.
    let c = json(&user, &home, &["init"]);
    assert!(
        c["harnesses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|h| h["enabled"] == false),
        "{c}"
    );
    let (ok, _, err) = distill(&user, &home, &["init"]);
    assert!(!ok && err.contains("already exists"), "{err}");
    let written = fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(written.contains("[[harnesses]]"), "{written}");

    let config = format!(
        r#"[[harnesses]]
kind = "claude-code"
enabled = true
roots = ["{claude}"]

[[harnesses]]
kind = "codex"
enabled = true
roots = ["{codex}"]

[privacy]
exclude_paths = ["/work/secret-client"]

[schedule]
sweep_every_minutes = 60
"#,
        claude = claude.display(),
        codex = codex.display(),
    );
    fs::write(home.join("config.toml"), config).unwrap();

    let h = json(&user, &home, &["harnesses"]);
    let claude_row = h
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == "claude-code")
        .unwrap();
    assert_eq!(claude_row["found"], true);
    human(&user, &home, &["harnesses"]);

    // Sweep: three captured, one excluded. A second sweep captures nothing.
    let s = json(&user, &home, &["sweep"]);
    assert_eq!(s["new"], 3, "{s}");
    assert_eq!(s["excluded"], 1, "{s}");
    let s = json(&user, &home, &["sweep", "--harness", "claude-code"]);
    assert_eq!(
        s["new"].as_u64().unwrap() + s["appended"].as_u64().unwrap(),
        0,
        "{s}"
    );
    let (ok, _, err) = distill(&user, &home, &["sweep", "--harness", "nope"]);
    assert!(!ok && err.contains("unknown harness"), "{err}");

    // A growing session is an append: the old capture is replaced, not duplicated.
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(&session_a)
        .unwrap();
    writeln!(f, r#"{{"type":"assistant","timestamp":"2026-11-10T10:01:00Z","message":{{"role":"assistant","model":"claude-opus-5-5","content":[{{"type":"text","text":"All tests pass."}}]}}}}"#).unwrap();
    drop(f);
    touch_later(&session_a, 5);
    let s = json(&user, &home, &["sweep"]);
    assert_eq!(s["appended"], 1, "{s}");
    let manifest = fs::read_to_string(home.join("raw/manifest.jsonl")).unwrap();
    assert!(manifest.contains("\"kind\":\"append\""));
    assert!(
        manifest.contains("\"source\":\"claude-code\"") && manifest.contains("\"fingerprint\"")
    );

    let v = json(&user, &home, &["verify"]);
    assert_eq!(v["checked"], 3, "{v}");
    assert!(
        v["missing"].as_array().unwrap().is_empty() && v["corrupt"].as_array().unwrap().is_empty()
    );
    human(&user, &home, &["verify"]);

    // Parse: events with tool calls, the old steward's session marked, facts without text.
    let n = json(&user, &home, &["parse"]);
    assert_eq!(n["sessions"], 3, "{n}");
    assert_eq!(n["failed"], 0, "{n}");
    let events = fs::read_to_string(home.join("sessions/claude-code/aaaa/events.jsonl")).unwrap();
    assert!(events.contains("\"kind\":\"tool_call\"") && events.contains("\"tool\":\"Bash\""));
    assert!(
        events.contains("All tests pass."),
        "the appended turn is parsed"
    );
    let marked: Value = serde_json::from_str(
        &fs::read_to_string(home.join("sessions/claude-code/cccc/meta.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(marked["steward"], true);
    let list = fs::read_to_string(home.join("sessions/index.jsonl")).unwrap();
    assert_eq!(list.lines().count(), 3);
    no_session_text(&["sessions/index.jsonl"], &list);
    let n = json(&user, &home, &["parse"]);
    assert_eq!(n["normalized"], 0, "parse is idempotent: {n}");
    human(&user, &home, &["parse", "--force"]);

    // Scan: every session's findings made, counts only, and a second scan finds them current.
    let sc = json(&user, &home, &["scan"]);
    assert_eq!(
        (sc["sessions"].as_u64(), sc["scanned"].as_u64()),
        (Some(3), Some(3)),
        "{sc}"
    );
    assert!(
        home.join("sessions/claude-code/aaaa/findings.json")
            .exists()
    );
    let sc = json(&user, &home, &["scan"]);
    assert_eq!(
        (sc["scanned"].as_u64(), sc["current"].as_u64()),
        (Some(0), Some(3)),
        "{sc}"
    );
    human(&user, &home, &["scan"]);

    // A tick is sweep, parse, scan then the word index, and one run-log entry.
    let before = json(&user, &home, &["runs", "--limit", "100"])
        .as_array()
        .unwrap()
        .len();
    let t = json(&user, &home, &["tick"]);
    let names: Vec<_> = t["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].clone())
        .collect();
    assert_eq!(names, ["sweep", "parse", "scan", "index"], "{t}");
    assert_eq!(t["steps"][3]["report"]["indexed"], 3, "{t}");
    let runs = json(&user, &home, &["runs", "--limit", "100"]);
    assert_eq!(runs.as_array().unwrap().len(), before + 1);
    assert_eq!(runs[0]["step"], "tick");
    for step in ["sweep", "verify", "parse", "scan", "tick"] {
        assert!(
            runs.as_array().unwrap().iter().any(|r| r["step"] == step),
            "no {step} entry"
        );
    }
    human(&user, &home, &["tick"]);
    human(&user, &home, &["runs"]);

    // Status: sessions and capture health per harness, the last sweep and scan progress.
    let st = json(&user, &home, &["status"]);
    assert_eq!(st["sessions"], 3, "{st}");
    assert_eq!(st["scan"]["current"], 3, "{st}");
    assert_eq!(st["scan"]["pending"], 0, "{st}");
    assert_eq!(st["index"]["current"], 3, "{st}");
    assert_eq!(st["index"]["pending"], 0, "{st}");
    assert!(st["last_sweep"].is_string(), "{st}");
    // Just swept, and no harness here deletes sessions sooner than the timer runs.
    assert_eq!(st["warnings"], serde_json::json!([]), "{st}");
    let of = |k: &str| {
        st["harnesses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["harness"] == k)
            .unwrap()
            .clone()
    };
    assert_eq!(of("claude-code")["sessions"], 2);
    assert_eq!(of("claude-code")["health"], "ok");
    assert_eq!(of("codex")["sessions"], 1);
    assert_eq!(of("pi")["health"], "disabled");
    let printed = human(&user, &home, &["status"]);
    assert!(
        printed.contains("claude-code")
            && printed.contains("last sweep")
            && printed.contains("scan: 3 of 3 sessions current")
            && printed.contains("word index: 3 of 3 sessions current"),
        "{printed}"
    );

    // Gone with the previous build.
    for gone in [
        "sessions",
        "search",
        "steward",
        "recipe",
        "export",
        "import-legacy",
        "normalize",
        "sources",
    ] {
        let (ok, _, _) = distill(&user, &home, &[gone]);
        assert!(!ok, "`distill {gone}` still exists");
    }
}

#[test]
fn an_existing_home_from_the_previous_build_keeps_working() {
    let tmp = tempfile::tempdir().unwrap();
    let user = tmp.path().join("user");
    let pi = tmp.path().join("pi");
    fs::create_dir_all(&user).unwrap();
    fs::create_dir_all(&pi).unwrap();
    let home = tmp.path().join("home");
    fs::write(
        pi.join("2026-11-01_p1.jsonl"),
        r#"{"type":"session","id":"p1","cwd":"/r","version":3}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"fix the failing test"}],"timestamp":1794436930000}}
"#,
    )
    .unwrap();
    // The previous build's config, and folders of features that are gone.
    fs::create_dir_all(home.join("recipes")).unwrap();
    fs::write(home.join("recipes/good-sessions.toml"), "x = 1\n").unwrap();
    fs::create_dir_all(home.join("exports/good-sessions/b1")).unwrap();
    fs::create_dir_all(home.join("steward")).unwrap();
    fs::write(
        home.join("config.toml"),
        format!(
            r#"[[sources]]
kind = "pi"
enabled = true
roots = ["{}"]

[privacy]
exclude_paths = []
redact_exports = true

[steward]
harness = "claude"
model = "some-model"
daily_budget_usd = 2.0
max_sessions_per_run = 25

[schedule]
sweep_every_minutes = 60
enrich_every_hours = 24
"#,
            pi.display()
        ),
    )
    .unwrap();

    let t = json(&user, &home, &["tick"]);
    assert!(
        t["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["ok"] == true),
        "{t}"
    );
    let st = json(&user, &home, &["status"]);
    assert_eq!(st["sessions"], 1, "{st}");
    assert!(home.join("recipes/good-sessions.toml").exists());
    assert!(home.join("exports/good-sessions/b1").exists());
    assert!(home.join("steward").exists());
    // The old config is left as it was written; only `init --force` rewrites it.
    assert!(
        fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .contains("[[sources]]")
    );
}

#[test]
fn schedule_print_is_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let user = tmp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    let home = tmp.path().join("home");
    let (ok, _, err) = distill(&user, &home, &["init"]);
    assert!(ok, "{err}");
    let (ok, out, err) = distill(&user, &home, &["schedule", "print"]);
    assert!(ok, "{err}");
    assert!(out.contains("tick"));
    if cfg!(target_os = "macos") {
        let plist = tmp.path().join("t.plist");
        fs::write(&plist, &out).unwrap();
        let lint = Command::new("plutil")
            .arg("-lint")
            .arg(&plist)
            .output()
            .unwrap();
        assert!(
            lint.status.success(),
            "{}",
            String::from_utf8_lossy(&lint.stdout)
        );
    }
}
