//! The leak test (issue #72, "Testing Decisions"): a synthetic session with planted secrets goes
//! through every step, the word index included, and then every file under the home and the output
//! of every command is searched for the values. Only the raw archive and that session's events may hold them, and
//! the output of collecting a reveal the owner approved.
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

/// Run `args` as a person would and again with `--json`, except the two that change the config,
/// which run once, keeping what each printed.
fn step(m: &Machine, printed: &mut Vec<(String, String)>, args: &[&str], must_succeed: bool) {
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
    // A token written as a JSON object key, and a short number under a credential-named key.
    let key_token = ["ghp_", &"Zq9".repeat(12)].concat();
    let pin: u64 = 9_000_000 + 382_617;
    let pin_text = pin.to_string();
    let secrets = [
        key.as_str(),
        password.as_str(),
        key_token.as_str(),
        pin_text.as_str(),
    ];

    let tmp = tempfile::tempdir().unwrap();
    let user = tmp.path().join("user");
    fs::create_dir_all(user.join("bin")).unwrap();
    let claude = tmp.path().join("claude");
    fs::create_dir_all(claude.join("proj")).unwrap();
    let said = serde_json::to_string(&format!("deploy with {key} and tell me the key")).unwrap();
    let mut input = serde_json::Map::new();
    input.insert(
        "command".into(),
        format!("export DB_PASSWORD={password} && deploy").into(),
    );
    input.insert(key_token.clone(), "on".into());
    input.insert("api_token".into(), pin.into());
    let input = serde_json::Value::Object(input);
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

    let mut printed: Vec<(String, String)> = Vec::new();
    step(&m, &mut printed, &["init"], true);
    fs::write(
        m.home.join("config.toml"),
        format!(
            "[[harnesses]]\nkind = \"claude-code\"\nenabled = true\nroots = [\"{}\"]\n\n\
             [steward]\ncard_model = \"small:8b\"\nembedding_model = \"embed:small\"\n",
            claude.display()
        ),
    )
    .unwrap();
    step(
        &m,
        &mut printed,
        &["providers", "add", "example-cloud", "--trusted"],
        true,
    );
    for args in [
        &["harnesses"][..],
        &["sweep"],
        &["verify"],
        &["parse"],
        &["scan"],
        &["tick"],
        &["status"],
        &["exposure"],
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
        &["search", "deploy", "--provider", "ollama"],
        &["search", "anthropic_key", "--provider", "example-cloud"],
        &[
            "search",
            "assigned_secret",
            "deploy",
            "--provider",
            "ollama",
        ],
        &["search", &key, "--provider", "ollama"],
        &["search", &password, "--provider", "example-cloud"],
    ] {
        step(&m, &mut printed, args, true);
    }
    // Where each fingerprint appears, by its short form.
    let (_, listed) = m.run(&["--json", "findings", "--kind", "all"]);
    let listed: serde_json::Value = serde_json::from_str(&listed).unwrap();
    let rows = listed["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 4, "{listed}");
    for r in rows {
        let short = r["fingerprint"].as_str().unwrap()[..8].to_string();
        step(&m, &mut printed, &["findings", &short], true);
    }
    // Refused reads print nothing of the session either.
    step(&m, &mut printed, &["read", "leak1"], false);
    step(
        &m,
        &mut printed,
        &["read", "leak1", "--provider", "unheard-of"],
        false,
    );
    step(&m, &mut printed, &["search", "deploy"], false);
    step(
        &m,
        &mut printed,
        &["search", "deploy", "--provider", "unheard-of"],
        false,
    );
    step(&m, &mut printed, &["reads"], true);

    // No local runtime: the model detection pass is skipped, and status says why.
    let status = &printed.iter().find(|(c, _)| c == "status").unwrap().1;
    assert!(status.contains("model detection: skipped ("), "{status}");

    // The reads served placeholders in their place.
    let served = printed
        .iter()
        .find(|(c, _)| c.starts_with("read leak1 --provider example-cloud"))
        .unwrap()
        .1
        .clone();
    assert!(served.contains("[secret: anthropic_key #"), "{served}");
    assert!(served.contains("[secret: assigned_secret #"), "{served}");
    assert!(served.contains("[secret: github_token #"), "{served}");
    assert!(served.contains("[secret: credential_field #"), "{served}");
    let found = &printed
        .iter()
        .find(|(c, _)| c.starts_with("search anthropic_key"))
        .unwrap()
        .1;
    assert!(
        found.contains("deploy with [secret: anthropic_key #"),
        "{found}"
    );

    // A reveal: requested by the remote model, approved by the owner (in the GUI, which calls
    // core), and collected once. Only the collection prints the values.
    let fps: Vec<String> = served
        .split("[secret: ")
        .skip(1)
        .map(|p| {
            let fp = p.split('#').nth(1).unwrap_or_default();
            fp.chars().take_while(|c| c.is_ascii_hexdigit()).collect()
        })
        .collect();
    assert_eq!(fps.len(), 4, "{served}");
    let who = ["--provider", "example-cloud", "--model", "m"];
    let mut request = vec!["reveal", "request", "leak1"];
    request.extend(fps.iter().map(String::as_str));
    request.extend(["--reason", "rotate them"]);
    request.extend(who);
    step(&m, &mut printed, &request, true);
    step(
        &m,
        &mut printed,
        &[
            "reveal",
            "request",
            "leak1",
            &fps[0],
            "--reason",
            "rotate it",
        ],
        false,
    );
    let filed: serde_json::Value = serde_json::from_str(
        &printed
            .iter()
            .rev()
            .find(|(c, _)| c.starts_with("--json reveal request") && c.contains("example-cloud"))
            .unwrap()
            .1,
    )
    .unwrap();
    let id = filed["request"].as_str().unwrap().to_string();
    let mut collect = vec!["reveal", "collect", id.as_str()];
    collect.extend(who);
    step(&m, &mut printed, &collect, false);
    step(&m, &mut printed, &["status"], true);
    distill_core::reveal::approve(&distill_core::Home::new(&m.home), &id).unwrap();
    let (ok, released) = m.run(&collect);
    assert!(ok, "{released}");
    for s in secrets {
        assert!(
            released.contains(s),
            "the collection lacks a value: {released}"
        );
    }
    // Collected once: the second time, and a different reader, get nothing.
    step(&m, &mut printed, &collect, false);
    step(
        &m,
        &mut printed,
        &["reveal", "collect", &id, "--provider", "example-cloud"],
        false,
    );
    step(&m, &mut printed, &["reads"], true);
    step(&m, &mut printed, &["status"], true);

    for (cmd, out) in &printed {
        for s in secrets {
            assert!(!out.contains(s), "`distill {cmd}` printed a secret:\n{out}");
        }
    }
    let log = printed
        .iter()
        .rev()
        .find(|(c, _)| c == "reads")
        .unwrap()
        .1
        .clone();
    for says in ["requested", "approved by the owner", "collected: 4 values"] {
        assert!(log.contains(&format!("reveal {id} {says}")), "{log}");
    }
    // Status counts the requests waiting: none, then the two filed (as a person would and with
    // `--json`), then the one left once the other is approved.
    let statuses: Vec<&String> = printed
        .iter()
        .filter(|(c, _)| c == "status")
        .map(|(_, out)| out)
        .collect();
    for (status, waiting) in statuses
        .iter()
        .zip(["0 requests", "2 requests", "1 request"])
    {
        assert!(
            status.contains(&format!("reveals: {waiting} waiting")),
            "{status}"
        );
    }

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
    // The word index was built, and is among the files searched above.
    let index: Vec<_> = all
        .iter()
        .filter(|f| f.starts_with(m.home.join("index")))
        .collect();
    assert!(index.len() >= 2, "{index:?}");
}
