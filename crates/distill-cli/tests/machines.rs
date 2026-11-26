//! A spoke's raw archive on the hub (ADR 0010, issue #137), through the real binary: its captures
//! parse into sessions of their own, keyed by machine, beside the hub's, and `verify` checks it.

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

/// Copy a folder's files, keeping its layout.
fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

#[test]
fn a_spokes_captures_are_sessions_keyed_by_its_machine() {
    let tmp = tempfile::tempdir().unwrap();
    let user = tmp.path().join("user");
    let claude = tmp.path().join("claude/proj");
    fs::create_dir_all(&claude).unwrap();
    fs::write(
        claude.join("s1.jsonl"),
        r#"{"type":"user","cwd":"/work/app","sessionId":"s1","timestamp":"2026-11-10T10:00:00Z","message":{"role":"user","content":"fix the failing test"}}
"#,
    )
    .unwrap();
    let home = tmp.path().join("home");
    json(&user, &home, &["init"]);
    fs::write(
        home.join("config.toml"),
        format!(
            "[[harnesses]]\nkind = \"claude-code\"\nenabled = true\nroots = [\"{}\"]\n",
            claude.parent().unwrap().display()
        ),
    )
    .unwrap();
    json(&user, &home, &["sweep"]);

    // The same captures, as another machine's: same origin paths, same session id.
    copy(&home.join("raw"), &home.join("machines/nas/raw"));
    let parsed = json(&user, &home, &["parse"]);
    assert_eq!(parsed["sessions"], 2, "{parsed}");
    // The spoke's session answers to its key on every face, here the owner's tags.
    let tagged = json(&user, &home, &["tag", "nas:claude-code/s1", "--add", "server"]);
    assert!(tagged.to_string().contains("server"), "{tagged}");
    json(&user, &home, &["tag", "claude-code/s1", "--add", "laptop"]);

    let checked = json(&user, &home, &["verify"]);
    assert_eq!(checked["checked"], 2, "{checked}");
    // A spoke's blob that changed is the spoke's, by name.
    let blob = walk(&home.join("machines/nas/raw"))
        .into_iter()
        .find(|p| !p.ends_with("manifest.jsonl"))
        .unwrap();
    fs::write(&blob, "changed").unwrap();
    let (ok, out) = distill(&user, &home, &["verify"]);
    assert!(!ok && out.contains("nas: "), "{out}");
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut all = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            all.extend(walk(&p));
        } else {
            all.push(p);
        }
    }
    all
}
