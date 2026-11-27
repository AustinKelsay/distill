//! `distill search` through the real binary against a temp home: synthetic sessions are swept,
//! parsed, scanned and indexed by a tick, then searched as a model at a named provider. No local
//! runtime is reachable, and `HOME` is a temp dir.

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

fn claude(cwd: &str, at: &str, said: &str, model: &str, result: &str) -> String {
    let j = |s: &str| serde_json::to_string(s).unwrap();
    format!(
        r#"{{"type":"user","cwd":{cwd},"timestamp":{at},"message":{{"role":"user","content":{said}}}}}
{{"type":"assistant","timestamp":{at},"message":{{"role":"assistant","model":{model},"content":[{{"type":"tool_use","id":"t1","name":"Bash","input":{{"command":"cargo test"}}}}]}}}}
{{"type":"user","timestamp":{at},"message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":{result}}}]}}}}
"#,
        cwd = j(cwd),
        at = j(at),
        said = j(said),
        model = j(model),
        result = j(result),
    )
}

impl Machine {
    fn new(sessions: &[(&str, String)]) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user");
        fs::create_dir_all(user.join("bin")).unwrap();
        let root = tmp.path().join("claude");
        fs::create_dir_all(root.join("proj")).unwrap();
        for (id, body) in sessions {
            fs::write(root.join(format!("proj/{id}.jsonl")), body).unwrap();
        }
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let m = Self {
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
                root.display()
            ),
        )
        .unwrap();
        m.ok(&["tick"]);
        m
    }

    fn run(&self, args: &[&str]) -> (bool, String, String) {
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
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn ok(&self, args: &[&str]) -> String {
        let (ok, out, err) = self.run(args);
        assert!(ok, "distill {args:?} failed: {err}");
        out
    }
}

/// The session keys a search printed, in order.
fn keys(out: &str) -> Vec<&str> {
    out.lines()
        .filter(|l| l.contains(" · Claude Code"))
        .map(|l| l.split(" · ").next().unwrap())
        .collect()
}

fn sessions() -> Vec<(&'static str, String)> {
    vec![
        (
            "s1",
            claude(
                "/work/app",
                "2026-11-01T10:00:00Z",
                "fix the failing test",
                "m-small",
                "thread panicked at overflow_check",
            ),
        ),
        (
            "s2",
            claude(
                "/work/site",
                "2026-11-05T10:00:00Z",
                "speed up the page",
                "Big-Opus-4",
                "all fine, no overflow",
            ),
        ),
    ]
}

#[test]
fn a_model_searches_by_words_and_filters_and_others_are_refused() {
    let m = Machine::new(&sessions());
    let out = m.ok(&["search", "overflow", "--provider", "ollama"]);
    // Each says it once; s2 in fewer words, so it scores higher.
    assert_eq!(keys(&out), ["claude-code/s2", "claude-code/s1"], "{out}");
    assert!(out.contains("=== Archived session from DISTILL"), "{out}");
    assert!(
        out.contains("Opened with: fix the failing test")
            && out.contains("Matched in event 2 (tool_result): thread panicked at overflow_check"),
        "{out}"
    );
    for (args, want) in [
        (&["--used-model", "opus"][..], vec!["claude-code/s2"]),
        (&["--project", "/work/app"], vec!["claude-code/s1"]),
        (&["--since", "2026-11-02"], vec!["claude-code/s2"]),
        (&["--until", "2026-11-02"], vec!["claude-code/s1"]),
        (&["--harness", "codex"], vec![]),
        (&["--limit", "1"], vec!["claude-code/s2"]),
        (&["--offset", "1"], vec!["claude-code/s1"]),
    ] {
        let mut a = vec!["search", "overflow", "--provider", "ollama"];
        a.extend_from_slice(args);
        assert_eq!(keys(&m.ok(&a)), want, "{a:?}");
    }
    let json: serde_json::Value = serde_json::from_str(&m.ok(&[
        "--json",
        "search",
        "PANICKED",
        "check",
        "--provider",
        "ollama",
    ]))
    .unwrap();
    assert!(json["text"].as_str().unwrap().contains("claude-code/s1"));

    for args in [
        &["search", "overflow"][..],
        &["search", "overflow", "--provider", "unheard-of"],
    ] {
        let (ok, out, err) = m.run(args);
        assert!(
            !ok && err.contains("refused") && err.contains("distill providers"),
            "{err}"
        );
        assert!(
            !out.contains("overflow") && !err.contains("failing test"),
            "{out}{err}"
        );
    }
    let (ok, _, err) = m.run(&["search", "x", "--provider", "ollama", "--since", "Nov 2"]);
    assert!(!ok && err.contains("YYYY-MM-DD"), "{err}");

    // DISTILL's reads for the index were one line, and the search another.
    let reads = m.ok(&["reads"]);
    assert!(reads.contains("indexed 2 sessions, 6 events"), "{reads}");
    assert!(reads.contains("listed 2 sessions"), "{reads}");
}

fn tree(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        if e.path().is_dir() {
            tree(&e.path(), out);
        } else {
            out.push(e.path());
        }
    }
}

#[test]
fn deleting_the_index_and_running_the_tick_rebuilds_it_with_the_same_results() {
    let m = Machine::new(&sessions());
    let queries: [&[&str]; 3] = [&["overflow"], &["cargo", "test"], &["page"]];
    let search = |q: &[&str]| {
        let mut a = vec!["search"];
        a.extend_from_slice(q);
        a.extend(["--provider", "ollama"]);
        m.ok(&a)
    };
    let before: Vec<String> = queries.iter().map(|q| search(q)).collect();
    let st: serde_json::Value = serde_json::from_str(&m.ok(&["--json", "status"])).unwrap();
    assert_eq!(st["index"]["current"], 2, "{st}");

    let mut files = Vec::new();
    tree(&m.home.join("index"), &mut files);
    assert!(!files.is_empty());
    fs::remove_dir_all(m.home.join("index")).unwrap();
    let st: serde_json::Value = serde_json::from_str(&m.ok(&["--json", "status"])).unwrap();
    assert_eq!(st["index"]["pending"], 2, "{st}");
    assert!(search(&["overflow"]).contains("2 sessions are not in the word index yet"));

    m.ok(&["tick"]);
    let after: Vec<String> = queries.iter().map(|q| search(q)).collect();
    assert_eq!(before, after);
    assert!(
        m.ok(&["status"])
            .contains("word index: 2 of 2 sessions current")
    );
}

#[test]
fn with_no_embedding_model_or_no_local_runtime_search_finds_by_words_and_says_so() {
    let m = Machine::new(&sessions());
    let out = m.ok(&["search", "overflow", "--provider", "ollama"]);
    assert!(
        out.contains(
            "Search by meaning is off (no embedding_model is set), so these are word matches only."
        ),
        "{out}"
    );
    let config = fs::read_to_string(m.home.join("config.toml")).unwrap();
    fs::write(
        m.home.join("config.toml"),
        format!("{config}\n[steward]\nembedding_model = \"embed:small\"\n"),
    )
    .unwrap();
    // No runtime is reachable here: the tick skips the vectors, and search says why.
    m.ok(&["tick"]);
    let out = m.ok(&["search", "overflow", "--provider", "ollama"]);
    assert_eq!(keys(&out), ["claude-code/s2", "claude-code/s1"], "{out}");
    assert!(
        out.contains("Search by meaning is off (")
            && out.contains("so these are word matches only."),
        "{out}"
    );
    assert!(!out.contains("no embedding_model is set"), "{out}");
    let st = m.ok(&["status"]);
    assert!(
        st.contains("vectors: skipped (") && st.contains("search by meaning is off"),
        "{st}"
    );
}

#[test]
fn search_supports_exclusion_shape_and_tool_filters() {
    let s1 = [
        r#"{"type":"user","timestamp":"2026-11-01T10:00:00Z","cwd":"/work/keep","message":{"role":"user","content":"overflow one"}}"#,
        r#"{"type":"assistant","timestamp":"2026-11-01T10:01:30Z","message":{"model":"claude-sonnet-4-6","content":[{"type":"thinking","thinking":"check bounds"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}},{"type":"text","text":"fixed overflow"}]}}"#,
        r#"{"type":"user","timestamp":"2026-11-01T10:02:00Z","cwd":"/work/keep","message":{"role":"user","content":"thanks"}}"#,
    ]
    .join("\n")
        + "\n";
    let s2 = [
        r#"{"type":"user","timestamp":"2026-11-02T10:00:00Z","cwd":"/work/drop","message":{"role":"user","content":"overflow two"}}"#,
        r#"{"type":"assistant","timestamp":"2026-11-02T10:00:05Z","message":{"model":"claude-sonnet-4-6","content":[{"type":"text","text":"quick answer"}]}}"#,
    ]
    .join("\n")
        + "\n";
    let m = Machine::new(&[("s1.jsonl", s1), ("s2.jsonl", s2)]);

    // Parse-time shape facts are recorded in meta.json.
    let meta: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(m.home.join("sessions/claude-code/s1/meta.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(meta["duration_secs"], 120);
    assert_eq!(meta["tools"], serde_json::json!(["Bash"]));
    assert_eq!(meta["counts"]["reasoning"], 1);
    assert!(meta["counts"]["bytes_reasoning"].as_u64().unwrap() > 0);

    let out = m.ok(&[
        "search",
        "overflow",
        "--provider",
        "ollama",
        "--exclude-project",
        "/work/drop",
        "--has-reasoning",
        "--min-turns",
        "2",
        "--min-tool-calls",
        "1",
        "--tool",
        "bash",
    ]);
    assert_eq!(keys(&out), ["claude-code/s1"], "{out}");

    // Bare --has-reasoning before positional query does not consume the query word.
    let bare_before_query = m.ok(&[
        "search",
        "--provider",
        "ollama",
        "--has-reasoning",
        "overflow",
    ]);
    assert_eq!(
        keys(&bare_before_query),
        ["claude-code/s1"],
        "{bare_before_query}"
    );

    let no_reason = m.ok(&[
        "search",
        "overflow",
        "--provider",
        "ollama",
        "--has-reasoning=false",
        "--max-turns",
        "1",
    ]);
    assert_eq!(keys(&no_reason), ["claude-code/s2"], "{no_reason}");

    let excluded = m.ok(&[
        "search",
        "overflow",
        "--provider",
        "ollama",
        "--exclude-harness",
        "claude-code",
    ]);
    assert!(keys(&excluded).is_empty(), "{excluded}");
}
