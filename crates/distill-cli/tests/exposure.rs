//! The exposure check through the real binary (issue #90): every file and folder a full run
//! creates is owner-only even with a umask that would allow anything, `distill exposure` reports
//! each rule, and `--fix` tightens an existing home.
//!
//! `HOME` is a temp dir, so the synced folders and the old app's folder are fake paths in it.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

struct Machine {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    user: PathBuf,
    home: PathBuf,
}

impl Machine {
    fn new(home_in_user: &str) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let user = root.join("user");
        fs::create_dir_all(&user).unwrap();
        Machine {
            home: user.join(home_in_user),
            root,
            user,
            _tmp: tmp,
        }
    }

    /// Run `distill` with umask 000, so anything Distill doesn't make owner-only is open to all.
    fn run(&self, args: &[&str]) -> (bool, String) {
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let out = Command::new("/bin/sh")
            .args(["-c", "umask 000; exec \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_distill"))
            .args(["--home", self.home.to_str().unwrap()])
            .args(args)
            .env("HOME", &self.user)
            .env("OLLAMA_HOST", closed.to_string())
            .env_remove("OLLAMA_MODELS")
            .env_remove("DISTILL_HOME")
            .env_remove("DISTILL_PROVIDER")
            .env_remove("DISTILL_MODEL")
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

    /// Set up the home with one synthetic Claude Code session to archive.
    fn with_a_session(&self) {
        let claude = self.root.join("claude");
        fs::create_dir_all(claude.join("proj")).unwrap();
        fs::write(
            claude.join("proj/s1.jsonl"),
            r#"{"type":"user","cwd":"/work/app","timestamp":"2026-11-10T10:00:00Z","message":{"role":"user","content":"fix the failing test"}}
{"type":"assistant","timestamp":"2026-11-10T10:00:05Z","message":{"role":"assistant","model":"m","content":"done"}}
"#,
        )
        .unwrap();
        self.ok(&["init"]);
        fs::write(
            self.home.join("config.toml"),
            format!(
                "[[harnesses]]\nkind = \"claude-code\"\nenabled = true\nroots = [\"{}\"]\n",
                claude.display()
            ),
        )
        .unwrap();
        // The test wrote the config with its own umask; set it as Distill would.
        chmod(&self.home.join("config.toml"), 0o600);
    }

    fn ok(&self, args: &[&str]) -> String {
        let (ok, out) = self.run(args);
        assert!(ok, "distill {args:?} failed: {out}");
        out
    }

    fn json(&self, args: &[&str]) -> (bool, Value) {
        let mut a = vec!["--json"];
        a.extend_from_slice(args);
        let (ok, out) = self.run(&a);
        let start = out.find('{').unwrap_or_else(|| panic!("no json: {out}"));
        let end = out.rfind('}').unwrap();
        (ok, serde_json::from_str(&out[start..=end]).unwrap())
    }
}

fn mode(p: &Path) -> u32 {
    fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777
}

fn chmod(p: &Path, m: u32) {
    fs::set_permissions(p, fs::Permissions::from_mode(m)).unwrap();
}

/// Every path under `dir`, `dir` included, that the group or others can use.
fn loose(dir: &Path) -> Vec<String> {
    walk(dir)
        .into_iter()
        .filter(|p| mode(p) & 0o077 != 0)
        .map(|p| format!("{} {:o}", p.display(), mode(&p)))
        .collect()
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![dir.to_path_buf()];
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if e.file_type().unwrap().is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

fn outcomes(v: &Value) -> Vec<(String, String)> {
    v["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["rule"].as_str().unwrap().to_string(),
                r["outcome"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn outcome(v: &Value, rule: &str) -> String {
    outcomes(v)
        .into_iter()
        .find(|(r, _)| r == rule)
        .unwrap_or_else(|| panic!("no rule {rule}: {v}"))
        .1
}

#[test]
fn everything_a_full_run_creates_is_owner_only() {
    // The home's own parent does not exist yet: folders made on the way to it count too.
    let m = Machine::new("nest/distill");
    m.with_a_session();
    for args in [
        &["sweep"][..],
        &["verify"],
        &["parse"],
        &["scan"],
        &["tick"],
        &["read", "s1", "--provider", "ollama"],
        &["reads"],
        &["status"],
        &["runs"],
        &["providers"],
        &["providers", "add", "example-cloud"],
    ] {
        m.ok(args);
    }
    assert!(m.home.join("keys/fingerprint.key").exists());
    assert!(m.home.join("log/reads.jsonl").exists());
    assert!(
        m.home
            .join("sessions/claude-code/s1/findings.json")
            .exists()
    );

    let open = loose(&m.user.join("nest"));
    assert!(open.is_empty(), "not owner-only:\n{}", open.join("\n"));

    let (ok, v) = m.json(&["exposure"]);
    assert!(ok, "{v}");
    for (rule, o) in outcomes(&v) {
        assert!(o == "pass" || o == "unchecked", "{rule}: {o}\n{v}");
    }
    assert_eq!(outcome(&v, "owner_only"), "pass");
    assert_eq!(outcome(&v, "key_owner_only"), "pass");
    assert_eq!(outcome(&v, "not_synced"), "pass");
    let (_, s) = m.json(&["status"]);
    assert_eq!(s["exposure"]["rules"], v["rules"]);
    assert!(v["backups"].as_str().unwrap().contains("Backups"));
}

#[test]
fn the_check_names_what_fails_and_fix_tightens_an_existing_home() {
    let m = Machine::new("distill");
    m.with_a_session();
    m.ok(&["tick"]);
    // A home from the previous build, left open.
    for (p, md) in [
        (m.home.clone(), 0o755),
        (m.home.join("config.toml"), 0o644),
        (m.home.join("keys/fingerprint.key"), 0o644),
    ] {
        chmod(&p, md);
    }
    fs::create_dir_all(m.home.join("exports/recipe/b1")).unwrap();
    fs::write(m.home.join("exports/recipe/b1/data.jsonl"), "{}\n").unwrap();
    chmod(&m.home.join("exports"), 0o755);
    chmod(&m.home.join("exports/recipe/b1/data.jsonl"), 0o644);

    let (ok, out) = m.run(&["exposure"]);
    assert!(!ok, "a failing rule fails the command:\n{out}");
    assert!(out.contains("fail"), "{out}");
    assert!(out.contains("config.toml (0644)"), "{out}");
    assert!(out.contains("exports"), "{out}");
    assert!(out.contains("--fix"), "{out}");
    assert!(out.to_lowercase().contains("backups"), "{out}");
    let (ok, v) = m.json(&["exposure"]);
    assert!(!ok);
    assert_eq!(outcome(&v, "owner_only"), "fail");
    assert_eq!(outcome(&v, "key_owner_only"), "fail");
    let left = &v["previous_build"][0];
    assert!(left["path"].as_str().unwrap().ends_with("exports"), "{v}");
    assert_eq!(left["mode"], "0755");
    let (_, s) = m.json(&["status"]);
    let warned = s["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["kind"] == "exposure")
        .count();
    assert_eq!(warned, 2, "{s}");

    let out = m.ok(&["exposure", "--fix"]);
    assert!(out.contains("Tightened"), "{out}");
    assert!(loose(&m.home).is_empty(), "{:?}", loose(&m.home));
    let (ok, v) = m.json(&["exposure"]);
    assert!(ok, "{v}");
    assert_eq!(v["previous_build"][0]["mode"], "0700");
}

#[test]
fn a_home_in_a_synced_folder_fails_the_check() {
    let m = Machine::new("Library/CloudStorage/Dropbox/distill");
    m.ok(&["init"]);
    let (ok, v) = m.json(&["exposure"]);
    assert!(!ok);
    assert_eq!(outcome(&v, "not_synced"), "fail");
    assert_eq!(outcome(&v, "owner_only"), "pass");
}
