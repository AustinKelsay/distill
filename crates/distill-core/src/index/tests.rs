//! The word index, through its public seams: `index::build` (the tick's step), `index::progress`
//! (status) and `read::search`. Sessions are synthetic, written as the parse step leaves them.

use super::*;
use crate::harness::Harness;
use crate::read::{Filter, Page, Searched};
use crate::runtime::{FakeRuntime, LocalRuntime, Stored};
use crate::trust::Reader;
use crate::util::sha256_hex;

/// A synthetic Claude Code session: what the owner said, one tool call and its result.
#[derive(Clone)]
pub(crate) struct S {
    pub id: String,
    pub cwd: String,
    pub model: String,
    pub started: String,
    pub said: String,
    pub command: String,
    pub result: String,
}

pub(crate) fn s(id: &str, started: &str, said: &str) -> S {
    S {
        id: id.into(),
        cwd: "/work/app".into(),
        model: "m-small".into(),
        started: started.into(),
        said: said.into(),
        command: "ls".into(),
        result: "done".into(),
    }
}

pub(crate) fn home_with(sessions: &[S]) -> (tempfile::TempDir, Home) {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(dir.path().join("home"));
    home.ensure().unwrap();
    for x in sessions {
        put(&home, x);
    }
    (dir, home)
}

/// Archive `x` as a capture of its file, replacing any earlier one, and parse it.
pub(crate) fn put(home: &Home, x: &S) {
    let j = |v: &str| serde_json::to_string(v).unwrap();
    let input = serde_json::json!({ "command": x.command });
    let body = format!(
        r#"{{"type":"user","cwd":{cwd},"timestamp":{t},"message":{{"role":"user","content":{said}}}}}
{{"type":"assistant","timestamp":{t},"message":{{"role":"assistant","model":{model},"content":[{{"type":"tool_use","id":"t1","name":"Bash","input":{input}}}]}}}}
{{"type":"user","timestamp":{t},"message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":{result}}}]}}}}
"#,
        cwd = j(&x.cwd),
        t = j(&x.started),
        said = j(&x.said),
        model = j(&x.model),
        result = j(&x.result),
    );
    let signal = sha256_hex(body.as_bytes());
    crate::archive::Archive::open(home)
        .unwrap()
        .put(
            Harness::ClaudeCode,
            &format!("/s/proj/{}.jsonl", x.id),
            &signal,
            body.as_bytes(),
        )
        .unwrap();
    normalize::normalize(home, false).unwrap();
}

pub(crate) fn local() -> LocalRuntime {
    FakeRuntime::stopped()
        .with_model("small:8b", Stored::Here)
        .runtime()
}

/// The keys the owner's search for `q` finds, best first, with each passage's text.
pub(crate) fn found(home: &Home, q: &str, filter: &Filter) -> Vec<(String, String)> {
    let page = Page {
        offset: 0,
        limit: 50,
    };
    match crate::read::search(home, &local(), &Reader::Owner, q, filter, page).unwrap() {
        Searched::Hits(h) => h
            .hits
            .into_iter()
            .map(|h| (h.facts.key(), h.passage.map(|p| p.text).unwrap_or_default()))
            .collect(),
        other => panic!("the owner gets hits: {other:?}"),
    }
}

fn keys(home: &Home, q: &str) -> Vec<String> {
    found(home, q, &Filter::default())
        .into_iter()
        .map(|(k, _)| k)
        .collect()
}

fn read_log(home: &Home) -> Vec<Value> {
    crate::readlog::list(home, None, usize::MAX)
        .unwrap()
        .iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect()
}

#[test]
fn the_index_is_built_through_the_read_path_as_distill_logged_as_one_line_with_a_count() {
    let (_d, home) = home_with(&[
        s("a", "2026-11-01T09:00:00Z", "rename the widget module"),
        s("b", "2026-11-02T09:00:00Z", "fix the gadget parser"),
    ]);
    let r = build(&home).unwrap();
    assert_eq!((r.sessions, r.indexed, r.current, r.failed), (2, 2, 0, 0));
    let log = read_log(&home);
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(log[0]["reader"], "distill");
    assert_eq!(log[0]["result"], "indexed");
    assert_eq!(log[0]["sessions"], 2);
    assert_eq!(log[0]["events"], 6);
    assert!(log[0].get("session").is_none());

    // Nothing changed: nothing is read, and nothing is logged.
    let r = build(&home).unwrap();
    assert_eq!((r.indexed, r.current), (0, 2));
    assert_eq!(read_log(&home).len(), 1);
    assert_eq!(keys(&home, "widget"), ["claude-code/a"]);
    assert_eq!(keys(&home, "GADGET Parser"), ["claude-code/b"]);
    assert!(
        keys(&home, "widget gadget").is_empty(),
        "every word must match"
    );
}

#[test]
fn an_entry_is_rebuilt_when_its_events_change_and_dropped_when_its_session_goes() {
    let (_d, home) = home_with(&[
        s("a", "2026-11-01T09:00:00Z", "rename the widget module"),
        s("b", "2026-11-02T09:00:00Z", "fix the gadget parser"),
    ]);
    build(&home).unwrap();
    let mut a = s("a", "2026-11-01T09:00:00Z", "rename the sprocket module");
    a.result = "moved sprocket".into();
    put(&home, &a);
    assert_eq!(progress(&home).unwrap().pending, 1);
    let r = build(&home).unwrap();
    assert_eq!((r.indexed, r.current), (1, 1));
    assert_eq!(keys(&home, "sprocket"), ["claude-code/a"]);
    assert!(keys(&home, "widget").is_empty(), "the old text is gone");

    fs::remove_dir_all(home.session_dir(Harness::ClaudeCode, "b")).unwrap();
    normalize::rebuild_session_list(&home).unwrap();
    let r = build(&home).unwrap();
    assert_eq!(r.removed, 1);
    assert!(keys(&home, "gadget").is_empty());
}

#[test]
fn an_entry_is_rebuilt_when_its_secret_findings_change() {
    let secret = ["sk-", "ant-", "api03-", &"Qx3".repeat(9)].concat();
    let (_d, home) = home_with(&[s(
        "a",
        "2026-11-01T09:00:00Z",
        &format!("deploy with {secret} now"),
    )]);
    build(&home).unwrap();
    let before = found(&home, "anthropic_key", &Filter::default());
    assert_eq!(
        before.len(),
        1,
        "a placeholder's kind is a word: {before:?}"
    );
    let short = |home: &Home| {
        let f = crate::findings::load(home, &normalize::read_session_list(home).unwrap()[0]);
        f.unwrap().findings[0].fingerprint.short().to_string()
    };
    let old = short(&home);
    assert_eq!(
        keys(&home, &old),
        ["claude-code/a"],
        "found by its fingerprint"
    );

    // A new fingerprint key: every fingerprint, so every placeholder, changes.
    fs::remove_file(home.fingerprint_key_path()).unwrap();
    crate::findings::scan_all(&home, &home.load_config().unwrap_or_default()).unwrap();
    assert_eq!(progress(&home).unwrap().pending, 1);
    assert_eq!(build(&home).unwrap().indexed, 1);
    let new = short(&home);
    assert_ne!(old, new);
    assert_eq!(keys(&home, &new), ["claude-code/a"]);
    assert!(keys(&home, &old).is_empty(), "the old placeholder is gone");
}

#[test]
fn no_secret_value_reaches_the_index() {
    let key = ["sk-", "ant-", "api03-", &"Wv5".repeat(9)].concat();
    let password = ["synthetic", "-index-", "pass-7781"].concat();
    let mut x = s("a", "2026-11-01T09:00:00Z", &format!("use {key} please"));
    x.command = format!("export DB_PASSWORD={password} && deploy");
    let (_d, home) = home_with(&[x]);
    build(&home).unwrap();
    let mut files = 0;
    for e in fs::read_dir(home.index_dir().join("words")).unwrap() {
        let bytes = fs::read(e.unwrap().path()).unwrap();
        let text = String::from_utf8_lossy(&bytes).to_lowercase();
        for piece in [
            key.to_lowercase(),
            password.clone(),
            "wv5wv5".into(),
            "7781".into(),
        ] {
            assert!(!text.contains(&piece), "{piece} in the index");
        }
        files += 1;
    }
    assert!(files >= 2, "a manifest and a segment");
    assert_eq!(keys(&home, "deploy"), ["claude-code/a"]);
    assert!(keys(&home, &key).is_empty());
    assert!(keys(&home, &password).is_empty());
}

#[test]
fn deleting_the_index_and_building_again_gives_the_same_results() {
    let words = ["alpha", "beta", "gamma", "delta"];
    let sessions: Vec<S> = (0..12)
        .map(|i| {
            let said = format!(
                "{} {} common {}",
                words[i % 4],
                words[(i + 1) % 4],
                "common ".repeat(i)
            );
            s(
                &format!("s{i:02}"),
                &format!("2026-11-01T{i:02}:00:00Z"),
                &said,
            )
        })
        .collect();
    let (_d, home) = home_with(&sessions);
    build(&home).unwrap();
    // Change sessions one at a time, a build each, so that the index splits into many segments
    // and merges them.
    for (i, x) in sessions.iter().enumerate().take(11) {
        let mut x = x.clone();
        x.result = format!("round {i} beta");
        put(&home, &x);
        build(&home).unwrap();
    }
    let p = progress(&home).unwrap();
    assert_eq!((p.current, p.pending), (12, 0));
    assert!(p.segments as usize <= MAX_SEGMENTS, "{p:?}");
    let queries = [
        "common",
        "beta",
        "alpha common",
        "round",
        "gamma delta",
        "done",
    ];
    let incremental: Vec<_> = queries
        .iter()
        .map(|q| found(&home, q, &Filter::default()))
        .collect();
    assert!(
        incremental.iter().all(|r| !r.is_empty()),
        "{incremental:#?}"
    );

    fs::remove_dir_all(home.index_dir()).unwrap();
    assert!(keys(&home, "common").is_empty());
    let r = build(&home).unwrap();
    assert_eq!((r.indexed, r.segments), (12, 3));
    let rebuilt: Vec<_> = queries
        .iter()
        .map(|q| found(&home, q, &Filter::default()))
        .collect();
    assert_eq!(incremental, rebuilt);
}

#[test]
fn a_query_is_answered_from_the_index_without_reading_every_session() {
    let (_d, home) = home_with(&[
        s("a", "2026-11-01T09:00:00Z", "rename the widget module"),
        s("b", "2026-11-02T09:00:00Z", "fix the gadget parser"),
    ]);
    build(&home).unwrap();
    // b's files are gone, though it is still listed: a search that read every session would
    // fail on it.
    fs::remove_dir_all(home.session_dir(Harness::ClaudeCode, "b")).unwrap();
    assert_eq!(
        found(&home, "widget", &Filter::default()),
        [(
            "claude-code/a".to_string(),
            "rename the widget module".to_string()
        )]
    );
}

#[test]
fn progress_counts_current_and_waiting_sessions() {
    let (_d, home) = home_with(&[
        s("a", "2026-11-01T09:00:00Z", "one"),
        s("b", "2026-11-02T09:00:00Z", "two"),
    ]);
    let p = progress(&home).unwrap();
    assert_eq!((p.sessions, p.current, p.pending, p.segments), (2, 0, 2, 0));
    assert!(!home.index_dir().exists(), "progress creates nothing");
    build(&home).unwrap();
    let p = progress(&home).unwrap();
    assert_eq!((p.current, p.pending, p.segments), (2, 0, 1));
    assert!(p.bytes > 0);
}

#[test]
fn words_are_letters_digits_and_underscores_and_a_part_finds_the_whole() {
    assert_eq!(
        query_words("Widget.rs widget, FOO_bar"),
        ["widget", "rs", "foo_bar"]
    );
    let (_d, home) = home_with(&[s(
        "a",
        "2026-11-01T09:00:00Z",
        "call parse_session_list then «ünïcode» word",
    )]);
    build(&home).unwrap();
    for q in ["parse_session_list", "session", "PARSE", "ünïcode"] {
        assert_eq!(keys(&home, q), ["claude-code/a"], "{q}");
    }
    assert!(keys(&home, "sess").is_empty(), "whole words only");
    let long = "x".repeat(MAX_WORD + 1);
    assert!(query_words(&long).is_empty());
}

/// Sessions of mixed sizes: shared words, `_` words, repeats, and some that are long.
fn mixed() -> Vec<S> {
    (0..14)
        .map(|i| {
            let mut x = s(
                &format!("m{i:02}"),
                &format!("2026-11-{:02}T09:00:00Z", i + 1),
                &format!("fix the parse_step_{} widget, then gadget {i}", i % 3),
            );
            x.result = (0..(i * i * 40))
                .map(|n| format!("id{:x} widget_{} ünï ", n * 7 + i, n % 5))
                .collect();
            x
        })
        .collect()
}

type Matches = Vec<(String, Vec<(String, f64, Vec<u64>)>)>;

/// Every query's matches in an index: the sessions, their scores and their events.
fn lookups(home: &Home) -> Matches {
    [
        "widget",
        "gadget",
        "parse",
        "step_1",
        "ünï",
        "id1f",
        "widget_3 gadget",
        "m",
    ]
    .iter()
    .map(|q| {
        let mut got: Vec<_> = lookup(home, &query_words(q))
            .unwrap()
            .matches
            .into_iter()
            .map(|w| (w.key, w.score, w.events))
            .collect();
        got.sort_by(|a, b| a.0.cmp(&b.0));
        (q.to_string(), got)
    })
    .collect()
}

#[test]
fn a_batch_is_written_once_it_passes_its_byte_budget_not_only_its_session_count() {
    let (_d, home) = home_with(&mixed());
    let facts = normalize::read_session_list(&home).unwrap();
    crate::util::create_dir_private(&home.words_dir()).unwrap();
    let mut batch = Batch::default();
    let mut m = Manifest::default();
    assert!(!batch.is_full(1));
    let (mut rounds, mut by_bytes) = (0, 0);
    read::for_index(&home, &facts, |f, served| {
        let (events, findings, card) = served.unwrap();
        let before = batch.bytes;
        batch.add(
            f,
            &events,
            card.as_ref(),
            MadeFrom::of(f, findings, card.as_ref()),
        );
        assert!(batch.bytes > before, "a session with words adds bytes");
        if batch.is_full(20_000) {
            // Written for its bytes, with fewer sessions than a batch holds at most.
            by_bytes += usize::from(batch.sessions.len() < BATCH);
            batch.flush(&home, &mut m)?;
            assert_eq!((batch.bytes, batch.sessions.len()), (0, 0));
            rounds += 1;
        }
        Ok(())
    })
    .unwrap();
    assert!(rounds >= 2 && by_bytes >= 1, "{rounds} {by_bytes}");
}

#[test]
fn a_build_split_by_bytes_or_in_passes_gives_the_same_matches_as_one_batch() {
    let sessions = mixed();
    let (_a, one) = home_with(&sessions);
    let (_b, by_bytes) = home_with(&sessions);
    let (_c, passes) = home_with(&sessions);
    let roomy = Limits {
        budget: usize::MAX,
        giant: usize::MAX,
        pass: u64::MAX,
    };
    build_within(&one, roomy).unwrap();
    build_within(&by_bytes, Limits { budget: 1, ..roomy }).unwrap();
    // Every session is a giant here, in passes of a few hundred words.
    build_within(
        &passes,
        Limits {
            giant: 0,
            pass: 300,
            ..roomy
        },
    )
    .unwrap();
    let want = lookups(&one);
    assert!(want.iter().any(|(_, m)| m.len() > 3), "{want:?}");
    assert_eq!(lookups(&by_bytes), want);
    assert_eq!(lookups(&passes), want);
    let counts = |h: &Home| -> Vec<(String, u64)> {
        load(h)
            .entries
            .into_iter()
            .map(|(k, e)| (k, e.words))
            .collect()
    };
    assert_eq!(counts(&by_bytes), counts(&one));
    assert_eq!(counts(&passes), counts(&one));
    // The owner's search ranks them the same too.
    for q in ["widget", "gadget 3", "parse_step_1"] {
        let w = found(&one, q, &Filter::default());
        assert_eq!(found(&by_bytes, q, &Filter::default()), w, "{q}");
        assert_eq!(found(&passes, q, &Filter::default()), w, "{q}");
    }
}
