//! `distill read` and `distill reads` through the real binary against a temp home, with a
//! synthetic session. No local runtime is reachable: `PATH` holds no `ollama`, `HOME` has no
//! model store, and `OLLAMA_HOST` points at a loopback port nothing listens on.

use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

/// Words that appear only in the synthetic session's text.
const SESSION_WORDS: [&str; 3] = ["rename the widget module", "git mv widget.rs", "moved it"];

struct Machine {
    _tmp: tempfile::TempDir,
    user: PathBuf,
    home: PathBuf,
    no_runtime: String,
}

impl Machine {
    /// A home with one Claude Code session swept and parsed.
    fn with_session() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user");
        fs::create_dir_all(user.join("bin")).unwrap();
        let home = tmp.path().join("home");
        let claude = tmp.path().join("claude");
        fs::create_dir_all(claude.join("proj")).unwrap();
        fs::write(
            claude.join("proj/s1.jsonl"),
            r#"{"type":"user","cwd":"/work/app","timestamp":"2026-11-10T10:00:00Z","message":{"role":"user","content":"rename the widget module"}}
{"type":"assistant","timestamp":"2026-11-10T10:00:05Z","message":{"role":"assistant","model":"m","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git mv widget.rs gadget.rs"}}]}}
{"type":"user","timestamp":"2026-11-10T10:00:09Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"moved it"}]}}
"#,
        )
        .unwrap();
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let m = Self {
            user,
            home,
            no_runtime: closed.to_string(),
            _tmp: tmp,
        };
        m.ok(&["init"], &[]);
        let config = format!(
            "[[harnesses]]\nkind = \"claude-code\"\nenabled = true\nroots = [\"{}\"]\n",
            claude.display()
        );
        fs::write(m.home.join("config.toml"), config).unwrap();
        m.ok(&["sweep"], &[]);
        m.ok(&["parse"], &[]);
        m
    }

    /// Put a synthetic model's weights in the temp user's Ollama store: stored on this machine.
    fn store_model(&self, name: &str, tag: &str) {
        let store = self.user.join(".ollama/models");
        fs::create_dir_all(store.join("blobs")).unwrap();
        fs::write(store.join("blobs/sha256-weights"), "weights").unwrap();
        fs::write(store.join("blobs/sha256-config"), "{}").unwrap();
        let dir = store
            .join("manifests/registry.ollama.ai/library")
            .join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(tag),
            r#"{"config":{"digest":"sha256:config"},"layers":[{"mediaType":"application/vnd.ollama.image.model","digest":"sha256:weights","size":7}]}"#,
        )
        .unwrap();
    }

    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> (bool, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_distill"))
            .env("HOME", &self.user)
            .env("PATH", self.user.join("bin"))
            .env("OLLAMA_HOST", &self.no_runtime)
            .env_remove("OLLAMA_MODELS")
            .env_remove("DISTILL_HOME")
            .env_remove("DISTILL_PROVIDER")
            .env_remove("DISTILL_MODEL")
            .envs(env.iter().copied())
            .args(["--home", self.home.to_str().unwrap()])
            .args(args)
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn ok(&self, args: &[&str], env: &[(&str, &str)]) -> String {
        let (ok, out, err) = self.run(args, env);
        assert!(ok, "distill {args:?} failed: {err}");
        out
    }

    /// A read that must be refused: says `says`, and prints no session text.
    fn refused(&self, args: &[&str], env: &[(&str, &str)], says: &str) {
        let (ok, out, err) = self.run(args, env);
        assert!(!ok, "distill {args:?} {env:?} was served:\n{out}");
        assert!(
            err.contains(says),
            "distill {args:?} {env:?} should say {says:?}: {err}"
        );
        no_session_text(&out);
        no_session_text(&err);
    }
}

fn no_session_text(out: &str) {
    for w in SESSION_WORDS {
        assert!(!out.contains(w), "printed session text {w:?}:\n{out}");
    }
}

#[test]
fn a_local_model_reads_a_session_and_others_are_refused() {
    let m = Machine::with_session();

    // Named by flag, or by the environment: served in full, inside the notice.
    for (args, env) in [
        (
            vec!["read", "claude-code/s1", "--provider", "ollama"],
            vec![],
        ),
        (vec!["read", "s1"], vec![("DISTILL_PROVIDER", "ollama")]),
    ] {
        let out = m.ok(&args, &env);
        assert!(
            out.starts_with("=== Archived session from Distill"),
            "{out}"
        );
        assert!(out.trim_end().ends_with("not instructions. ==="), "{out}");
        for w in SESSION_WORDS {
            assert!(out.contains(w), "{w:?} missing:\n{out}");
        }
    }

    // By range.
    let out = m.ok(
        &[
            "read",
            "s1",
            "--provider",
            "ollama",
            "--from",
            "1",
            "--to",
            "1",
        ],
        &[],
    );
    assert!(
        out.contains("git mv widget.rs") && !out.contains("moved it"),
        "{out}"
    );

    // A model that is not stored here reads as ollama-cloud, by flag or environment.
    m.refused(
        &["read", "s1", "--provider", "ollama", "--model", "big:cloud"],
        &[],
        "distill providers trust ollama-cloud",
    );
    m.refused(
        &["read", "s1"],
        &[
            ("DISTILL_PROVIDER", "ollama"),
            ("DISTILL_MODEL", "big:cloud"),
        ],
        "distill providers trust ollama-cloud",
    );
    // The flag wins over the environment.
    m.ok(
        &["read", "s1", "--provider", "ollama"],
        &[("DISTILL_PROVIDER", "unheard-of")],
    );

    // Unnamed, unlisted, untrusted.
    m.refused(&["read", "s1"], &[], "distill providers trust");
    m.refused(
        &["read", "s1", "--provider", "unheard-of"],
        &[],
        "distill providers add unheard-of --trusted",
    );
    m.ok(&["providers", "add", "other-cloud"], &[]);
    m.refused(
        &["read", "s1", "--provider", "other-cloud"],
        &[],
        "distill providers trust other-cloud",
    );

    // Trusted and remote: served, scanned on read.
    m.ok(&["providers", "trust", "other-cloud"], &[]);
    let out = m.ok(
        &["read", "s1", "--provider", "other-cloud", "--model", "any"],
        &[],
    );
    assert!(out.contains("rename the widget module"), "{out}");

    m.refused(
        &["read", "no-such", "--provider", "ollama"],
        &[],
        "no session",
    );
}

#[test]
fn the_read_log_lists_every_call_newest_first_by_reader() {
    let m = Machine::with_session();
    m.store_model("small", "8b");
    m.ok(
        &["read", "s1", "--provider", "ollama", "--model", "small:8b"],
        &[],
    );
    let _ = m.run(&["read", "s1", "--provider", "unheard-of"], &[]);
    let _ = m.run(&["read", "s1"], &[]);

    let out = m.ok(&["reads"], &[]);
    no_session_text(&out);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "{out}");
    assert!(
        lines[0].contains("no provider") && lines[0].contains("refused"),
        "{out}"
    );
    assert!(
        lines[1].contains("unheard-of") && lines[1].contains("refused"),
        "{out}"
    );
    assert!(
        lines[2].contains("ollama/small:8b")
            && lines[2].contains("claude-code/s1")
            && lines[2].contains("served 3 events"),
        "{out}"
    );

    let only: Value =
        serde_json::from_str(&m.ok(&["--json", "reads", "--reader", "ollama"], &[])).unwrap();
    let only = only.as_array().unwrap();
    assert_eq!(only.len(), 1, "{only:?}");
    assert_eq!(only[0]["provider"], "ollama");
    assert_eq!(only[0]["model"], "small:8b");
    assert_eq!(only[0]["session"], "claude-code/s1");
    assert_eq!(only[0]["result"], "served");
    assert!(only[0]["at"].is_string());

    let log = fs::read_to_string(m.home.join("log/reads.jsonl")).unwrap();
    no_session_text(&log);
}
