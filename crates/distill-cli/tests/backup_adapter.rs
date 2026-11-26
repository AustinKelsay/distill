//! A backup through an adapter, through the real binary (issue #177): `backup run`, `backup verify
//! --full`, `backup status` and `restore --kind` reach `distill-backup-<kind>` on PATH. The
//! reference adapter keeps objects in a folder.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

fn distill(user: &Path, home: &Path, args: &[&str]) -> Output {
    let bin = user.join("bin");
    Command::new(env!("CARGO_BIN_EXE_distill"))
        .args(["--home", home.to_str().unwrap()])
        .args(args)
        .env("HOME", user)
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("OLLAMA_HOST", "127.0.0.1:1")
        .env_remove("OLLAMA_MODELS")
        .env_remove("DISTILL_HOME")
        .output()
        .unwrap()
}

fn ok(user: &Path, home: &Path, args: &[&str]) -> String {
    let out = distill(user, home, args);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "distill {args:?} failed: {text}");
    text
}

#[test]
fn a_home_backs_up_through_an_adapter_and_comes_back_from_it() {
    let tmp = tempfile::tempdir().unwrap();
    let user = tmp.path().join("user");
    fs::create_dir_all(user.join("bin")).unwrap();
    let adapter = user.join("bin/distill-backup-reference");
    fs::write(
        &adapter,
        include_str!("../../distill-core/tests/fixtures/distill-backup-reference"),
    )
    .unwrap();
    fs::set_permissions(&adapter, fs::Permissions::from_mode(0o755)).unwrap();
    let claude = tmp.path().join("claude/proj");
    fs::create_dir_all(&claude).unwrap();
    fs::write(
        claude.join("s1.jsonl"),
        r#"{"type":"user","cwd":"/work/app","timestamp":"2026-11-10T10:00:00Z","message":{"role":"user","content":"fix the failing test"}}
"#,
    )
    .unwrap();
    let offsite = tmp.path().join("offsite");
    fs::create_dir(&offsite).unwrap();

    let home = tmp.path().join("home");
    ok(&user, &home, &["init"]);
    fs::write(
        home.join("config.toml"),
        format!(
            "[[harnesses]]\nkind = \"claude-code\"\nenabled = true\nroots = [\"{}\"]\n\n\
             [[backup]]\nname = \"offsite\"\nkind = \"reference\"\ndir = \"{}\"\n",
            claude.parent().unwrap().display(),
            offsite.display()
        ),
    )
    .unwrap();
    ok(&user, &home, &["sweep"]);
    let key = ok(&user, &home, &["--json", "backup", "init"]);
    let key: serde_json::Value = serde_json::from_str(&key).unwrap();
    let key_file = tmp.path().join("key.txt");
    fs::write(&key_file, key["backup_key"].as_str().unwrap()).unwrap();

    let ran = ok(&user, &home, &["backup", "run"]);
    assert!(ran.contains("offsite"), "{ran}");
    assert!(offsite.join("objects").read_dir().unwrap().count() > 0);
    let checked = ok(&user, &home, &["backup", "verify", "--full"]);
    assert!(checked.contains("fetched and decrypted"), "{checked}");
    let status = ok(&user, &home, &["--json", "backup", "status"]);
    assert!(status.contains("\"offsite\""), "{status}");

    let restored = tmp.path().join("restored");
    let dir = format!("dir={}", offsite.display());
    let back = ok(
        &user,
        &restored,
        &[
            "restore",
            "--kind",
            "reference",
            "--set",
            &dir,
            "--key-file",
            key_file.to_str().unwrap(),
        ],
    );
    assert!(back.contains("Restored into"), "{back}");
    assert!(restored.join("raw").is_dir());

    // An adapter that isn't on PATH is found where DISTILL is installed, as the window names it
    // (`DISTILL_CLI`): the timer and the window may run with a PATH that leaves that folder out.
    let beside = Command::new(env!("CARGO_BIN_EXE_distill"))
        .args(["--home", tmp.path().join("beside").to_str().unwrap()])
        .args([
            "restore",
            "--kind",
            "reference",
            "--set",
            &dir,
            "--key-file",
        ])
        .arg(&key_file)
        .env("HOME", &user)
        .env("PATH", "/usr/bin:/bin")
        .env("DISTILL_CLI", user.join("bin/distill"))
        .env_remove("DISTILL_HOME")
        .output()
        .unwrap();
    assert!(
        beside.status.success(),
        "{}",
        String::from_utf8_lossy(&beside.stderr)
    );

    // An adapter that isn't installed says so.
    let missing = distill(
        &user,
        &tmp.path().join("other"),
        &[
            "restore",
            "--kind",
            "nowhere",
            "--key-file",
            key_file.to_str().unwrap(),
        ],
    );
    let said = String::from_utf8_lossy(&missing.stderr);
    assert!(
        !missing.status.success() && said.contains("no `distill-backup-nowhere` on PATH"),
        "{said}"
    );
}
