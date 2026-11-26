//! A spoke (ADR 0010, issue #139), through the real binary: `init --spoke` names it, its tick only
//! captures, and nothing reads there.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn distill(user: &Path, home: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_distill"))
        .env("HOME", user)
        .env_remove("DISTILL_HOME")
        .env("OLLAMA_HOST", "127.0.0.1:1")
        .args(["--home", home.to_str().unwrap(), "--json"])
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn json(user: &Path, home: &Path, args: &[&str]) -> Value {
    let (ok, out) = distill(user, home, args);
    assert!(ok, "distill {args:?} failed: {out}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("{args:?}: {e}\n{out}"))
}

#[test]
fn a_spoke_captures_and_serves_no_reader() {
    let tmp = tempfile::tempdir().unwrap();
    let user = tmp.path().join("user");
    // Under Claude Code's own folder, so `init` finds it.
    let claude = user.join(".claude/projects/proj");
    fs::create_dir_all(&claude).unwrap();
    fs::write(
        claude.join("s1.jsonl"),
        r#"{"type":"user","cwd":"/work/app","sessionId":"s1","timestamp":"2026-11-10T10:00:00Z","message":{"role":"user","content":"fix the failing test"}}
"#,
    )
    .unwrap();
    let home = tmp.path().join("home");

    let (ok, out) = distill(&user, &home, &["init", "--spoke", "Build Box"]);
    assert!(!ok && !home.join("config.toml").exists(), "{out}");
    let c = json(&user, &home, &["init", "--spoke", "nas"]);
    assert_eq!(c["spoke"]["machine"], "nas", "{c}");
    let written = fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(written.contains("[spoke]\nmachine = \"nas\""), "{written}");
    // A spoke keeps its name and stays a spoke.
    for args in [
        &["init", "--force"][..],
        &["init", "--force", "--spoke", "other"],
    ] {
        let (ok, out) = distill(&user, &home, args);
        assert!(
            !ok && out.contains("a machine keeps its name"),
            "{args:?}: {out}"
        );
    }
    json(&user, &home, &["init", "--force", "--spoke", "nas"]);

    let tick = json(&user, &home, &["tick"]);
    let steps: Vec<_> = tick["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(steps, ["sweep"], "{tick}");
    assert_eq!(tick["steps"][0]["report"]["new"], 1, "{tick}");
    assert!(home.join("raw/manifest.jsonl").exists());
    assert!(!home.join("sessions/claude-code").exists());
    let (ok, out) = distill(&user, &home, &["parse"]);
    assert!(!ok && out.contains("the hub parses what it pulls"), "{out}");

    // No reader reads here, the owner included; each refusal is in the read log.
    for args in [
        &["search", "failing"][..],
        &["read", "claude-code/s1"],
        &["tags"],
    ] {
        let (ok, out) = distill(&user, &home, args);
        assert!(
            !ok && out.contains("this home is the spoke \"nas\""),
            "{args:?}: {out}"
        );
    }
    let log = fs::read_to_string(home.join("log/reads.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 3, "{log}");

    // A spoke's own exposure check: owner-only, not synced, no listener. Without `lsof` the
    // listener rule can't be checked, and says so.
    let exposure = json(&user, &home, &["exposure"]);
    for rule in exposure["rules"].as_array().unwrap() {
        let unchecked = rule["rule"] == "no_listener" && rule["outcome"] == "unchecked";
        assert!(rule["outcome"] == "pass" || unchecked, "{exposure}");
    }

    // A config that can't be read serves no reader either: it might name a spoke.
    let bad = written.replace("machine = \"nas\"", "machine = \"Build Box\"");
    fs::write(home.join("config.toml"), bad).unwrap();
    let (ok, out) = distill(&user, &home, &["search", "failing"]);
    assert!(!ok && out.contains("the config can't be read"), "{out}");
    assert!(
        !out.contains("Build Box"),
        "a reader gets no line of the config: {out}"
    );
    // Nor is it parsed, scanned, or written over, since it may name a spoke.
    for args in [&["parse"][..], &["scan"], &["init", "--force"]] {
        let (ok, out) = distill(&user, &home, args);
        assert!(
            !ok && out.contains("can't tell whether this home is a spoke"),
            "{args:?}: {out}"
        );
    }
}
