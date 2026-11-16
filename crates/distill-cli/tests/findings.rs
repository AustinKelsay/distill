//! `distill findings` through the real binary against a temp home: the rotation list, a lookup
//! by short form, and the read log. The secrets are fake, and built from pieces at run time so
//! that no secret scanner mistakes this file for a leak. No local runtime is reachable.

use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

struct Machine {
    _tmp: tempfile::TempDir,
    user: PathBuf,
    home: PathBuf,
    no_runtime: String,
}

impl Machine {
    fn run(&self, args: &[&str]) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_distill"))
            .env("HOME", &self.user)
            .env("PATH", self.user.join("bin"))
            .env("OLLAMA_HOST", &self.no_runtime)
            .env_remove("OLLAMA_MODELS")
            .env_remove("DISTILL_HOME")
            .env_remove("DISTILL_PROVIDER")
            .env_remove("DISTILL_MODEL")
            .args(["--home", self.home.to_str().unwrap()])
            .args(args)
            .output()
            .unwrap();
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), printed)
    }

    fn ok(&self, args: &[&str]) -> String {
        let (ok, out) = self.run(args);
        assert!(ok, "distill {args:?} failed: {out}");
        out
    }
}

/// A home with two Claude Code sessions that share one fake key, swept, parsed and scanned.
fn machine(key: &str) -> Machine {
    let tmp = tempfile::tempdir().unwrap();
    let user = tmp.path().join("user");
    fs::create_dir_all(user.join("bin")).unwrap();
    let claude = tmp.path().join("claude");
    fs::create_dir_all(claude.join("proj")).unwrap();
    for (id, day) in [("f1", "01"), ("f2", "07")] {
        let said = serde_json::to_string(&format!("deploy with {key}")).unwrap();
        fs::write(
            claude.join(format!("proj/{id}.jsonl")),
            format!(
                r#"{{"type":"user","cwd":"/work/app","timestamp":"2026-11-{day}T10:00:00Z","message":{{"role":"user","content":{said}}}}}
"#
            ),
        )
        .unwrap();
    }
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let m = Machine {
        user,
        home: tmp.path().join("home"),
        no_runtime: closed.to_string(),
        _tmp: tmp,
    };
    m.ok(&["init"]);
    fs::write(
        m.home.join("config.toml"),
        format!(
            "[[harnesses]]\nkind = \"claude-code\"\nenabled = true\nroots = [\"{}\"]\n",
            claude.display()
        ),
    )
    .unwrap();
    for step in ["sweep", "parse", "scan"] {
        m.ok(&[step]);
    }
    m
}

#[test]
fn findings_lists_the_rotation_list_and_where_one_fingerprint_appears() {
    let key = ["sk-", "ant-", "api03-", &"Rw5".repeat(9)].concat();
    let m = machine(&key);

    let list = m.ok(&["findings"]);
    assert!(
        list.contains("anthropic_key") && list.contains("2 sessions"),
        "{list}"
    );
    assert!(list.contains("first seen 2026-11-01") && list.contains("last seen 2026-11-07"));
    let json: Value =
        serde_json::from_str(&m.ok(&["--json", "findings", "--sort", "recent"])).unwrap();
    let fingerprint = json["rows"][0]["fingerprint"].as_str().unwrap().to_string();
    assert_eq!(json["rows"][0]["sessions"], 2);
    assert_eq!(json["total"], 1);

    let short = &fingerprint[..8];
    let found = m.ok(&["findings", short]);
    assert!(
        found.contains("claude-code/f1") && found.contains("claude-code/f2"),
        "{found}"
    );
    assert!(
        found.find("claude-code/f2") < found.find("claude-code/f1"),
        "{found}"
    );

    let filtered = m.ok(&["findings", "--harness", "codex"]);
    assert!(filtered.contains("No fingerprints match"), "{filtered}");
    let filtered = m.ok(&[
        "findings",
        "--since",
        "2026-11-05",
        "--kind",
        "anthropic_key",
    ]);
    assert!(filtered.contains("1 session"), "{filtered}");
    assert!(
        m.ok(&["findings", "--kind", "personal"])
            .contains("No fingerprints match")
    );
    let (ok, bad) = m.run(&["findings", "--kind", "tea_leaves"]);
    assert!(!ok && bad.contains("unknown kind"), "{bad}");

    for out in [list, found] {
        assert!(!out.contains(&key), "{out}");
    }
    let reads = m.ok(&["reads"]);
    assert!(
        reads.contains("(findings)") && reads.contains("listed 1 fingerprint in 2 sessions"),
        "{reads}"
    );
}
