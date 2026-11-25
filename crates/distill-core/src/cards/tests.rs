use super::*;
use crate::archive::Archive;
use crate::harness::Harness;
use crate::read::NOTICE_START;
use crate::runtime::{Call, FakeRuntime, Stored};
use crate::tick::Deadline;

const MODEL: &str = "small:8b";

/// A Claude Code session `id` that opens with `said`, started at `at`.
fn session(id: &str, at: &str, said: &str) -> String {
    let said = serde_json::to_string(said).unwrap();
    r#"{"type":"user","cwd":"/work/app","timestamp":"AT","message":{"role":"user","content":SAID}}
{"type":"assistant","timestamp":"AT","message":{"role":"assistant","model":"m","content":[{"type":"text","text":"done with ID"}]}}
"#
    .replace("SAID", &said)
    .replace("AT", at)
    .replace("ID", id)
}

/// A home holding `sessions` (id, start time, what the owner said), parsed and with a config
/// whose `card_model` is [`MODEL`].
fn home_with(sessions: &[(&str, &str, &str)]) -> (tempfile::TempDir, Home, Config) {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(dir.path().join("home"));
    home.ensure().unwrap();
    let mut a = Archive::open(&home).unwrap();
    for (id, at, said) in sessions {
        a.put(
            Harness::ClaudeCode,
            &format!("/s/proj/{id}.jsonl"),
            "1",
            session(id, at, said).as_bytes(),
        )
        .unwrap();
    }
    drop(a);
    crate::normalize::normalize(&home, false).unwrap();
    // No harnesses, so that a tick in a test sweeps nothing of this machine.
    let mut config = Config {
        harnesses: vec![],
        ..Default::default()
    };
    config.steward.card_model = Some(MODEL.into());
    config.save(&home.config_path()).unwrap();
    (dir, home, config)
}

fn reply(title: &str) -> String {
    serde_json::json!({
        "title": title,
        "summary": "The owner asked for a module rename. The agent moved the file and updated imports.",
        "tags": ["Rename", "rust"],
        "outcome": "done",
    })
    .to_string()
}

fn model(reply: impl Fn(&str, &str) -> String + Send + Sync + 'static) -> FakeRuntime {
    FakeRuntime::running()
        .with_model(MODEL, Stored::Here)
        .replying(reply)
}

fn facts(home: &Home, id: &str) -> SessionMeta {
    crate::normalize::find(home, Harness::ClaudeCode, id)
        .unwrap()
        .unwrap()
}

#[test]
fn the_steward_writes_a_card_from_the_session_as_a_local_model_reads_it() {
    let (_d, home, config) =
        home_with(&[("s1", "2026-11-10T10:00:00Z", "rename the widget module")]);
    let fake = model(|_, _| reply("Rename the widget module"));

    let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    assert_eq!(
        (r.written, r.failed, r.skipped.as_deref()),
        (1, 0, None),
        "{r:?}"
    );

    let card = load(&home, &facts(&home, "s1")).unwrap();
    assert_eq!(card.title, "Rename the widget module");
    assert!(card.summary.starts_with("The owner asked"), "{card:?}");
    assert_eq!(card.tags, ["rename", "rust"]);
    assert_eq!(card.outcome, Outcome::Done);
    assert_eq!(card.model, MODEL);
    assert_eq!(card.prompt_version, PROMPT_VERSION);

    // The model got the prompt and the session as the read path serves a model.
    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    let Call::Generate { model, prompt } = &calls[0] else {
        panic!("{calls:?}")
    };
    assert_eq!(model, MODEL);
    assert!(prompt.starts_with(PROMPT), "{prompt}");
    assert!(prompt.contains(NOTICE_START), "{prompt}");
    assert!(prompt.contains("rename the widget module"), "{prompt}");

    // The read is logged as the model at the local provider.
    let log = crate::readlog::list(&home, None, usize::MAX).unwrap();
    let line = serde_json::to_value(&log[0]).unwrap();
    assert_eq!(line["provider"], "ollama", "{line}");
    assert_eq!(line["model"], MODEL, "{line}");
    assert_eq!(line["session"], "claude-code/s1", "{line}");
    assert_eq!(line["result"], "served", "{line}");
}

/// Change `id`'s stored facts with `edit`, and rebuild the session list from them.
fn edit_facts(home: &Home, id: &str, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = home.meta_path(Harness::ClaudeCode, id);
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut v);
    std::fs::write(&path, v.to_string()).unwrap();
    crate::normalize::rebuild_session_list(home).unwrap();
}

/// Which sessions have a card now.
fn carded(home: &Home, ids: &[&str]) -> Vec<String> {
    ids.iter()
        .filter(|id| load(home, &facts(home, id)).is_some())
        .map(|id| id.to_string())
        .collect()
}

#[test]
fn cards_are_written_newest_first_up_to_sessions_per_run_and_never_for_mirrors_or_the_old_steward()
{
    let (_d, home, mut config) = home_with(&[
        ("old", "2026-11-01T10:00:00Z", "the oldest"),
        ("mid", "2026-11-05T10:00:00Z", "the middle one"),
        ("new", "2026-11-09T10:00:00Z", "the newest"),
        ("mirror", "2026-11-09T10:00:00Z", "a mirror of the newest"),
        (
            "steward",
            "2026-11-13T10:00:00Z",
            "the previous steward's own",
        ),
    ]);
    edit_facts(&home, "mirror", |v| {
        v["mirror"] = true.into();
        v["mirror_of"] = serde_json::json!(["claude-code"]);
        v["mirror_refs"] = serde_json::json!(["claude-code/new"]);
    });
    edit_facts(&home, "steward", |v| v["steward"] = true.into());
    config.steward.sessions_per_run = 2;
    let fake = model(|_, _| reply("A session"));
    let all = ["old", "mid", "new", "mirror", "steward"];

    let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.sessions, r.pending, r.written), (3, 3, 2), "{r:?}");
    assert_eq!(carded(&home, &all), ["mid", "new"]);

    let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.current, r.pending, r.written), (2, 1, 1), "{r:?}");
    assert_eq!(carded(&home, &all), ["old", "mid", "new"]);

    let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.current, r.pending, r.written), (3, 0, 0), "{r:?}");
    assert_eq!(fake.calls().len(), 3, "a current card is not written again");
}

#[test]
fn an_answer_that_is_not_a_usable_card_is_a_failure_for_that_session_and_is_retried_later() {
    let (_d, home, config) = home_with(&[
        ("a", "2026-11-01T10:00:00Z", "first"),
        ("b", "2026-11-02T10:00:00Z", "second"),
    ]);
    // The newest session gets an answer that is no card; the other a good one.
    let bad = model(|_, prompt| {
        if prompt.contains("second") {
            "Sure! Here is the card: {\"title\": \"x\"}".into()
        } else {
            reply("First")
        }
    });
    let r = write(&home, &config, &bad.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.written, r.failed), (1, 1), "{r:?}");
    assert!(
        r.errors[0].starts_with("claude-code/b: the model's answer is not a usable card"),
        "{r:?}"
    );
    assert!(
        !r.errors[0].contains("Sure!"),
        "no answer text in the run log: {r:?}"
    );
    assert_eq!(carded(&home, &["a", "b"]), ["a"]);
    assert_eq!(progress(&home).unwrap().failing, 1);

    // Retried at the next run, after sessions never tried.
    let good = model(|_, _| reply("Second"));
    let r = write(&home, &config, &good.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.written, r.failed), (1, 0), "{r:?}");
    assert_eq!(load(&home, &facts(&home, "b")).unwrap().title, "Second");
    assert_eq!(progress(&home).unwrap().failing, 0);
}

#[test]
fn a_reply_is_parsed_strictly() {
    let good = r#"{"title":"T","summary":"S.","tags":["A"," b ","a","Rust Lang","rust_lang"],"outcome":"partly_done"}"#;
    let r = parse_reply(good).unwrap();
    assert_eq!((r.title.as_str(), r.outcome), ("T", Outcome::PartlyDone));
    assert_eq!(
        r.tags,
        ["a", "b", "rust lang", "rust_lang"],
        "stored as written, lowercase"
    );
    assert_eq!(parse_reply(&format!("```json\n{good}\n```")).unwrap(), r);
    for bad in [
        "",
        "no json here",
        r#"["A title","A summary.",["rust"],"done"]"#,
        &format!("Here it is: {good}"),
        &format!("{good} and more"),
        r#"{"title":"T","summary":"S.","tags":[],"outcome":"success"}"#,
        r#"{"title":"T","summary":"S.","tags":[],"outcome":"done","quality":5}"#,
        r#"{"title":"T","summary":"S.","outcome":"done"}"#,
        r#"{"title":"","summary":"S.","tags":[],"outcome":"done"}"#,
        r#"{"title":"two\nlines","summary":"S.","tags":[],"outcome":"done"}"#,
        r#"{"title":"T","summary":"  ","tags":[],"outcome":"done"}"#,
        r#"{"title":"T","summary":"S.","tags":["a","b","c","d","e","f","g","h","i"],"outcome":"done"}"#,
        r#"{"title":"T","summary":"S.","tags":[""],"outcome":"done"}"#,
        "```json\n{\"title\":\"T\"",
    ] {
        assert!(parse_reply(bad).is_err(), "{bad:?} parsed");
    }
    assert!(parse_reply(&serde_json::json!({"title": "x".repeat(101), "summary": "S.", "tags": [], "outcome": "done"}).to_string()).is_err());
}

#[test]
fn a_card_goes_out_of_date_when_its_events_or_secret_findings_change_and_is_written_again() {
    let (_d, home, config) =
        home_with(&[("s1", "2026-11-10T10:00:00Z", "rename the widget module")]);
    let fake = model(|_, _| reply("Rename"));
    write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    let f = facts(&home, "s1");
    let scan = crate::findings::load(&home, &f).unwrap();
    assert!(current(&home, &f, &scan).is_some());

    // New events: another capture of the same session.
    let mut a = Archive::open(&home).unwrap();
    let longer = session("s1", "2026-11-10T10:00:00Z", "rename the widget module")
        + r#"{"type":"user","timestamp":"2026-11-10T10:01:00Z","message":{"role":"user","content":"and the tests"}}
"#;
    a.put(
        Harness::ClaudeCode,
        "/s/proj/s1.jsonl",
        "2",
        longer.as_bytes(),
    )
    .unwrap();
    drop(a);
    crate::normalize::normalize(&home, false).unwrap();
    crate::findings::scan_all(&home, &config).unwrap();
    let f = facts(&home, "s1");
    let scan = crate::findings::load(&home, &f).unwrap();
    assert!(load(&home, &f).is_some(), "the old card is still on disk");
    assert!(current(&home, &f, &scan).is_none(), "but it is not current");
    assert_eq!(progress(&home).unwrap().pending, 1);
    let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    assert_eq!(r.written, 1, "{r:?}");
    let scan = crate::findings::load(&home, &f).unwrap();
    assert!(current(&home, &f, &scan).is_some());

    // Different secret findings: the placeholders the card was built from changed.
    let path = home.findings_path(Harness::ClaudeCode, "s1");
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let fp = "ab".repeat(32);
    v["findings"] = serde_json::json!([{"event": 0, "field": "text", "start": 0, "end": 6,
        "kind": "anthropic_key", "fingerprint": fp}]);
    std::fs::write(&path, v.to_string()).unwrap();
    let scan = crate::findings::load(&home, &f).unwrap();
    assert!(current(&home, &f, &scan).is_none());
}

#[test]
fn a_card_from_the_previous_build_counts_as_absent() {
    let (_d, home, config) =
        home_with(&[("s1", "2026-11-10T10:00:00Z", "rename the widget module")]);
    let path = home.card_path(Harness::ClaudeCode, "s1");
    std::fs::write(
        &path,
        r#"{"task":"Rename","outcome":"success","quality":4,"tags":[],"summary":"Old.",
            "produced_by":{"harness":"command","prompt_version":2,"written_at":"2026-01-01T00:00:00Z"}}"#,
    )
    .unwrap();
    let f = facts(&home, "s1");
    assert_eq!(load(&home, &f), None);
    let r = write(
        &home,
        &config,
        &model(|_, _| reply("New")).runtime(),
        Deadline::NONE,
    )
    .unwrap();
    assert_eq!((r.pending, r.written), (1, 1), "{r:?}");
    assert_eq!(load(&home, &f).unwrap().title, "New");
}

const NEW_MODEL: &str = "better:14b";

/// A runtime with both models stored here, whose answer names the model that wrote it, and that
/// fails for every session whose text holds `failing`.
fn both_models(failing: &'static str) -> FakeRuntime {
    FakeRuntime::running()
        .with_model(MODEL, Stored::Here)
        .with_model(NEW_MODEL, Stored::Here)
        .replying(move |model, prompt| {
            if prompt.contains(failing) {
                "no card".into()
            } else {
                reply(&format!("By {model}"))
            }
        })
}

/// Each session's card, as (id, model) for those that have one, and whether it is shown.
fn cards_by_model(home: &Home, config: &Config, ids: &[&str]) -> Vec<(String, String, bool)> {
    let key = FingerprintKey::existing(home).unwrap();
    ids.iter()
        .filter_map(|id| {
            let f = facts(home, id);
            let shown = stored_current(home, config, key.as_ref(), &f).is_some();
            load(home, &f).map(|c| (id.to_string(), c.model, shown))
        })
        .collect()
}

#[test]
fn a_new_card_model_makes_cards_stale_which_stay_shown_and_are_rewritten_after_every_missing_one() {
    let (_d, home, mut config) = home_with(&[
        ("old1", "2026-11-01T10:00:00Z", "the oldest"),
        ("old2", "2026-11-02T10:00:00Z", "the older"),
        ("new1", "2026-11-08T10:00:00Z", "the newer"),
        ("new2", "2026-11-09T10:00:00Z", "the newest"),
    ]);
    let ids = ["old1", "old2", "new1", "new2"];
    config.steward.sessions_per_run = 2;
    let fake = both_models("never fails");
    write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    let shown = |id: &str, model: &str| (id.to_string(), model.to_string(), true);
    assert_eq!(
        cards_by_model(&home, &config, &ids),
        [shown("new1", MODEL), shown("new2", MODEL)]
    );

    // A new card_model: the two cards are stale, and still shown, in a list too.
    config.steward.card_model = Some(NEW_MODEL.into());
    config.save(&home.config_path()).unwrap();
    let card = load(&home, &facts(&home, "new2")).unwrap();
    assert!(card.stale(NEW_MODEL) && !card.stale(MODEL));
    let p = progress(&home).unwrap();
    assert_eq!(
        (p.sessions, p.current, p.stale, p.pending, p.failing),
        (4, 0, 2, 2, 0),
        "{p:?}"
    );
    let rt = FakeRuntime::not_installed().runtime();
    let crate::read::Listed::Rows(rows) = crate::read::list(
        &home,
        &rt,
        &crate::trust::Reader::Owner,
        &Default::default(),
        Default::default(),
    )
    .unwrap() else {
        panic!("the owner gets rows")
    };
    let listed: Vec<_> = rows
        .rows
        .iter()
        .map(|r| r.card.as_ref().map(|c| c.card.model.as_str()))
        .collect();
    assert_eq!(listed, [Some(MODEL), Some(MODEL), None, None]);

    // The missing cards go first, though their sessions are older; old1's attempt fails.
    let fake = both_models("the oldest");
    let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    assert_eq!(
        (
            r.current,
            r.pending,
            r.stale,
            r.written,
            r.rewritten,
            r.failed
        ),
        (0, 2, 2, 1, 0, 1),
        "{r:?}"
    );
    assert_eq!(
        r.summary(),
        "wrote 1 card (0 already current, 1 waiting, 2 stale), 1 to retry"
    );
    assert_eq!(
        cards_by_model(&home, &config, &ids),
        [
            shown("old2", NEW_MODEL),
            shown("new1", MODEL),
            shown("new2", MODEL)
        ]
    );

    // A failed session with no card is retried before any stale card is rewritten.
    config.steward.sessions_per_run = 1;
    let fake = both_models("never fails");
    let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.written, r.rewritten), (1, 0), "{r:?}");
    assert_eq!(load(&home, &facts(&home, "old1")).unwrap().model, NEW_MODEL);

    // Then the stale cards, newest session first.
    let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    assert_eq!(
        (r.current, r.stale, r.written, r.rewritten),
        (2, 2, 1, 1),
        "{r:?}"
    );
    assert_eq!(
        r.summary(),
        "wrote 1 card (2 already current, 0 waiting, 1 stale)"
    );
    assert_eq!(
        cards_by_model(&home, &config, &ids),
        [
            shown("old1", NEW_MODEL),
            shown("old2", NEW_MODEL),
            shown("new1", MODEL),
            shown("new2", NEW_MODEL)
        ]
    );
    let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    assert_eq!(r.rewritten, 1, "{r:?}");
    let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.current, r.stale, r.written), (4, 0, 0), "{r:?}");
    let p = progress(&home).unwrap();
    assert_eq!((p.current, p.stale, p.pending), (4, 0, 0), "{p:?}");
}

#[test]
fn a_card_written_with_an_older_prompt_version_is_stale_shown_and_rewritten() {
    let (_d, home, config) =
        home_with(&[("s1", "2026-11-10T10:00:00Z", "rename the widget module")]);
    let fake = model(|_, _| reply("Rename"));
    write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    let f = facts(&home, "s1");
    // The card as the previous prompt wrote it, scanned as it is.
    let mut card = load(&home, &f).unwrap();
    card.prompt_version = PROMPT_VERSION - 1;
    assert!(card.stale(MODEL));
    std::fs::write(
        home.card_path(Harness::ClaudeCode, "s1"),
        serde_json::to_string(&card).unwrap(),
    )
    .unwrap();
    let fpath = home.findings_path(Harness::ClaudeCode, "s1");
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fpath).unwrap()).unwrap();
    v["card"]["hash"] = card.hash().into();
    std::fs::write(&fpath, v.to_string()).unwrap();

    let scan = crate::findings::load(&home, &f).unwrap();
    assert_eq!(
        current(&home, &f, &scan),
        Some(card),
        "a stale card is shown"
    );
    let p = progress(&home).unwrap();
    assert_eq!((p.current, p.stale, p.pending), (0, 1, 0), "{p:?}");
    let s = crate::status::status(&home).unwrap();
    assert_eq!((s.cards.stale, s.cards.pending), (1, 0));

    let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.stale, r.written, r.rewritten), (1, 1, 1), "{r:?}");
    assert_eq!(load(&home, &f).unwrap().prompt_version, PROMPT_VERSION);
    let p = progress(&home).unwrap();
    assert_eq!((p.current, p.stale), (1, 0), "{p:?}");
}

/// A fake key, built at run time so no secret scanner mistakes it for a leak.
fn fake_key() -> String {
    ["sk-", "ant-", "api03-", &"Qw8".repeat(9)].concat()
}

/// Every file under `dir` that holds `value`, relative to it.
fn holding(dir: &std::path::Path, value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().filter_map(Result::ok) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if String::from_utf8_lossy(&std::fs::read(&p).unwrap()).contains(value) {
                out.push(p.strip_prefix(dir).unwrap().display().to_string());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_card_whose_text_holds_a_secret_is_discarded_and_written_again() {
    let key = fake_key();
    let (_d, home, config) = home_with(&[(
        "s1",
        "2026-11-10T10:00:00Z",
        &format!("deploy with {key} now"),
    )]);
    // The model saw a placeholder, yet its answer holds the value (a test of the guard).
    let echoed = key.clone();
    let leaky = model(move |_, prompt| {
        assert!(!prompt.contains(&echoed), "the model was served the value");
        reply(&format!("Deploy with {echoed}"))
    });
    let r = write(&home, &config, &leaky.runtime(), Deadline::NONE).unwrap();
    assert_eq!((r.written, r.failed, r.discarded), (0, 1, 1), "{r:?}");
    assert!(r.errors[0].contains("held a secret"), "{r:?}");
    assert!(!format!("{r:?}").contains(&key));
    assert!(!home.card_path(Harness::ClaudeCode, "s1").exists());
    // Only the raw archive and the session's events hold the value: in the session's folder,
    // neither its card nor its findings do.
    let held = holding(&home.root, &key);
    let in_session = |p: &String| {
        p.starts_with("sessions/claude-code/s1/")
            && !p.ends_with("/card.json")
            && !p.ends_with("/findings.json")
            && !p.ends_with("/meta.json")
    };
    assert!(held.iter().any(in_session), "{held:?}");
    assert!(
        held.iter().all(|p| p.starts_with("raw/") || in_session(p)),
        "{held:?}"
    );

    // Rewritten at the next run.
    let r = write(
        &home,
        &config,
        &model(|_, _| reply("Deploy")).runtime(),
        Deadline::NONE,
    )
    .unwrap();
    assert_eq!(r.written, 1, "{r:?}");
}

#[test]
fn card_findings_are_stored_with_the_cards_field_names_as_their_place() {
    let (_d, home, config) = home_with(&[("s1", "2026-11-10T10:00:00Z", "write to the team")]);
    let fake = model(|_, _| {
        serde_json::json!({
            "title": "Mail the team",
            "summary": "Drafted a note to jane@example.com about the launch.",
            "tags": ["mail", "ops@example.org"],
            "outcome": "unknown",
        })
        .to_string()
    });
    write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    let f = facts(&home, "s1");
    let scan = crate::findings::load(&home, &f).unwrap();
    let card = current(&home, &f, &scan).expect("a card with personal details is kept");
    let found = &scan.card.as_ref().unwrap().findings;
    let places: Vec<_> = found
        .iter()
        .map(|x| (x.field.as_str(), x.kind, x.start, x.end))
        .collect();
    use crate::detect::Kind;
    let at = card.summary.find("jane@").unwrap();
    assert_eq!(
        places,
        [
            ("summary", Kind::Email, at, at + "jane@example.com".len()),
            ("tags/1", Kind::Email, 0, "ops@example.org".len()),
        ]
    );
    let file = std::fs::read_to_string(home.findings_path(Harness::ClaudeCode, "s1")).unwrap();
    assert!(
        !file.contains("jane@example.com"),
        "a finding never holds its value"
    );
}

#[test]
fn a_stored_card_found_to_hold_a_secret_at_a_later_scan_is_discarded_and_written_again() {
    let (_d, home, config) =
        home_with(&[("s1", "2026-11-10T10:00:00Z", "rename the widget module")]);
    write(
        &home,
        &config,
        &model(|_, _| reply("Rename")).runtime(),
        Deadline::NONE,
    )
    .unwrap();
    let f = facts(&home, "s1");
    // A card written before the detector knew this kind of secret.
    let path = home.card_path(Harness::ClaudeCode, "s1");
    let mut card = load(&home, &f).unwrap();
    card.summary = format!("Used {} to deploy.", fake_key());
    std::fs::write(&path, serde_json::to_string(&card).unwrap()).unwrap();
    // A new detector: the findings are out of date, and the scan runs again.
    let fpath = home.findings_path(Harness::ClaudeCode, "s1");
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fpath).unwrap()).unwrap();
    v["made_from"]["detector"] = 0.into();
    v["card"]["hash"] = card.hash().into();
    v["card"]["findings"] = serde_json::json!([]);
    std::fs::write(&fpath, v.to_string()).unwrap();

    crate::findings::scan_all(&home, &config).unwrap();
    assert!(!path.exists(), "the card is discarded");
    assert!(holding(&home.root, &fake_key()).is_empty());
    let r = write(
        &home,
        &config,
        &model(|_, _| reply("Rename again")).runtime(),
        Deadline::NONE,
    )
    .unwrap();
    assert_eq!(r.written, 1, "{r:?}");
    assert_eq!(load(&home, &f).unwrap().title, "Rename again");
}

#[test]
fn with_no_card_model_or_no_local_runtime_that_can_run_it_the_step_is_skipped() {
    let (_d, home, mut config) = home_with(&[("s1", "2026-11-10T10:00:00Z", "rename it")]);
    let cases = [
        (
            FakeRuntime::not_installed(),
            "no local runtime is installed",
        ),
        (
            FakeRuntime::stopped().with_model(MODEL, Stored::Here),
            "the local runtime is not running",
        ),
        (
            FakeRuntime::running().with_model(MODEL, Stored::Cloud),
            "card_model \"small:8b\" is not stored on this machine",
        ),
    ];
    for (fake, why) in cases {
        let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
        assert_eq!(r.skipped.as_deref(), Some(why), "{r:?}");
        assert_eq!(r.summary(), format!("cards skipped: {why}"));
        assert_eq!(fake.calls(), []);
    }
    config.steward.card_model = None;
    let fake = model(|_, _| reply("never"));
    let r = write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    assert_eq!(r.skipped.as_deref(), Some("no card_model is set"));
    assert_eq!(fake.calls(), []);
    assert!(
        crate::readlog::list(&home, None, usize::MAX)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn status_shows_card_progress_and_why_the_step_is_skipped() {
    let (_d, home, mut config) = home_with(&[
        ("a", "2026-11-01T10:00:00Z", "first"),
        ("b", "2026-11-02T10:00:00Z", "second"),
    ]);
    // Set, but the last tick found no runtime running.
    crate::tick::tick_with(&home, &config, &FakeRuntime::stopped().runtime()).unwrap();
    let s = crate::status::status(&home).unwrap();
    assert_eq!(s.cards.model.as_deref(), Some(MODEL));
    assert_eq!(
        s.cards.skipped.as_deref(),
        Some("the local runtime is not running")
    );
    assert_eq!(
        (s.cards.sessions, s.cards.current, s.cards.pending),
        (2, 0, 2)
    );

    config.steward.sessions_per_run = 1;
    crate::tick::tick_with(&home, &config, &model(|_, _| reply("One")).runtime()).unwrap();
    let s = crate::status::status(&home).unwrap();
    assert_eq!(s.cards.skipped, None);
    assert_eq!(
        (s.cards.current, s.cards.pending, s.cards.failing),
        (1, 1, 0)
    );

    config.steward.card_model = None;
    config.save(&home.config_path()).unwrap();
    let s = crate::status::status(&home).unwrap();
    assert_eq!(s.cards.skipped.as_deref(), Some("no card_model is set"));
}

fn reply_with(title: &str, summary: &str, tags: &[&str], outcome: &str) -> String {
    serde_json::json!({"title": title, "summary": summary, "tags": tags, "outcome": outcome})
        .to_string()
}

/// Two sessions with cards, indexed: `a` renamed something and is done, `b` failed a deploy.
fn carded_home() -> (tempfile::TempDir, Home, Config) {
    let (d, home, config) = home_with(&[
        ("a", "2026-11-01T10:00:00Z", "rename the widget module"),
        ("b", "2026-11-02T10:00:00Z", "ship the gadget"),
    ]);
    let fake = model(|_, prompt| {
        if prompt.contains("widget") {
            reply_with(
                "Rename the widget",
                "Moved widget.rs to gadget.rs everywhere.",
                &["refactor", "rust"],
                "done",
            )
        } else {
            reply_with(
                "Ship the gadget",
                "The deploy to staging timed out twice.",
                &["deploy"],
                "failed",
            )
        }
    });
    write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    crate::index::build(&home).unwrap();
    (d, home, config)
}

fn owner_hits(home: &Home, query: &str, filter: &crate::read::Filter) -> Vec<crate::read::Hit> {
    let rt = FakeRuntime::not_installed().runtime();
    match crate::read::search(
        home,
        &rt,
        &crate::trust::Reader::Owner,
        query,
        filter,
        Default::default(),
    )
    .unwrap()
    {
        crate::read::Searched::Hits(h) => h.hits,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_cards_title_summary_and_tags_are_in_the_word_index_and_shown_in_search_results() {
    let (_d, home, _) = carded_home();
    let none = crate::read::Filter::default();
    // Words only the cards hold.
    for (q, want) in [
        ("staging", "b"),
        ("refactor", "a"),
        ("everywhere", "a"),
        ("ship", "b"),
    ] {
        let hits = owner_hits(&home, q, &none);
        let keys: Vec<_> = hits.iter().map(|h| h.facts.session_id.as_str()).collect();
        assert_eq!(keys, [want], "{q}");
    }
    let hit = &owner_hits(&home, "staging", &none)[0];
    let shown = hit.card.as_ref().expect("the hit shows its card");
    assert_eq!(shown.card.title, "Ship the gadget");
    assert_eq!(shown.card.summary, "The deploy to staging timed out twice.");

    // A model reads the card's title and summary in place of the opening.
    let rt = model(|_, _| String::new()).runtime();
    let reader = crate::trust::Reader::model(Some("ollama"), Some(MODEL));
    let crate::read::Searched::Text(t) =
        crate::read::search(&home, &rt, &reader, "staging", &none, Default::default()).unwrap()
    else {
        panic!("a model gets text")
    };
    assert!(
        t.contains("  Card: Ship the gadget · outcome failed · tags deploy\n"),
        "{t}"
    );
    assert!(
        t.contains("  Summary: The deploy to staging timed out twice.\n"),
        "{t}"
    );
    assert!(!t.contains("Opened with"), "{t}");
}

#[test]
fn search_and_list_filter_by_tag_and_outcome() {
    let (_d, home, _) = carded_home();
    let by = |tags: Option<&str>, outcome: Option<Outcome>| crate::read::Filter {
        tags: tags
            .map(|t| t.split(',').map(str::to_string).collect())
            .unwrap_or_default(),
        outcome,
        ..Default::default()
    };
    let ids = |hits: Vec<crate::read::Hit>| -> Vec<String> {
        hits.into_iter().map(|h| h.facts.session_id).collect()
    };
    // "gadget" is in both sessions.
    assert_eq!(ids(owner_hits(&home, "gadget", &by(None, None))).len(), 2);
    assert_eq!(
        ids(owner_hits(&home, "gadget", &by(Some("DEPLOY"), None))),
        ["b"]
    );
    assert_eq!(
        ids(owner_hits(&home, "gadget", &by(None, Some(Outcome::Done)))),
        ["a"]
    );
    assert!(owner_hits(&home, "gadget", &by(Some("rust"), Some(Outcome::Failed))).is_empty());

    let rt = FakeRuntime::not_installed().runtime();
    let list = |f| match crate::read::list(
        &home,
        &rt,
        &crate::trust::Reader::Owner,
        &f,
        Default::default(),
    )
    .unwrap()
    {
        crate::read::Listed::Rows(r) => r.rows,
        other => panic!("{other:?}"),
    };
    let rows = list(by(None, None));
    let titles: Vec<_> = rows
        .iter()
        .map(|r| r.card.as_ref().map(|c| c.card.title.clone()))
        .collect();
    assert_eq!(
        titles,
        [
            Some("Ship the gadget".into()),
            Some("Rename the widget".into())
        ]
    );
    let rows = list(by(Some("refactor"), None));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].facts.session_id, "a");

    // Several tags mean all of them (#159): `a` has both of these, and no session has both
    // `rust` and `deploy`.
    let ids_of = |rows: Vec<crate::read::Row>| -> Vec<String> {
        rows.into_iter().map(|r| r.facts.session_id).collect()
    };
    assert_eq!(ids_of(list(by(Some("rust,REFACTOR"), None))), ["a"]);
    assert!(list(by(Some("rust,deploy"), None)).is_empty());
    assert!(owner_hits(&home, "gadget", &by(Some("rust,deploy"), None)).is_empty());
    assert_eq!(
        ids(owner_hits(
            &home,
            "gadget",
            &by(Some("refactor,rust"), None)
        )),
        ["a"]
    );
    let counted = match crate::read::tags(
        &home,
        &rt,
        &crate::trust::Reader::Owner,
        &by(Some("rust,refactor"), None),
    )
    .unwrap()
    {
        crate::read::Tagged::Counts(t) => t.into_iter().map(|c| c.tag).collect::<Vec<_>>(),
        other => panic!("{other:?}"),
    };
    assert_eq!(counted, ["refactor", "rust"]);
}

#[test]
fn the_owners_aliases_merge_tags_where_they_are_shown_and_filtered() {
    let (_d, home, mut config) = carded_home();
    let rt = FakeRuntime::not_installed().runtime();
    let rows = |tag: &str| match crate::read::list(
        &home,
        &rt,
        &crate::trust::Reader::Owner,
        &crate::read::Filter {
            tags: vec![tag.into()],
            ..Default::default()
        },
        Default::default(),
    )
    .unwrap()
    {
        crate::read::Listed::Rows(r) => r.rows,
        other => panic!("{other:?}"),
    };
    let ids = |rows: Vec<crate::read::Row>| -> Vec<String> {
        rows.into_iter().map(|r| r.facts.session_id).collect()
    };
    assert_eq!(ids(rows("rust")), ["a"]);

    // Aliases change no card: "refactor" and "deploy" are now shown and filtered as "rust".
    config.tags.aliases = [
        ("Refactor".into(), "rust".into()),
        ("deploy".into(), "Rust".into()),
    ]
    .into();
    config.save(&home.config_path()).unwrap();
    assert_eq!(ids(rows("rust")), ["b", "a"]);
    assert_eq!(
        ids(rows("REFACTOR")),
        ["b", "a"],
        "the query goes through them too"
    );
    let a = rows("rust")
        .into_iter()
        .find(|r| r.facts.session_id == "a")
        .unwrap();
    assert_eq!(a.card.unwrap().card.tags, ["rust"], "each tag once");
    assert_eq!(
        load(&home, &facts(&home, "a")).unwrap().tags,
        ["refactor", "rust"],
        "the card keeps what its model wrote"
    );
}

#[test]
fn the_owners_tag_edits_win_over_the_model_and_each_undoes_the_other() {
    let (_d, home, config) = carded_home();
    let rt = FakeRuntime::not_installed().runtime();
    let rows = |tag: &str| match crate::read::list(
        &home,
        &rt,
        &crate::trust::Reader::Owner,
        &crate::read::Filter {
            tags: vec![tag.into()],
            ..Default::default()
        },
        Default::default(),
    )
    .unwrap()
    {
        crate::read::Listed::Rows(r) => r.rows,
        other => panic!("{other:?}"),
    };
    let ids =
        |tag: &str| -> Vec<String> { rows(tag).into_iter().map(|r| r.facts.session_id).collect() };
    let counts = || match crate::read::tags(
        &home,
        &rt,
        &crate::trust::Reader::Owner,
        &Default::default(),
    )
    .unwrap()
    {
        crate::read::Tagged::Counts(c) => c
            .into_iter()
            .map(|c| (c.tag, c.sessions))
            .collect::<Vec<_>>(),
        other => panic!("{other:?}"),
    };

    let (key, edit) =
        crate::tags::tag_session(&home, &config, "a", &["Ship It".into()], &["RUST".into()])
            .unwrap();
    assert_eq!(key, "claude-code/a");
    assert_eq!(
        (&edit.add[..], &edit.hide[..]),
        (&["ship-it".to_string()][..], &["rust".to_string()][..])
    );
    assert!(ids("rust").is_empty(), "a hidden tag is not there");
    assert_eq!(ids("ship it"), ["a"], "an added one is");
    let a = rows("ship-it").remove(0).card.unwrap();
    assert_eq!(a.card.tags, ["refactor", "ship-it"]);
    assert_eq!(
        counts(),
        [
            ("deploy".into(), 1),
            ("refactor".into(), 1),
            ("ship-it".into(), 1)
        ]
    );
    assert_eq!(
        load(&home, &facts(&home, "a")).unwrap().tags,
        ["refactor", "rust"],
        "the card keeps what its model wrote"
    );
    // The word index holds the tags as shown: the edit put a's entry out of date.
    let none = crate::read::Filter::default();
    let found = |q: &str| -> Vec<String> {
        owner_hits(&home, q, &none)
            .into_iter()
            .map(|h| h.facts.session_id)
            .collect()
    };
    assert_eq!(found("rust"), ["a"], "until the index is built again");
    assert_eq!(crate::index::progress(&home).unwrap().pending, 1);
    crate::index::build(&home).unwrap();
    assert!(found("rust").is_empty());
    assert!(found("ship-it").contains(&"a".to_string()));
    assert_eq!(
        crate::home::Home::keep("tags.json"),
        Some(crate::home::Keep::Always),
        "nothing rebuilds the owner's edits"
    );

    // Hiding an added tag takes it off and adding a hidden one shows it again; each is kept.
    let (_, edit) = crate::tags::tag_session(
        &home,
        &config,
        "claude-code/a",
        &["rust".into()],
        &["ship-it".into()],
    )
    .unwrap();
    assert_eq!(
        (&edit.add[..], &edit.hide[..]),
        (&["rust".to_string()][..], &["ship-it".to_string()][..])
    );
    assert_eq!(ids("rust"), ["a"]);
    assert!(ids("ship-it").is_empty());

    // No secret and no personal detail goes in, and only a session in the archive is tagged.
    for bad in [fake_key(), "Avery Example".to_string()] {
        let mut config = config.clone();
        config.privacy.personal_terms.push("Avery Example".into());
        let e = crate::tags::tag_session(&home, &config, "a", &[bad], &[]).unwrap_err();
        assert!(e.to_string().contains("secret or a personal detail"), "{e}");
    }
    assert!(crate::tags::tag_session(&home, &config, "nope", &["x".into()], &[]).is_err());
    let e = crate::tags::tag_session(&home, &config, "a", &["ci,cd".into()], &[]).unwrap_err();
    assert!(e.to_string().contains("comma"), "{e}");
    assert_eq!(
        crate::tags::OwnerTags::load(&home).unwrap().sessions.len(),
        1,
        "a refused edit changes nothing"
    );
    crate::index::build(&home).unwrap();
    assert_eq!(
        found("rust"),
        ["a"],
        "edits that show the card as written leave the index as it was"
    );

    // Tags are told apart through the aliases: with `js` shown as `javascript`, adding
    // `javascript` replaces hiding `js`.
    let mut config = config.clone();
    config.tags.aliases = [("js".into(), "javascript".into())].into();
    config.save(&home.config_path()).unwrap();
    std::fs::remove_file(home.owner_tags_path()).unwrap();
    crate::tags::tag_session(&home, &config, "a", &[], &["js".into()]).unwrap();
    let (_, edit) =
        crate::tags::tag_session(&home, &config, "a", &["javascript".into()], &[]).unwrap();
    assert_eq!(
        (&edit.add[..], &edit.hide[..]),
        (&["javascript".to_string()][..], &[][..])
    );
    // Should both stand, as when an alias came later, the hide wins wherever tags are read; and
    // an added tag whose alias destination, as written, holds a secret is not shown or found.
    let both = crate::tags::OwnerTags {
        sessions: [(
            "claude-code/a".into(),
            crate::tags::Edit {
                add: vec!["javascript".into(), "pal".into()],
                hide: vec!["js".into()],
            },
        )]
        .into(),
    };
    std::fs::write(home.owner_tags_path(), serde_json::to_vec(&both).unwrap()).unwrap();
    let mut aliased = config.tags.aliases.clone();
    let bearer = ["Bearer ", &"Zq9xW7".repeat(6)].concat();
    aliased.insert("pal".into(), bearer.clone());
    config.tags.aliases = aliased;
    config.save(&home.config_path()).unwrap();
    for t in ["javascript".to_string(), "pal".into(), bearer.clone()] {
        assert!(ids(&t).is_empty(), "{t}");
    }
    let a = rows("rust").remove(0).card.unwrap();
    assert_eq!(a.card.tags, ["refactor", "rust"]);
    assert!(
        counts()
            .iter()
            .all(|(t, _)| t == "deploy" || t == "refactor" || t == "rust")
    );

    // Two names for one tag in one edit count once: with `js` hidden, adding `js` and
    // `javascript` shows it again, as one tag.
    std::fs::remove_file(home.owner_tags_path()).unwrap();
    crate::tags::tag_session(&home, &config, "a", &[], &["js".into()]).unwrap();
    let (_, edit) = crate::tags::tag_session(
        &home,
        &config,
        "a",
        &["js".into(), "javascript".into()],
        &[],
    )
    .unwrap();
    assert_eq!(
        (&edit.add[..], &edit.hide[..]),
        (&["js".to_string()][..], &[][..])
    );

    // A model tag shown as written, because its alias chain goes through a secret, is not found
    // by the chain's end.
    config.tags.aliases = [
        ("refactor".into(), bearer.clone()),
        (bearer.clone(), "releasework".into()),
    ]
    .into();
    config.save(&home.config_path()).unwrap();
    assert!(ids("releasework").is_empty());
    assert_eq!(ids("refactor"), ["a"]);
    // Nor is a tag found by a name whose own chain goes through one, and tags such a chain
    // would join stay two tags to an edit.
    config.tags.aliases.insert("zzz".into(), bearer.clone());
    config.tags.aliases.insert(bearer.clone(), "deploy".into());
    config.save(&home.config_path()).unwrap();
    assert!(ids("zzz").is_empty());
    let (_, edit) =
        crate::tags::tag_session(&home, &config, "b", &[], &["zzz".into(), "deploy".into()])
            .unwrap();
    assert_eq!(edit.hide, ["zzz", "deploy"]);
    // And the owner adds the chain's end beside such a model tag.
    crate::tags::tag_session(&home, &config, "a", &["deploy".into()], &[]).unwrap();
    assert_eq!(ids("deploy"), ["a"]);

    // An edit says what the owner wants shown, whatever card it lands on: hiding an added tag
    // the model has since written too still hides it.
    crate::tags::tag_session(&home, &config, "a", &["rust".into()], &[]).unwrap();
    crate::tags::tag_session(&home, &config, "a", &[], &["rust".into()]).unwrap();
    assert!(ids("rust").is_empty());
}

#[test]
fn an_out_of_date_card_is_not_shown_and_the_index_follows_the_card() {
    let (_d, home, config) = carded_home();
    let none = crate::read::Filter::default();
    // A new card for `b`: the index entry goes out of date with it.
    let f = facts(&home, "b");
    std::fs::remove_file(home.card_path(Harness::ClaudeCode, "b")).unwrap();
    assert!(crate::index::progress(&home).unwrap().pending >= 1);
    let fake =
        model(|_, _| reply_with("Ship it again", "Retried the rollout.", &["deploy"], "done"));
    write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    crate::index::build(&home).unwrap();
    assert!(
        owner_hits(&home, "staging", &none).is_empty(),
        "the old card's words are gone"
    );
    assert_eq!(owner_hits(&home, "rollout", &none).len(), 1);

    // A card whose session changed is not shown.
    let path = home.card_path(Harness::ClaudeCode, "b");
    let mut card = load(&home, &f).unwrap();
    card.made_from.capture_hash = "0".repeat(64);
    std::fs::write(&path, serde_json::to_string(&card).unwrap()).unwrap();
    let hits = owner_hits(&home, "gadget", &none);
    let b = hits.iter().find(|h| h.facts.session_id == "b").unwrap();
    assert!(b.card.is_none(), "{b:?}");
}

#[test]
fn a_long_session_is_cut_in_the_middle_keeping_its_start_and_its_closing_notice() {
    let long = format!("start here {} end there", "word ".repeat(10_000));
    let (_d, home, config) = home_with(&[("s1", "2026-11-10T10:00:00Z", &long)]);
    let fake = model(|_, _| reply("Long"));
    write(&home, &config, &fake.runtime(), Deadline::NONE).unwrap();
    let Call::Generate { prompt, .. } = &fake.calls()[0] else {
        panic!()
    };
    let session = &prompt[PROMPT.len()..];
    assert!(session.len() < SESSION_BUDGET + 100, "{}", session.len());
    assert!(session.contains("start here"));
    assert!(session.contains(" left out …]"));
    assert!(session.trim_end().ends_with(crate::read::NOTICE_END));
}

#[test]
fn a_tag_with_a_finding_in_it_is_shown_as_written_and_its_finding_follows_it() {
    use crate::findings::CardFinding;
    let finding = |field: &str| CardFinding {
        field: field.into(),
        start: 0,
        end: 5,
        kind: crate::detect::Kind::PersonName,
        fingerprint: serde_json::from_str("\"0123456789abcdef\"").unwrap(),
    };
    let config = crate::tags::TagsConfig {
        aliases: [("js".into(), "javascript".into())].into(),
        ..Default::default()
    };
    let tags: Vec<String> = ["js", "javascript", "Alice Smith", "Rust Lang"]
        .map(String::from)
        .into();
    let (shown, found) = viewed(
        "k",
        &tags,
        &[finding("tags/2"), finding("title")],
        &config,
        |_| false,
    );
    assert_eq!(shown, ["javascript", "Alice Smith", "rust-lang"]);
    assert_eq!(
        found.iter().map(|f| f.field.as_str()).collect::<Vec<_>>(),
        ["tags/1", "title"]
    );

    // A view the detector finds something in is not shown: the stored tag is.
    let (shown, _) = viewed("k", &tags, &[], &config, |v| v == "rust-lang");
    assert_eq!(shown, ["javascript", "alice-smith", "Rust Lang"]);
    // Nor one whose alias destination, as the owner wrote it, holds one.
    let config = crate::tags::TagsConfig {
        aliases: [("js".into(), "Bearer xyz".into())].into(),
        ..Default::default()
    };
    let (shown, _) = viewed("k", &tags, &[], &config, |v| v == "Bearer xyz");
    assert_eq!(shown[0], "js");

    // The owner's edits come last: a hidden tag goes with its findings, and the places of the
    // rest follow; an added tag joins unless the detector finds something in it.
    let mut config = crate::tags::TagsConfig::default();
    config.owner.sessions.insert(
        "k".into(),
        crate::tags::Edit {
            add: vec!["ops".into(), "rust-lang".into(), "bob".into()],
            hide: vec!["JS".into(), "alice smith".into()],
        },
    );
    let (shown, found) = viewed(
        "k",
        &tags,
        &[finding("tags/2"), finding("tags/3"), finding("title")],
        &config,
        |v| v == "bob",
    );
    assert_eq!(shown, ["javascript", "Rust Lang", "ops"]);
    assert_eq!(
        found.iter().map(|f| f.field.as_str()).collect::<Vec<_>>(),
        ["tags/1", "title"]
    );
}

#[test]
fn the_tags_in_use_are_counted_through_the_view_and_the_read_is_logged() {
    use crate::read::{TagCount, Tagged};
    let (_d, home, mut config) = carded_home();
    let rt = FakeRuntime::not_installed().runtime();
    let counts = |filter: &crate::read::Filter| match crate::read::tags(
        &home,
        &rt,
        &crate::trust::Reader::Owner,
        filter,
    )
    .unwrap()
    {
        Tagged::Counts(c) => c,
        other => panic!("{other:?}"),
    };
    let t = |tag: &str, sessions| TagCount {
        tag: tag.into(),
        sessions,
    };
    let all = crate::read::Filter::default();
    assert_eq!(
        counts(&all),
        [t("deploy", 1), t("refactor", 1), t("rust", 1)]
    );

    // Through the owner's aliases: `a`'s two tags are one, counted once.
    config.tags.aliases = [
        ("refactor".into(), "rust".into()),
        ("deploy".into(), "rust".into()),
    ]
    .into();
    config.save(&home.config_path()).unwrap();
    assert_eq!(counts(&all), [t("rust", 2)]);
    let only_a = crate::read::Filter {
        outcome: Some(Outcome::Done),
        ..Default::default()
    };
    assert_eq!(counts(&only_a), [t("rust", 1)]);

    // A model gets text between the notices; each call is one read log line with the counts.
    let model = crate::trust::Reader::model(Some("ollama"), Some(MODEL));
    let model_rt = FakeRuntime::stopped()
        .with_model(MODEL, Stored::Here)
        .runtime();
    let Tagged::Text(text) = crate::read::tags(&home, &model_rt, &model, &all).unwrap() else {
        panic!("a model gets text")
    };
    assert_eq!(
        text,
        format!(
            "1 tags on 2 sessions with a current card, most used first:\n{NOTICE_START}\nrust (2)\n{}\n",
            crate::read::NOTICE_END
        )
    );
    let log = crate::readlog::list(&home, None, 1).unwrap();
    let last = &log[0];
    assert_eq!(
        last.result,
        crate::readlog::ReadResult::Tags {
            tags: 1,
            sessions: 2
        }
    );
    assert_eq!(last.result.text(), "listed 1 tag on 2 sessions");

    // A reader the owner has not trusted gets a refusal, logged as one.
    let untrusted = crate::trust::Reader::model(Some("unheard-of"), Some("m"));
    assert!(matches!(
        crate::read::tags(&home, &model_rt, &untrusted, &all).unwrap(),
        Tagged::Refused(_)
    ));
    assert!(matches!(
        crate::readlog::list(&home, None, 1).unwrap()[0].result,
        crate::readlog::ReadResult::Refused { .. }
    ));
}
