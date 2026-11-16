//! Search through the read path, with synthetic sessions indexed by the tick's index step.

use super::*;
use crate::index::tests::{S, home_with, local, s};
use crate::read::date;

fn ollama() -> Reader {
    Reader::model(Some("ollama"), Some("small:8b"))
}

fn indexed(sessions: &[S]) -> (tempfile::TempDir, Home) {
    let (d, home) = home_with(sessions);
    index::build(&home).unwrap();
    (d, home)
}

fn text(r: Searched) -> String {
    match r {
        Searched::Text(t) => t,
        other => panic!("not text: {other:?}"),
    }
}

fn as_model(home: &Home, q: &str, filter: &Filter) -> String {
    text(search(home, &local(), &ollama(), q, filter, Page::default()).unwrap())
}

/// The session keys in a model's results, in order.
fn keys(t: &str) -> Vec<String> {
    t.lines()
        .filter(|l| l.contains(" · Claude Code"))
        .map(|l| l.split(" · ").next().unwrap().to_string())
        .collect()
}

fn fake_key() -> String {
    ["sk-", "ant-", "api03-", &"Rt8".repeat(9)].concat()
}

#[test]
fn a_model_gets_facts_the_opening_and_the_matching_passage_inside_the_notice() {
    let mut a = s("a", "2026-11-01T09:00:00Z", "the build is broken again");
    a.result = format!("error: linker failed near {} please", fake_key());
    let b = s(
        "b",
        "2026-11-02T09:00:00Z",
        "a linker question, linker linker",
    );
    let (_d, home) = indexed(&[a, b]);
    let t = as_model(&home, "linker", &Filter::default());
    let start = t.find(NOTICE_START).expect("opens the notice");
    let end = t.find(NOTICE_END).expect("closes the notice");
    // b says it more often in less text: it comes first.
    assert_eq!(keys(&t), ["claude-code/b", "claude-code/a"], "{t}");
    for inside in [
        "Opened with: the build is broken again",
        "Matched in event 2 (tool_result): error: linker failed near [secret: anthropic_key #",
        "Matched in event 0 (user): a linker question",
    ] {
        let at = t
            .find(inside)
            .unwrap_or_else(|| panic!("{inside:?} missing: {t}"));
        assert!(start < at && at < end, "{t}");
    }
    for fact in ["/work/app", "m-small", "2026-11-01T09:00:00Z", "3 events"] {
        assert!(t.contains(fact), "{fact:?} missing: {t}");
    }
    assert!(!t.contains(&fake_key()), "{t}");
    assert!(
        t.starts_with("Sessions 1–2 of 2 that match, best first."),
        "{t}"
    );
    assert!(t.contains("This is the last page."), "{t}");
}

#[test]
fn a_passage_in_tool_input_says_where_and_a_long_string_is_cut_around_the_match() {
    let mut a = s("a", "2026-11-01T09:00:00Z", "run it");
    a.command = "cargo build --release".into();
    a.result = format!("{} needle {}", "hay ".repeat(100), "hay ".repeat(100));
    let (_d, home) = indexed(&[a]);
    let t = as_model(&home, "release", &Filter::default());
    assert!(
        t.contains("Matched in event 1 (tool_call, its input): cargo build --release"),
        "{t}"
    );
    let t = as_model(&home, "needle", &Filter::default());
    let line = t.lines().find(|l| l.contains("Matched in")).unwrap();
    assert!(
        line.contains("): …hay hay") && line.ends_with(" hay…"),
        "{line}"
    );
    assert!(line.len() < 400, "{line}");
}

#[test]
fn filters_narrow_the_search_and_the_previous_stewards_sessions_are_left_out_unless_asked_for() {
    let (_d, home) = home_with(&[]);
    let mut sessions = vec![
        s("a", "2026-11-01T09:00:00Z", "deploy the app"),
        S {
            cwd: "/work/other".into(),
            model: "Big-Opus-4".into(),
            ..s("b", "2026-11-03T09:00:00Z", "deploy the other app")
        },
    ];
    // A session that worked inside the home: the previous build's steward ran it.
    sessions.push(S {
        cwd: home.root.join("steward").to_string_lossy().into_owned(),
        ..s("c", "2026-11-05T09:00:00Z", "deploy notes")
    });
    for x in &sessions {
        crate::index::tests::put(&home, x);
    }
    index::build(&home).unwrap();
    let q = |f: Filter| keys(&as_model(&home, "deploy", &f));
    let all = Filter::default();
    // Best first: the same word in less text scores higher.
    assert_eq!(q(all.clone()), ["claude-code/a", "claude-code/b"]);
    assert_eq!(
        q(Filter {
            steward: true,
            ..all.clone()
        }),
        ["claude-code/c", "claude-code/a", "claude-code/b"]
    );
    assert_eq!(
        q(Filter {
            model: Some("opus".into()),
            ..all.clone()
        }),
        ["claude-code/b"]
    );
    assert_eq!(
        q(Filter {
            project: Some("/work/app".into()),
            ..all.clone()
        }),
        ["claude-code/a"]
    );
    assert_eq!(
        q(Filter {
            since: Some(date("2026-11-02").unwrap()),
            ..all.clone()
        }),
        ["claude-code/b"]
    );
    assert_eq!(
        q(Filter {
            harness: Some(crate::Harness::Codex),
            ..all
        }),
        Vec::<String>::new()
    );
}

#[test]
fn untrusted_and_unnamed_providers_get_the_refusal_and_nothing_else() {
    let (_d, home) = indexed(&[s("a", "2026-11-01T09:00:00Z", "deploy the app")]);
    for reader in [
        Reader::model(None, None),
        Reader::model(Some("unheard-of"), Some("m")),
        Reader::model(Some("ollama"), Some("big:cloud")),
    ] {
        let Searched::Refused(why) = search(
            &home,
            &local(),
            &reader,
            "deploy",
            &Filter::default(),
            Page::default(),
        )
        .unwrap() else {
            panic!("{reader:?} is refused");
        };
        assert!(why.contains("distill providers"), "{why}");
        for leak in ["claude-code", "/work/app", "deploy the app"] {
            assert!(!why.contains(leak), "{leak:?} in {why}");
        }
    }
}

#[test]
fn each_search_is_one_read_log_line_with_a_count_and_never_the_query() {
    let (_d, home) = indexed(&[
        s("a", "2026-11-01T09:00:00Z", "deploy the app"),
        s("b", "2026-11-02T09:00:00Z", "deploy again"),
    ]);
    as_model(&home, "deploy", &Filter::default());
    let log = crate::readlog::list(&home, Some("ollama"), usize::MAX).unwrap();
    assert_eq!(log.len(), 1);
    let line = serde_json::to_value(&log[0]).unwrap();
    assert_eq!(line["result"], "listed");
    assert_eq!(line["sessions"], 2);
    assert!(line.get("session").is_none(), "{line}");
    let file = std::fs::read_to_string(home.read_log_path()).unwrap();
    assert!(!file.contains("deploy"), "{file}");
    assert!(
        search(
            &home,
            &local(),
            &ollama(),
            " ?! ",
            &Filter::default(),
            Page::default()
        )
        .is_err()
    );
}

#[test]
fn the_owner_gets_the_stored_passage_with_its_findings() {
    let key = fake_key();
    let mut a = s("a", "2026-11-01T09:00:00Z", "hello");
    a.result = format!("token {key} rotated");
    let (_d, home) = indexed(&[a]);
    let Searched::Hits(h) = search(
        &home,
        &local(),
        &Reader::Owner,
        "rotated",
        &Filter::default(),
        Page::default(),
    )
    .unwrap() else {
        panic!("the owner gets hits");
    };
    assert_eq!((h.total, h.next, h.unindexed), (1, None, 0));
    let hit = &h.hits[0];
    assert_eq!(hit.found_by, [Source::Words]);
    assert_eq!(hit.opening, Opening::Text("hello".into()));
    let p = hit.passage.as_ref().unwrap();
    assert_eq!(
        (p.event, p.kind, &p.field),
        (2, EventKind::ToolResult, &Field::Text)
    );
    assert_eq!(p.text, format!("token {key} rotated"));
    assert_eq!(p.findings.len(), 1);
    assert_eq!(&p.text[p.findings[0].start..p.findings[0].end], key);
}

#[test]
fn a_page_holds_at_most_the_limit_and_says_where_the_next_starts() {
    let sessions: Vec<S> = (0..5)
        .map(|i| {
            s(
                &format!("s{i}"),
                &format!("2026-11-0{}T09:00:00Z", i + 1),
                "same words",
            )
        })
        .collect();
    let (_d, home) = indexed(&sessions);
    let page = |offset, limit| {
        text(
            search(
                &home,
                &local(),
                &ollama(),
                "same",
                &Filter::default(),
                Page { offset, limit },
            )
            .unwrap(),
        )
    };
    let first = page(0, 2);
    // Equal scores: newest first.
    assert_eq!(
        keys(&first),
        ["claude-code/s4", "claude-code/s3"],
        "{first}"
    );
    assert!(first.contains("next page starts at offset 2"), "{first}");
    assert_eq!(keys(&page(4, 2)), ["claude-code/s0"]);
    assert!(page(9, 2).contains("5 match in all"));
}

#[test]
fn sessions_not_yet_indexed_are_counted() {
    let (_d, home) = home_with(&[s("a", "2026-11-01T09:00:00Z", "deploy")]);
    let t = as_model(&home, "deploy", &Filter::default());
    assert!(t.contains("No sessions match."), "{t}");
    assert!(t.contains("1 session is not in the word index yet"), "{t}");
}

#[test]
fn the_owners_hit_carries_the_findings_in_its_opening() {
    let a = s(
        "a",
        "2026-11-01T09:00:00Z",
        &format!("rotate {} today", fake_key()),
    );
    let (_d, home) = indexed(&[a]);
    let Searched::Hits(h) = search(
        &home,
        &local(),
        &Reader::Owner,
        "rotate",
        &Filter::default(),
        Page::default(),
    )
    .unwrap() else {
        panic!("the owner gets hits");
    };
    let hit = &h.hits[0];
    let Opening::Text(t) = &hit.opening else {
        panic!("an opening");
    };
    assert_eq!(hit.opening_findings.len(), 1);
    let f = &hit.opening_findings[0];
    assert_eq!(&t[f.start..f.end], fake_key());
}
