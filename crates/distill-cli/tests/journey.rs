//! The whole loop through the real binary, against fixture sources in a temp dir: sweep,
//! append detection, verify, normalize, steward cards (stub harness), exclusion, steward
//! tagging, recipe build with redaction, approval gate, publish, and the budget cap.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn distill(home: &Path, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_distill"))
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

fn json(home: &Path, args: &[&str]) -> Value {
    let mut a = vec!["--json"];
    a.extend_from_slice(args);
    let (ok, out, err) = distill(home, &a);
    assert!(ok, "distill {args:?} failed: {err}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("bad json from {args:?}: {e}\n{out}"))
}

fn claude_session(cwd: &str, secret: &str) -> String {
    format!(
        r#"{{"type":"user","cwd":"{cwd}","version":"2.1.0","timestamp":"2026-11-10T10:00:00Z","message":{{"role":"user","content":"fix the failing test, my key is {secret}"}}}}
{{"type":"assistant","timestamp":"2026-11-10T10:00:05Z","message":{{"role":"assistant","model":"claude-opus-5-5","content":[{{"type":"tool_use","id":"t1","name":"Bash","input":{{"command":"cargo test","token":"synthetic-token-12345"}}}}]}}}}
{{"type":"user","timestamp":"2026-11-10T10:00:09Z","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":"ok"}}]}}}}
"#
    )
}

#[test]
fn full_loop_through_the_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let claude = root.join("claude");
    let codex = root.join("codex");
    fs::create_dir_all(claude.join("proj")).unwrap();
    fs::create_dir_all(codex.join("2026/11/10")).unwrap();

    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let session_a = claude.join("proj/aaaa.jsonl");
    fs::write(&session_a, claude_session("/work/app", secret)).unwrap();
    // Excluded by privacy settings: never archived.
    fs::write(
        claude.join("proj/bbbb.jsonl"),
        claude_session("/work/secret-client/repo", "x"),
    )
    .unwrap();
    // Run by the steward itself (cwd inside the home): archived, tagged, kept out of datasets.
    fs::write(
        claude.join("proj/cccc.jsonl"),
        claude_session(&format!("{}/steward", home.display()), "x"),
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

    let stub = root.join("steward.sh");
    fs::write(
        &stub,
        "#!/bin/sh\ncat > /dev/null\necho 'Here is the card: {\"task\":\"Fix the failing test\",\"outcome\":\"success\",\"quality\":5,\"corrections\":[],\"tags\":[\"rust\"],\"summary\":\"Ran the tests.\"}'\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let (ok, _, err) = distill(&home, &["init"]);
    assert!(ok, "{err}");
    let config = format!(
        r#"[[sources]]
kind = "claude-code"
enabled = true
roots = ["{claude}"]

[[sources]]
kind = "codex"
enabled = true
roots = ["{codex}"]

[privacy]
exclude_paths = ["/work/secret-client"]
redact_exports = true

[steward]
harness = "command"
daily_budget_usd = 1.0
max_sessions_per_run = 10
command = ["{stub}"]

[schedule]
sweep_every_minutes = 60
enrich_every_hours = 24
"#,
        claude = claude.display(),
        codex = codex.display(),
        stub = stub.display()
    );
    fs::write(home.join("config.toml"), config).unwrap();

    // Sweep: three captured, one excluded. A second sweep captures nothing.
    let s = json(&home, &["sweep"]);
    assert_eq!(s["new"], 3, "{s}");
    assert_eq!(s["excluded"], 1, "{s}");
    let s = json(&home, &["sweep"]);
    assert_eq!(
        s["new"].as_u64().unwrap() + s["appended"].as_u64().unwrap(),
        0,
        "{s}"
    );

    // A growing session is an append: the old blob is replaced, not duplicated.
    let blobs_before = fs::read_dir(home.join("raw")).unwrap().count();
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(&session_a)
        .unwrap();
    writeln!(f, r#"{{"type":"assistant","timestamp":"2026-11-10T10:01:00Z","message":{{"role":"assistant","model":"claude-opus-5-5","content":[{{"type":"text","text":"All tests pass."}}]}}}}"#).unwrap();
    drop(f);
    // Make sure the mtime differs even on coarse filesystems.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    fs::File::options()
        .write(true)
        .open(&session_a)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let s = json(&home, &["sweep"]);
    assert_eq!(s["appended"], 1, "{s}");
    let manifest = fs::read_to_string(home.join("raw/manifest.jsonl")).unwrap();
    assert!(manifest.contains("\"kind\":\"append\""));
    let _ = blobs_before;

    let v = json(&home, &["verify"]);
    assert_eq!(v["checked"], 3, "{v}");
    assert!(
        v["missing"].as_array().unwrap().is_empty() && v["corrupt"].as_array().unwrap().is_empty()
    );

    // Normalize: full trajectories, steward session tagged.
    let n = json(&home, &["normalize"]);
    assert_eq!(n["sessions"], 3, "{n}");
    assert_eq!(n["failed"], 0, "{n}");
    let events = fs::read_to_string(home.join("sessions/claude-code/aaaa/events.jsonl")).unwrap();
    assert!(events.contains("\"kind\":\"tool_call\"") && events.contains("\"tool\":\"Bash\""));
    assert!(
        events.contains("All tests pass."),
        "appended turn is normalized"
    );
    let steward_meta: Value = serde_json::from_str(
        &fs::read_to_string(home.join("sessions/claude-code/cccc/meta.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(steward_meta["steward"], true);
    // Normalize is idempotent.
    let n = json(&home, &["normalize"]);
    assert_eq!(n["normalized"], 0, "{n}");

    // Steward writes cards for the two user sessions, not its own.
    let r = json(&home, &["steward", "run"]);
    assert_eq!(r["carded"], 2, "{r}");
    assert_eq!(r["remaining"], 0, "{r}");
    let card: Value = serde_json::from_str(
        &fs::read_to_string(home.join("sessions/codex/cx1/card.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(card["quality"], 5);
    assert_eq!(card["produced_by"]["harness"], "command");
    assert!(!home.join("sessions/claude-code/cccc/card.json").exists());

    // Dataset: built from the default recipe, secret redacted, steward session left out.
    let b = json(&home, &["export", "build", "good-sessions"]);
    let build = b[0]["build"].as_str().unwrap().to_string();
    assert_eq!(b[0]["records"], 2, "{b}");
    assert!(b[0]["redactions"].as_u64().unwrap() >= 1, "{b}");
    assert_eq!(b[0]["approved"], false);
    let data_path = home.join(format!("exports/good-sessions/{build}/data.jsonl"));
    let data = fs::read(&data_path).unwrap();
    let text = String::from_utf8_lossy(&data);
    assert!(!text.contains(secret), "secret leaked into the dataset");
    assert!(text.contains("[REDACTED:github_token]"));
    assert!(text.contains("\"tool_calls\""));
    assert!(
        !text.contains("synthetic-token"),
        "credential in tool arguments leaked"
    );
    for line in text.lines() {
        let rec: Value = serde_json::from_str(line).unwrap();
        for m in rec["messages"].as_array().unwrap() {
            for call in m["tool_calls"].as_array().into_iter().flatten() {
                let args = call["function"]["arguments"].as_str().unwrap();
                let parsed: Value = serde_json::from_str(args).expect("arguments stay valid JSON");
                if let Some(t) = parsed.get("token") {
                    assert_eq!(t, "[REDACTED:credential_field]");
                }
            }
        }
    }
    assert!(!text.contains("cccc"));

    // Rebuilding from the same inputs gives the same id and the same bytes.
    fs::remove_dir_all(data_path.parent().unwrap()).unwrap();
    let b2 = json(&home, &["export", "build", "good-sessions"]);
    assert_eq!(b2[0]["build"], build.as_str());
    assert_eq!(fs::read(&data_path).unwrap(), data);

    // A build from changed inputs supersedes the old one: only the newest asks for approval.
    let old_card = home.join("sessions/codex/cx1/card.json");
    let saved = fs::read(&old_card).unwrap();
    fs::remove_file(&old_card).unwrap();
    let b3 = json(&home, &["export", "build", "good-sessions"]);
    assert_ne!(b3[0]["build"], build.as_str());
    let st = json(&home, &["status"]);
    let pending: Vec<_> = st["needs_you"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["kind"] == "approve_export")
        .collect();
    assert_eq!(pending.len(), 1, "{st}");
    assert_eq!(pending[0]["build"], b3[0]["build"]);
    assert_eq!(st["totals"]["datasets"], 1);
    fs::write(&old_card, saved).unwrap();
    let b4 = json(&home, &["export", "build", "good-sessions"]);
    assert_eq!(
        b4[0]["build"],
        build.as_str(),
        "same inputs, same build again"
    );
    // Make the original build the newest again so the steps below act on it.
    let manifest = home.join(format!("exports/good-sessions/{build}/manifest.json"));
    let mut m: Value = serde_json::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
    m["built_at"] = Value::String("2999-01-01T00:00:00Z".into());
    fs::write(&manifest, serde_json::to_vec(&m).unwrap()).unwrap();

    // The dashboard asks for approval; publish refuses until it is given.
    let st = json(&home, &["status"]);
    let needs = st["needs_you"].as_array().unwrap();
    assert!(needs.iter().any(|n| n["kind"] == "approve_export"), "{st}");
    assert_eq!(st["totals"]["sessions"], 3);
    assert_eq!(st["totals"]["enriched"], 1.0);
    let out_dir = root.join("out");
    let (ok, _, err) = distill(
        &home,
        &[
            "export",
            "publish",
            "good-sessions",
            &build,
            "--to",
            out_dir.to_str().unwrap(),
        ],
    );
    assert!(!ok && err.contains("not approved"), "{err}");
    let (ok, _, err) = distill(&home, &["export", "approve", "good-sessions", &build]);
    assert!(ok, "{err}");
    let (ok, out, err) = distill(
        &home,
        &[
            "export",
            "publish",
            "good-sessions",
            &build,
            "--to",
            out_dir.to_str().unwrap(),
        ],
    );
    assert!(ok, "{err}");
    assert!(Path::new(out.trim()).exists());
    let st = json(&home, &["status"]);
    assert!(
        !st["needs_you"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["kind"] == "approve_export")
    );

    // Search and session listing.
    let hits = json(&home, &["search", "failing test"]);
    assert!(
        hits.as_array()
            .unwrap()
            .iter()
            .any(|h| h["session_id"] == "aaaa"),
        "{hits}"
    );
    let list = json(&home, &["sessions"]);
    assert_eq!(
        list.as_array().unwrap().len(),
        2,
        "steward session hidden from listing"
    );

    // Run log covers every step.
    let runs = json(&home, &["runs", "--limit", "50"]);
    for step in ["sweep", "verify", "normalize", "steward", "export"] {
        assert!(
            runs.as_array().unwrap().iter().any(|r| r["step"] == step),
            "no {step} entry"
        );
    }

    // A card goes stale when its session grows: the steward picks it up again.
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(&session_a)
        .unwrap();
    writeln!(f, r#"{{"type":"user","timestamp":"2026-11-10T10:05:00Z","message":{{"role":"user","content":"no, that broke the build"}}}}"#).unwrap();
    drop(f);
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
    fs::File::options()
        .write(true)
        .open(&session_a)
        .unwrap()
        .set_modified(later)
        .unwrap();
    json(&home, &["sweep"]);
    json(&home, &["normalize"]);
    let r = json(&home, &["steward", "run", "--dry-run"]);
    assert_eq!(r["remaining"], 1, "{r}");

    // A paid call whose answer is not a valid card still counts toward the budget.
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("claude");
    fs::write(&fake, "#!/bin/sh\ncat > /dev/null\necho '{\"result\":\"not a card\",\"total_cost_usd\":0.4,\"is_error\":false}'\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let cfg = fs::read_to_string(home.join("config.toml")).unwrap();
    fs::write(
        home.join("config.toml"),
        cfg.replace("harness = \"command\"", "harness = \"claude\""),
    )
    .unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new(env!("CARGO_BIN_EXE_distill"))
        .args(["--home", home.to_str().unwrap(), "--json", "steward", "run"])
        .env("PATH", &path)
        .output()
        .unwrap();
    let r: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(r["failed"], 1, "{r}");
    assert!((r["cost_usd"].as_f64().unwrap() - 0.4).abs() < 1e-9, "{r}");
    let st = json(&home, &["status"]);
    assert!(
        (st["totals"]["spent_today_usd"].as_f64().unwrap() - 0.4).abs() < 1e-9,
        "{st}"
    );
    fs::write(home.join("config.toml"), cfg).unwrap();

    // Spend cap: a Claude steward with no budget left stops before calling anything.
    let cfg = fs::read_to_string(home.join("config.toml")).unwrap();
    fs::write(
        home.join("config.toml"),
        cfg.replace("harness = \"command\"", "harness = \"claude\"")
            .replace("daily_budget_usd = 1.0", "daily_budget_usd = 0.0"),
    )
    .unwrap();
    fs::remove_file(home.join("sessions/codex/cx1/card.json")).unwrap();
    let r = json(&home, &["steward", "run"]);
    assert_eq!(r["stopped_for_budget"], true, "{r}");
    assert_eq!(r["carded"], 0);
}

#[test]
fn schedule_print_is_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let (ok, _, err) = distill(&home, &["init"]);
    assert!(ok, "{err}");
    let (ok, out, err) = distill(&home, &["schedule", "print"]);
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
