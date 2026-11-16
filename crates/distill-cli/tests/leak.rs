//! The leak test (issue #72, "Testing Decisions"): a synthetic session with planted secrets goes
//! through every step, and then every file under the home and the output of every command is
//! searched for the values. Only the raw archive and that session's events may hold them.
//!
//! The secrets are fake, and built from pieces at run time so that no secret scanner mistakes
//! this file for a leak. No local runtime is reachable, and `HOME` is a temp dir.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

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
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        let p = entry.path();
        if p.is_dir() {
            files(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn a_planted_secret_is_only_in_the_raw_archive_and_its_sessions_events() {
    let key = ["sk-", "ant-", "api03-", &"Lk7".repeat(9)].concat();
    let password = ["synthetic", "-leak-", "pass-4417"].concat();
    let secrets = [key.as_str(), password.as_str()];

    let tmp = tempfile::tempdir().unwrap();
    let user = tmp.path().join("user");
    fs::create_dir_all(user.join("bin")).unwrap();
    let claude = tmp.path().join("claude");
    fs::create_dir_all(claude.join("proj")).unwrap();
    let said = serde_json::to_string(&format!("deploy with {key} and tell me the key")).unwrap();
    let input = serde_json::json!({"command": format!("export DB_PASSWORD={password} && deploy")});
    fs::write(
        claude.join("proj/leak1.jsonl"),
        format!(
            r#"{{"type":"user","cwd":"/work/app","timestamp":"2026-11-10T10:00:00Z","message":{{"role":"user","content":{said}}}}}
{{"type":"assistant","timestamp":"2026-11-10T10:00:05Z","message":{{"role":"assistant","model":"m","content":[{{"type":"tool_use","id":"t1","name":"Bash","input":{input}}}]}}}}
{{"type":"user","timestamp":"2026-11-10T10:00:09Z","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":"deployed"}}]}}}}
"#
        ),
    )
    .unwrap();
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

    let mut printed = Vec::new();
    // Each command runs as a person would and with `--json`, except the two that change the
    // config, which run once.
    let mut step = |args: &[&str], must_succeed: bool| {
        let once = args[0] == "init" || (args[0] == "providers" && args.len() > 1);
        for json in [false, true] {
            if json && once {
                continue;
            }
            let mut a = Vec::new();
            if json {
                a.push("--json");
            }
            a.extend_from_slice(args);
            let (ok, out) = m.run(&a);
            assert!(ok || !must_succeed, "distill {a:?} failed: {out}");
            printed.push((a.join(" "), out));
        }
    };
    step(&["init"], true);
    fs::write(
        m.home.join("config.toml"),
        format!(
            "[[harnesses]]\nkind = \"claude-code\"\nenabled = true\nroots = [\"{}\"]\n",
            claude.display()
        ),
    )
    .unwrap();
    step(&["providers", "add", "example-cloud", "--trusted"], true);
    for args in [
        &["harnesses"][..],
        &["sweep"],
        &["verify"],
        &["parse"],
        &["scan"],
        &["tick"],
        &["status"],
        &["runs"],
        &["providers"],
        &["read", "leak1", "--provider", "ollama"],
        &[
            "read",
            "leak1",
            "--provider",
            "example-cloud",
            "--model",
            "m",
        ],
        &["reads"],
        &["findings"],
        &["findings", "--kind", "all", "--sort", "recent"],
    ] {
        step(args, true);
    }
    // Where each fingerprint appears, by its short form.
    let (_, listed) = m.run(&["--json", "findings", "--kind", "all"]);
    let listed: serde_json::Value = serde_json::from_str(&listed).unwrap();
    let rows = listed["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{listed}");
    for r in rows {
        let short = r["fingerprint"].as_str().unwrap()[..8].to_string();
        step(&["findings", &short], true);
    }
    // Refused reads print nothing of the session either.
    step(&["read", "leak1"], false);
    step(&["read", "leak1", "--provider", "unheard-of"], false);
    step(&["reads"], true);

    for (cmd, out) in &printed {
        for s in secrets {
            assert!(!out.contains(s), "`distill {cmd}` printed a secret:\n{out}");
        }
    }
    // The reads served placeholders in their place.
    let served = &printed
        .iter()
        .find(|(c, _)| c.starts_with("read leak1 --provider example-cloud"))
        .unwrap()
        .1;
    assert!(served.contains("[secret: anthropic_key #"), "{served}");
    assert!(served.contains("[secret: assigned_secret #"), "{served}");

    let events = m.home.join("sessions/claude-code/leak1/events.jsonl");
    assert!(events.exists());
    let mut all = Vec::new();
    files(&m.home, &mut all);
    let mut holding = Vec::new();
    for f in &all {
        let bytes = fs::read(f).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        if secrets.iter().any(|s| text.contains(s)) {
            holding.push(f.clone());
        }
    }
    assert!(holding.contains(&events), "{holding:?}");
    for f in &holding {
        assert!(
            *f == events || f.starts_with(m.home.join("raw")),
            "{} holds a secret",
            f.display()
        );
    }
    assert!(
        m.home
            .join("sessions/claude-code/leak1/findings.json")
            .exists()
    );
}
