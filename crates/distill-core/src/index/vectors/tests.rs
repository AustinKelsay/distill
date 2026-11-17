//! Vectors and search by meaning, through their public seams: `vectors::build` (the tick's step),
//! `vectors::progress` (status), `read::search` and the tick. The fake runtime embeds with fixed
//! vectors: one dimension per topic, counting the topic's words, so texts that share no word can
//! still be close.

use super::*;
use crate::harness::Harness;
use crate::index::tests::{S, home_with, put, s};
use crate::read::{Filter, Hit, Meaning, Page, Searched, Source};
use crate::runtime::{Call, FakeRuntime, Stored};
use crate::tick::Deadline;
use crate::trust::Reader;

const EMBED: &str = "embed:small";
const CARDS: &str = "small:8b";

/// The topics the fake embeds by; a text's vector counts each topic's words in it.
const TOPICS: [&[&str]; 4] = [
    &["car", "vehicle", "automobile", "engine", "brakes", "tyre"],
    &["cat", "kitten", "feline", "pet", "vet"],
    &["invoice", "budget", "money", "payment", "billing"],
    &["deploy", "release", "ship", "production", "rollout"],
];

fn topics(text: &str) -> Vec<f32> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .collect();
    TOPICS
        .iter()
        .map(|t| words.iter().filter(|w| t.contains(&w.as_str())).count() as f32)
        .collect()
}

fn fake() -> FakeRuntime {
    FakeRuntime::running()
        .with_model(EMBED, Stored::Here)
        .with_model(CARDS, Stored::Here)
        .embedding(topics)
}

/// A home holding `sessions`, scanned, with a config whose `embedding_model` is [`EMBED`].
fn home(sessions: &[S]) -> (tempfile::TempDir, Home, Config) {
    let (d, home) = home_with(sessions);
    let mut config = Config {
        harnesses: vec![],
        ..Default::default()
    };
    config.steward.embedding_model = Some(EMBED.into());
    config.steward.card_model = Some(CARDS.into());
    config.save(&home.config_path()).unwrap();
    findings::scan_all(&home, &config).unwrap();
    (d, home, config)
}

/// The index and the vectors, built.
fn built(sessions: &[S]) -> (tempfile::TempDir, Home, Config) {
    let (d, home, config) = home(sessions);
    crate::index::build(&home).unwrap();
    build(&home, &config, &fake().runtime(), Deadline::NONE).unwrap();
    (d, home, config)
}

fn owner_search(home: &Home, runtime: &LocalRuntime, q: &str) -> crate::read::Hits {
    let page = Page {
        offset: 0,
        limit: 50,
    };
    match crate::read::search(home, runtime, &Reader::Owner, q, &Filter::default(), page).unwrap() {
        Searched::Hits(h) => *h,
        other => panic!("the owner gets hits: {other:?}"),
    }
}

fn ranked(hits: &[Hit]) -> Vec<(String, Vec<Source>)> {
    hits.iter()
        .map(|h| (h.facts.key(), h.found_by.clone()))
        .collect()
}

fn embedded(fake: &FakeRuntime) -> Vec<Vec<String>> {
    fake.calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Embed { model, texts } => {
                assert_eq!(model, EMBED);
                Some(texts)
            }
            Call::Generate { .. } => None,
        })
        .collect()
}

fn fake_key() -> String {
    ["sk-", "ant-", "api03-", &"Vq3".repeat(9)].concat()
}

fn card_reply(summary: &str) -> String {
    serde_json::json!({"title": "A session", "summary": summary, "tags": [], "outcome": "done"})
        .to_string()
}

#[test]
fn the_steward_embeds_what_the_owner_wrote_with_placeholders_and_the_card_summary() {
    let mut a = s(
        "a",
        "2026-11-01T09:00:00Z",
        &format!("the engine stalls, the key is {}", fake_key()),
    );
    a.command = "deploy to production".into();
    a.result = "billing invoice".into();
    let (_d, home, config) = home(&[a]);
    let f = fake().replying(|_, _| card_reply("Looked at why the vehicle stalls."));
    cards::write(&home, &config, &f.runtime(), Deadline::NONE).unwrap();

    let r = build(&home, &config, &f.runtime(), Deadline::NONE).unwrap();
    assert_eq!(
        (r.sessions, r.embedded, r.current, r.failed, r.vectors),
        (1, 1, 0, 0, 2),
        "{r:?}"
    );
    let texts = embedded(&f);
    assert_eq!(texts.len(), 1, "one call for the session: {texts:?}");
    // What the owner wrote, with the placeholder; not the tool call or its result. Then the
    // card's summary.
    assert_eq!(texts[0].len(), 2, "{texts:?}");
    assert!(
        texts[0][0].starts_with("the engine stalls, the key is [secret: anthropic_key #"),
        "{texts:?}"
    );
    assert_eq!(texts[0][1], "Looked at why the vehicle stalls.");
    let file = std::fs::read(home.vectors_path()).unwrap();
    assert!(!String::from_utf8_lossy(&file).contains(&fake_key()));

    // The read was logged as the embedding model at the local provider, one line with counts.
    let log: Vec<serde_json::Value> = crate::readlog::list(&home, None, usize::MAX)
        .unwrap()
        .iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect();
    let line = log
        .iter()
        .find(|l| l["model"] == EMBED)
        .unwrap_or_else(|| panic!("{log:#?}"));
    assert_eq!(
        (&line["provider"], &line["result"], &line["sessions"]),
        (&"ollama".into(), &"indexed".into(), &1.into()),
        "{line}"
    );

    // Built again, nothing has changed, so nothing is embedded.
    let r = build(&home, &config, &f.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.embedded, r.current), (0, 1), "{r:?}");
    assert_eq!(embedded(&f).len(), 1);
}

#[test]
fn search_by_meaning_finds_a_session_whose_wording_differs_from_the_query() {
    let (_d, home, _) = built(&[
        s(
            "a",
            "2026-11-01T09:00:00Z",
            "the automobile engine will not start",
        ),
        s("b", "2026-11-02T09:00:00Z", "my kitten needs a vet"),
        s(
            "c",
            "2026-11-03T09:00:00Z",
            "send the invoice for the budget",
        ),
    ]);
    let f = fake();
    // No session has "car" or "trouble": words find nothing, meaning finds the automobile.
    let h = owner_search(&home, &f.runtime(), "car trouble");
    assert_eq!(
        ranked(&h.hits),
        [("claude-code/a".to_string(), vec![Source::Meaning])]
    );
    assert_eq!(h.meaning, Meaning::Used { unembedded: 0 });
    let p = h.hits[0].passage.as_ref().unwrap();
    assert_eq!(
        (p.event, p.text.as_str()),
        (0, "the automobile engine will not start")
    );
    // The query was embedded with the embedding model.
    assert_eq!(embedded(&f), [vec!["car trouble".to_string()]]);
}

#[test]
fn word_and_meaning_results_are_merged_into_one_ranked_list() {
    let (_d, home, _) = built(&[
        // Only about cars, in other words.
        s(
            "a",
            "2026-11-01T09:00:00Z",
            "the vehicle engine and its brakes",
        ),
        // Says "car", but is mostly about money.
        s(
            "b",
            "2026-11-02T09:00:00Z",
            "car invoice payment billing budget",
        ),
        s("c", "2026-11-03T09:00:00Z", "my kitten needs a vet"),
        // Says "car" and is about cars.
        s("d", "2026-10-30T09:00:00Z", "car engine brakes tyre"),
    ]);
    let h = owner_search(&home, &fake().runtime(), "car");
    // Words: b and d (d says it in less text, b is newer; BM25 decides). Meaning: a and d are
    // as close as can be, b far behind, c not at all. d is high in both and comes first; a,
    // closest by meaning, and b, found by both but low in meaning, follow.
    let r = ranked(&h.hits);
    assert_eq!(
        r[0],
        ("claude-code/d".into(), vec![Source::Words, Source::Meaning])
    );
    let rest: Vec<&str> = r[1..].iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(rest.len(), 2, "{r:?}");
    assert!(
        rest.contains(&"claude-code/a") && rest.contains(&"claude-code/b"),
        "{r:?}"
    );
    assert!(!r.iter().any(|(k, _)| k == "claude-code/c"), "{r:?}");
    assert_eq!(h.total, 3);
}

#[test]
fn a_model_searching_reads_the_merged_list_and_is_told_when_meaning_is_off() {
    let (_d, home, config) = built(&[
        s(
            "a",
            "2026-11-01T09:00:00Z",
            "the automobile engine will not start",
        ),
        s("b", "2026-11-02T09:00:00Z", "the car is parked"),
    ]);
    let reader = Reader::model(Some("ollama"), None);
    let text = |rt: &LocalRuntime| match crate::read::search(
        &home,
        rt,
        &reader,
        "car",
        &Filter::default(),
        Page::default(),
    )
    .unwrap()
    {
        Searched::Text(t) => t,
        other => panic!("{other:?}"),
    };
    let t = text(&fake().runtime());
    assert!(t.starts_with("Sessions 1–2 of 2 that match"), "{t}");
    assert!(!t.contains("Search by meaning is off"), "{t}");

    for (rt, why) in [
        (
            FakeRuntime::stopped()
                .with_model(EMBED, Stored::Here)
                .runtime(),
            "the local runtime is not running",
        ),
        (
            FakeRuntime::not_installed().runtime(),
            "no local runtime is installed",
        ),
        (
            FakeRuntime::running()
                .with_model(EMBED, Stored::Cloud)
                .runtime(),
            "embedding_model \"embed:small\" is not stored on this machine",
        ),
    ] {
        let t = text(&rt);
        assert!(t.starts_with("Sessions 1–1 of 1 that match"), "{t}");
        assert!(
            t.contains(&format!(
                "Search by meaning is off ({why}), so these are word matches only."
            )),
            "{t}"
        );
    }
    let mut unset = config.clone();
    unset.steward.embedding_model = None;
    unset.save(&home.config_path()).unwrap();
    let t = text(&fake().runtime());
    assert!(
        t.contains("Search by meaning is off (no embedding_model is set)"),
        "{t}"
    );
    let h = owner_search(&home, &fake().runtime(), "car");
    assert_eq!(
        h.meaning,
        Meaning::Skipped("no embedding_model is set".into())
    );
}

#[test]
fn vectors_are_rebuilt_when_their_events_secret_findings_or_card_change() {
    let a = s(
        "a",
        "2026-11-01T09:00:00Z",
        "the automobile engine will not start",
    );
    let (_d, home, config) = home(std::slice::from_ref(&a));
    let f = fake();
    let embeds = || embedded(&f).len();
    build(&home, &config, &f.runtime(), Deadline::NONE).unwrap();
    assert_eq!(embeds(), 1);

    // New events: the session is captured again.
    let mut changed = a.clone();
    changed.said = "my kitten needs a vet".into();
    put(&home, &changed);
    findings::scan_all(&home, &config).unwrap();
    let r = build(&home, &config, &f.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.embedded, r.pending), (1, 1), "{r:?}");
    assert_eq!(embedded(&f)[1], ["my kitten needs a vet"]);

    // A card is written: its summary joins the vectors.
    let cf = fake().replying(|_, _| card_reply("Took the kitten to the vet."));
    cards::write(&home, &config, &cf.runtime(), Deadline::NONE).unwrap();
    let r = build(&home, &config, &f.runtime(), Deadline::NONE).unwrap();
    assert_eq!(r.embedded, 1, "{r:?}");
    assert_eq!(
        embedded(&f)[2],
        ["my kitten needs a vet", "Took the kitten to the vet."]
    );

    // Different secret findings: a personal term now found in the session.
    let mut c = config.clone();
    c.privacy.personal_terms = vec!["kitten".into()];
    c.save(&home.config_path()).unwrap();
    findings::scan_all(&home, &c).unwrap();
    let before = embeds();
    build(&home, &c, &f.runtime(), Deadline::NONE).unwrap();
    // A personal detail changes no placeholder, so the vectors stay current.
    assert_eq!(embeds(), before);
    let mut secret = changed.clone();
    secret.said = format!("my kitten needs a vet, key {}", fake_key());
    put(&home, &secret);
    findings::scan_all(&home, &c).unwrap();
    let r = build(&home, &c, &f.runtime(), Deadline::NONE).unwrap();
    assert_eq!(r.embedded, 1, "{r:?}");
    let last = embedded(&f).pop().unwrap();
    assert!(
        last[0].contains("[secret: anthropic_key #") && !last[0].contains(&fake_key()),
        "{last:?}"
    );
}

#[test]
fn deleting_the_file_and_running_the_tick_rebuilds_it_with_the_same_results() {
    let (_d, home, config) = built(&[
        s(
            "a",
            "2026-11-01T09:00:00Z",
            "the vehicle engine and its brakes",
        ),
        s(
            "b",
            "2026-11-02T09:00:00Z",
            "car invoice payment billing budget",
        ),
        s("c", "2026-11-03T09:00:00Z", "my kitten needs a vet"),
    ]);
    let before = ranked(&owner_search(&home, &fake().runtime(), "car pet").hits);
    let bytes = std::fs::read(home.vectors_path()).unwrap();
    std::fs::remove_file(home.vectors_path()).unwrap();
    let h = owner_search(&home, &fake().runtime(), "car pet");
    assert_eq!(
        h.meaning,
        Meaning::Skipped("no vectors have been made with \"embed:small\" yet".into())
    );
    let t = crate::tick::tick_with(&home, &config, &fake().runtime()).unwrap();
    let step = t.steps.iter().find(|s| s.name == "vectors").unwrap();
    assert!(step.ok, "{step:?}");
    assert_eq!(step.report["embedded"], 3, "{step:?}");
    assert_eq!(std::fs::read(home.vectors_path()).unwrap(), bytes);
    assert_eq!(
        ranked(&owner_search(&home, &fake().runtime(), "car pet").hits),
        before
    );
}

#[test]
fn vectors_of_another_model_are_rebuilt_whole() {
    let (_d, home, mut config) = built(&[s("a", "2026-11-01T09:00:00Z", "the car")]);
    config.steward.embedding_model = Some("other:embed".into());
    config.save(&home.config_path()).unwrap();
    let f = FakeRuntime::running()
        .with_model("other:embed", Stored::Here)
        .embedding(topics);
    let p = progress(&home).unwrap();
    assert_eq!((p.current, p.pending), (0, 1), "{p:?}");
    let r = build(&home, &config, &f.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.embedded, r.current), (1, 0), "{r:?}");
}

#[test]
fn sessions_are_embedded_newest_first_up_to_sessions_per_run_and_never_mirrors_or_the_old_steward()
{
    let (_d, home, mut config) = home(&[
        s("old", "2026-11-01T09:00:00Z", "the car"),
        s("mid", "2026-11-02T09:00:00Z", "the kitten"),
        s("new", "2026-11-03T09:00:00Z", "the invoice"),
        s("stew", "2026-11-04T09:00:00Z", "the release"),
    ]);
    // The previous build's steward ran "stew".
    let meta = home.meta_path(Harness::ClaudeCode, "stew");
    let mut m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&meta).unwrap()).unwrap();
    m["steward"] = true.into();
    std::fs::write(&meta, m.to_string()).unwrap();
    crate::normalize::rebuild_session_list(&home).unwrap();
    config.steward.sessions_per_run = 2;
    let f = fake();
    let r = build(&home, &config, &f.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.sessions, r.pending, r.embedded), (3, 3, 2), "{r:?}");
    assert_eq!(embedded(&f), [vec!["the invoice"], vec!["the kitten"]]);
    let p = progress(&home).unwrap();
    assert_eq!(
        (p.sessions, p.current, p.pending, p.vectors),
        (3, 2, 1, 2),
        "{p:?}"
    );
    build(&home, &config, &f.runtime(), Deadline::NONE).unwrap();
    assert_eq!(embedded(&f).last().unwrap(), &["the car"]);
}

#[test]
fn a_long_session_is_embedded_in_pieces_spread_over_it() {
    let said = (0..40)
        .map(|i| format!("part{i} {}", "word ".repeat(100)))
        .collect::<Vec<_>>()
        .join("\n");
    let (_d, home, config) = home(&[s("a", "2026-11-01T09:00:00Z", &said)]);
    let f = fake();
    build(&home, &config, &f.runtime(), Deadline::NONE).unwrap();
    let texts = &embedded(&f)[0];
    assert_eq!(texts.len(), MAX_CHUNKS, "{}", texts.len());
    assert!(texts.iter().all(|t| t.len() <= CHUNK && !t.is_empty()));
    assert!(texts[0].starts_with("part0 "), "{}", &texts[0][..20]);
    assert!(texts.last().unwrap().contains("part3"), "spread to the end");
}

#[test]
fn a_query_that_holds_a_secret_is_never_embedded() {
    let password = ["synthetic", "-vec-", "pass-5531"].concat();
    let mut a = s("a", "2026-11-01T09:00:00Z", "the car");
    a.command = format!("export DB_PASSWORD={password}");
    let (_d, home, _) = built(&[a]);
    let f = fake();
    for q in [
        format!("car {}", fake_key()),
        password.clone(),
        format!("car DB_PASSWORD={password}"),
    ] {
        let h = owner_search(&home, &f.runtime(), &q);
        assert_eq!(
            h.meaning,
            Meaning::Skipped("the query holds a secret, which is never embedded".into()),
            "{q}"
        );
    }
    assert_eq!(embedded(&f), Vec::<Vec<String>>::new());
}

#[test]
fn with_no_embedding_model_or_no_runtime_the_step_is_skipped_and_status_says_why() {
    let (_d, home, config) = home(&[s("a", "2026-11-01T09:00:00Z", "the car")]);
    let mut unset = config.clone();
    unset.steward.embedding_model = None;
    let r = build(&home, &unset, &fake().runtime(), Deadline::NONE).unwrap();
    assert_eq!(r.skipped.as_deref(), Some("no embedding_model is set"));
    let r = build(
        &home,
        &config,
        &FakeRuntime::stopped().runtime(),
        Deadline::NONE,
    )
    .unwrap();
    assert_eq!(
        r.skipped.as_deref(),
        Some("the local runtime is not running")
    );
    assert!(!home.vectors_path().exists());

    // Status takes the reason from the last tick.
    crate::tick::tick_with(
        &home,
        &config,
        &FakeRuntime::stopped()
            .with_model(EMBED, Stored::Here)
            .runtime(),
    )
    .unwrap();
    let p = crate::status::status(&home).unwrap().vectors;
    assert_eq!(
        p.skipped.as_deref(),
        Some("the local runtime is not running")
    );
    assert_eq!((p.sessions, p.current, p.pending), (1, 0, 1), "{p:?}");
    crate::tick::tick_with(&home, &config, &fake().runtime()).unwrap();
    let p = crate::status::status(&home).unwrap().vectors;
    assert_eq!(p.skipped, None);
    assert_eq!((p.current, p.pending, p.vectors), (1, 0, 1), "{p:?}");
    assert!(p.bytes > 0);
    assert_eq!(p.model.as_deref(), Some(EMBED));
}

#[test]
fn a_passed_deadline_embeds_nothing_and_logs_no_session_as_sent() {
    let (_d, home, config) = home(&[s("a", "2026-11-01T10:00:00Z", "rename the widget")]);
    let f = fake();
    let r = build(
        &home,
        &config,
        &f.runtime(),
        Deadline::after(std::time::Duration::ZERO),
    )
    .unwrap();
    assert_eq!((r.pending, r.embedded, r.failed), (1, 0, 0), "{r:?}");
    assert!(r.stopped_at_deadline, "{r:?}");
    assert!(r.summary().ends_with(", stopped at the tick's time limit"));
    assert_eq!(f.calls(), []);
    let embedder = Reader::model(Some(crate::trust::OLLAMA), Some(EMBED));
    let sent: u64 = crate::readlog::list(&home, None, usize::MAX)
        .unwrap()
        .into_iter()
        .filter(|e| e.reader == embedder)
        .map(|e| match e.result {
            crate::readlog::ReadResult::Indexed { sessions, .. } => sessions,
            other => panic!("{other:?}"),
        })
        .sum();
    assert_eq!(sent, 0);
}
