use super::*;
use crate::archive::Archive;
use crate::detect::Kind;
use crate::findings::Field;
use crate::harness::Harness;
use crate::read::{NOTICE_END, NOTICE_START, Range, Read};
use crate::runtime::{Call, FakeRuntime, Stored};

const MODEL: &str = "small:8b";

/// What the owner says in the synthetic session: made-up details only.
const SAID: &str = "Write to Jane Roe at 12 Example Street about the Acme Widgets invoice, \
                    or call her on 555-0134.";
/// What the agent says back.
const ANSWERED: &str = "Done: the letter to Jane Roe is drafted.";
/// What a tool printed, which the model must never be sent.
const TOOL_OUTPUT: &str = "contacts: Bob Example 555-0199";

/// A Claude Code session `id`, started at `at`: the owner asks, the agent runs a tool whose
/// output names someone else, then answers.
fn session(_id: &str, at: &str, said: &str) -> String {
    let said = serde_json::to_string(said).unwrap();
    let answered = serde_json::to_string(ANSWERED).unwrap();
    let out = serde_json::to_string(TOOL_OUTPUT).unwrap();
    r#"{"type":"user","cwd":"/work/app","timestamp":"AT","message":{"role":"user","content":SAID}}
{"type":"assistant","timestamp":"AT","message":{"role":"assistant","model":"m","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cat contacts.txt"}}]}}
{"type":"user","timestamp":"AT","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":OUT}]}}
{"type":"assistant","timestamp":"AT","message":{"role":"assistant","model":"m","content":[{"type":"text","text":ANSWERED}]}}
"#
    .replace("SAID", &said)
    .replace("ANSWERED", &answered)
    .replace("OUT", &out)
    .replace("AT", at)
}

/// A home holding `sessions` (id, start time, what the owner said), parsed and scanned, with a
/// config whose `card_model` is [`MODEL`] and no `detail_model`.
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
    let mut config = Config {
        harnesses: vec![],
        ..Default::default()
    };
    config.steward.card_model = Some(MODEL.into());
    config.save(&home.config_path()).unwrap();
    crate::findings::scan_all(&home, &config).unwrap();
    (dir, home, config)
}

fn reply(details: &[(&str, &str)]) -> String {
    let details: Vec<_> = details
        .iter()
        .map(|(kind, text)| serde_json::json!({"kind": kind, "text": text}))
        .collect();
    serde_json::json!({ "details": details }).to_string()
}

/// The answer a careful model gives for [`SAID`] and [`ANSWERED`].
fn careful() -> String {
    reply(&[
        ("person_name", "Jane Roe"),
        ("postal_address", "12 Example Street"),
        ("organization", "Acme Widgets"),
        ("phone_number", "555-0134"),
    ])
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

fn prompts(fake: &FakeRuntime) -> Vec<String> {
    fake.calls()
        .into_iter()
        .map(|c| match c {
            Call::Generate { model, prompt } => {
                assert_eq!(model, MODEL);
                prompt
            }
            other => panic!("{other:?}"),
        })
        .collect()
}

/// The text each of a session's model findings covers, with its kind and event, from the
/// owner's read.
fn found(home: &Home, id: &str) -> Vec<(u64, Kind, String)> {
    let runtime = FakeRuntime::stopped().runtime();
    let Read::Events(x) = crate::read::read(
        home,
        &runtime,
        &Reader::Owner,
        &format!("claude-code/{id}"),
        Range::default(),
    )
    .unwrap() else {
        panic!("the owner is served events")
    };
    x.findings
        .iter()
        .filter(|f| MODEL_KINDS.contains(&f.kind))
        .map(|f| {
            assert_eq!(f.field, Field::Text);
            let e = x.events.iter().find(|e| e.seq == f.event).unwrap();
            let text = e.text.as_deref().unwrap();
            (f.event, f.kind, text[f.start..f.end].to_string())
        })
        .collect()
}

#[test]
fn the_steward_sends_what_the_owner_and_the_agent_wrote_to_the_detail_model_and_places_its_answers()
{
    let (_d, home, config) = home_with(&[("s1", "2026-11-10T10:00:00Z", SAID)]);
    let fake = model(|_, _| careful());

    let r = find(&home, &config, &fake.runtime()).unwrap();
    assert_eq!(
        (r.written, r.failed, r.skipped.as_deref()),
        (1, 0, None),
        "{r:?}"
    );
    assert_eq!(
        r.model.as_deref(),
        Some(MODEL),
        "detail_model falls back to card_model"
    );

    // The model got the prompt and the messages, between the notices, and no tool output.
    let prompts = prompts(&fake);
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    let p = &prompts[0];
    assert!(p.starts_with(PROMPT), "{p}");
    assert!(p.contains(NOTICE_START) && p.contains(NOTICE_END), "{p}");
    assert!(p.contains(SAID) && p.contains(ANSWERED), "{p}");
    assert!(
        !p.contains("Bob Example") && !p.contains("cat contacts"),
        "{p}"
    );

    // The read is logged as the model at the local provider.
    let log = crate::readlog::list(&home, None, usize::MAX).unwrap();
    let line = serde_json::to_value(&log[0]).unwrap();
    assert_eq!(line["provider"], "ollama", "{line}");
    assert_eq!(line["model"], MODEL, "{line}");
    assert_eq!(line["result"], "served", "{line}");

    // Each answer is placed wherever it is written, in the owner's and the agent's messages.
    assert_eq!(
        found(&home, "s1"),
        [
            (0, Kind::PersonName, "Jane Roe".to_string()),
            (0, Kind::PostalAddress, "12 Example Street".to_string()),
            (0, Kind::Organization, "Acme Widgets".to_string()),
            (0, Kind::PhoneNumber, "555-0134".to_string()),
            (3, Kind::PersonName, "Jane Roe".to_string()),
        ]
    );
    // Findings never hold values.
    let file = std::fs::read_to_string(home.findings_path(Harness::ClaudeCode, "s1")).unwrap();
    for value in ["Jane Roe", "Example Street", "Acme", "555-0134"] {
        assert!(!file.contains(value), "{file}");
    }
}

/// A fake key, built at run time so no secret scanner mistakes it for a leak.
fn fake_key() -> String {
    ["sk-", "ant-", "api03-", &"Rd5".repeat(9)].concat()
}

#[test]
fn an_answer_not_written_in_what_the_owner_or_the_agent_wrote_is_dropped() {
    let key = fake_key();
    let said = format!("Ask Jane Roe and Janet to deploy with {key} today.");
    let (_d, home, config) = home_with(&[("s1", "2026-11-10T10:00:00Z", &said)]);
    let echoed = key.clone();
    let fake = model(move |_, prompt| {
        assert!(!prompt.contains(&echoed), "the model was served the value");
        reply(&[
            ("person_name", "Jane Roe"),
            // Not in the session at all.
            ("person_name", "Nobody Example"),
            // Only in tool output, which the model was never sent.
            ("person_name", "Bob Example"),
            // Only inside a longer word.
            ("person_name", "Jan"),
            // A secret's value, which the model never saw: never a model finding.
            ("organization", &echoed),
            // A placeholder.
            ("organization", "[secret: anthropic_key"),
        ])
    });
    let r = find(&home, &config, &fake.runtime()).unwrap();
    assert_eq!((r.written, r.found, r.unplaced), (1, 2, 4), "{r:?}");
    assert_eq!(
        found(&home, "s1"),
        [
            (0, Kind::PersonName, "Jane Roe".to_string()),
            (3, Kind::PersonName, "Jane Roe".to_string()),
        ]
    );
    let f = crate::findings::load(&home, &facts(&home, "s1")).unwrap();
    assert_eq!(
        f.all().filter(|f| f.is_secret()).count(),
        1,
        "the secret keeps its place: {f:?}"
    );
}

#[test]
fn a_reply_is_parsed_strictly() {
    let good = r#"{"details":[{"kind":"person_name","text":" Jane Roe "},{"kind":"phone_number","text":"555-0134"}]}"#;
    let r = parse_reply(good).unwrap();
    assert_eq!(
        r,
        [
            (Kind::PersonName, "Jane Roe".to_string()),
            (Kind::PhoneNumber, "555-0134".to_string())
        ]
    );
    assert_eq!(parse_reply(&format!("```json\n{good}\n```")).unwrap(), r);
    assert_eq!(parse_reply(r#"{"details":[]}"#).unwrap(), []);
    // A model in JSON mode with nothing to list may answer an empty object.
    assert_eq!(parse_reply("{}").unwrap(), []);
    assert_eq!(parse_reply("{\n\n}").unwrap(), []);
    let many: Vec<_> = (0..201)
        .map(|i| serde_json::json!({"kind": "person_name", "text": format!("P{i}")}))
        .collect();
    for bad in [
        "",
        "no json here",
        &format!("Here they are: {good}"),
        &format!("{good} and more"),
        r#"{"details":[{"kind":"email","text":"x"}]}"#,
        r#"{"details":[{"kind":"person_name","text":"x","why":"y"}]}"#,
        r#"{"details":[{"kind":"person_name"}]}"#,
        r#"{"details":[{"kind":"person_name","text":"  "}]}"#,
        r#"{"details":[],"notes":"none"}"#,
        r#"{"notes":"none"}"#,
        r#"[{"kind":"person_name","text":"x"}]"#,
        "```json\n{\"details\":[]}",
        &serde_json::json!({"details": many}).to_string(),
        &serde_json::json!({"details": [{"kind": "person_name", "text": "x".repeat(201)}]})
            .to_string(),
    ] {
        assert!(parse_reply(bad).is_err(), "{bad:?} parsed");
    }
}

#[test]
fn each_pass_can_be_rerun_alone_and_neither_overwrites_the_other() {
    let (_d, home, mut config) = home_with(&[("s1", "2026-11-10T10:00:00Z", SAID)]);
    find(&home, &config, &model(|_, _| careful()).runtime()).unwrap();
    let f = facts(&home, "s1");
    let before = crate::findings::load(&home, &f).unwrap();
    assert_eq!(before.model_findings().len(), 5);

    // A new personal term makes the pattern findings out of date; the scan keeps the model's,
    // except where a pattern finding now takes the place.
    config.privacy.personal_terms = vec!["Acme Widgets".into()];
    config.save(&home.config_path()).unwrap();
    let r = crate::findings::scan_all(&home, &config).unwrap();
    assert_eq!(r.scanned, 1);
    let after = crate::findings::load(&home, &f).unwrap();
    assert!(
        after.findings.iter().any(|f| f.kind == Kind::PersonalTerm),
        "{after:?}"
    );
    assert_eq!(after.model_findings().len(), 4, "{after:?}");
    assert!(
        after
            .model_findings()
            .iter()
            .all(|f| f.kind != Kind::Organization)
    );
    // So the model pass does not run again.
    let fake = model(|_, _| panic!("asked again"));
    let r = find(&home, &config, &fake.runtime()).unwrap();
    assert_eq!((r.current, r.written), (1, 0), "{r:?}");

    // Another model makes only the model's findings out of date; the pass rewrites them alone.
    config.steward.detail_model = Some("other:3b".into());
    let fake = FakeRuntime::running()
        .with_model("other:3b", Stored::Here)
        .replying(|_, _| reply(&[("phone_number", "555-0134")]));
    let r = find(&home, &config, &fake.runtime()).unwrap();
    assert_eq!((r.written, r.found), (1, 1), "{r:?}");
    let again = crate::findings::load(&home, &f).unwrap();
    assert_eq!(
        again.findings, after.findings,
        "the pattern findings are untouched"
    );
    assert_eq!(again.made_from, after.made_from);
    assert_eq!(again.model.unwrap().made_from.model, "other:3b");
}

#[test]
fn a_session_without_current_model_findings_is_still_served() {
    let (_d, home, config) = home_with(&[("s1", "2026-11-10T10:00:00Z", SAID)]);
    let runtime = FakeRuntime::stopped()
        .with_model(MODEL, Stored::Here)
        .runtime();
    let reader = Reader::model(Some("ollama"), Some(MODEL));
    // Pattern findings, no model findings yet.
    let read = crate::read::read(&home, &runtime, &reader, "claude-code/s1", Range::default());
    assert!(matches!(read.unwrap(), Read::Text(t) if t.contains("Jane Roe")));

    // Model findings for events that have since changed are out of date: still served, and the
    // owner is not given places that no longer fit.
    find(&home, &config, &model(|_, _| careful()).runtime()).unwrap();
    let mut a = Archive::open(&home).unwrap();
    let longer = session(
        "s1",
        "2026-11-10T10:00:00Z",
        &format!("Hello there. {SAID}"),
    );
    a.put(
        Harness::ClaudeCode,
        "/s/proj/s1.jsonl",
        "2",
        longer.as_bytes(),
    )
    .unwrap();
    drop(a);
    crate::normalize::normalize(&home, false).unwrap();
    let read = crate::read::read(&home, &runtime, &reader, "claude-code/s1", Range::default());
    assert!(matches!(read.unwrap(), Read::Text(t) if t.contains("Jane Roe")));
    assert_eq!(found(&home, "s1"), []);
    assert_eq!(progress(&home).unwrap().pending, 1);
    // The pass finds them again in the new events.
    let r = find(&home, &config, &model(|_, _| careful()).runtime()).unwrap();
    assert_eq!(r.written, 1, "{r:?}");
    assert_eq!(
        found(&home, "s1")[0],
        (0, Kind::PersonName, "Jane Roe".to_string())
    );
}

#[test]
fn sessions_get_the_pass_newest_first_up_to_sessions_per_run_and_never_twice() {
    let (_d, home, mut config) = home_with(&[
        ("a", "2026-11-01T10:00:00Z", "first: Jane Roe"),
        ("b", "2026-11-03T10:00:00Z", "third: Jane Roe"),
        ("c", "2026-11-02T10:00:00Z", "second: Jane Roe"),
    ]);
    config.steward.sessions_per_run = 2;
    let fake = model(|_, _| reply(&[("person_name", "Jane Roe")]));
    let r = find(&home, &config, &fake.runtime()).unwrap();
    assert_eq!((r.sessions, r.pending, r.written), (3, 3, 2), "{r:?}");
    let asked: Vec<String> = prompts(&fake)
        .iter()
        .map(|p| {
            p.split(": Jane")
                .next()
                .unwrap()
                .rsplit('\n')
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(asked, ["third", "second"]);
    let r = find(&home, &config, &fake.runtime()).unwrap();
    assert_eq!((r.current, r.pending, r.written), (2, 1, 1), "{r:?}");
    let r = find(&home, &config, &fake.runtime()).unwrap();
    assert_eq!((r.current, r.written), (3, 0), "{r:?}");
    assert_eq!(
        prompts(&fake).len(),
        3,
        "current sessions are not asked again"
    );
}

#[test]
fn an_unusable_answer_is_a_failure_for_that_session_and_is_retried_after_the_rest() {
    let (_d, home, config) = home_with(&[
        ("a", "2026-11-01T10:00:00Z", "first"),
        ("b", "2026-11-02T10:00:00Z", "second"),
    ]);
    let bad = model(|_, prompt| {
        if prompt.contains("second") {
            "Sure! Jane Roe is a name.".into()
        } else {
            reply(&[])
        }
    });
    let r = find(&home, &config, &bad.runtime()).unwrap();
    assert_eq!((r.written, r.failed), (1, 1), "{r:?}");
    assert!(
        r.errors[0].starts_with("claude-code/b: the model's answer is not a usable list"),
        "{r:?}"
    );
    assert!(
        !r.errors[0].contains("Jane"),
        "no answer text in the run log: {r:?}"
    );
    assert_eq!(progress(&home).unwrap().failing, 1);
    let r = find(&home, &config, &model(|_, _| reply(&[])).runtime()).unwrap();
    assert_eq!((r.written, r.failed), (1, 0), "{r:?}");
    assert_eq!(progress(&home).unwrap().failing, 0);
}

#[test]
fn a_long_session_is_sent_in_parts_each_between_the_notices() {
    let filler = "the widget module needs a rename and a test. ".repeat(500);
    let said = format!("{filler}\nFinally, thank Jane Roe.");
    let (_d, home, config) = home_with(&[("s1", "2026-11-10T10:00:00Z", &said)]);
    let fake = model(|_, prompt| {
        if prompt.contains("thank Jane Roe") {
            reply(&[("person_name", "Jane Roe")])
        } else {
            reply(&[])
        }
    });
    let r = find(&home, &config, &fake.runtime()).unwrap();
    assert_eq!((r.written, r.found), (1, 2), "{r:?}");
    let prompts = prompts(&fake);
    assert!(prompts.len() >= 2, "{}", prompts.len());
    for p in &prompts {
        let part = p.strip_prefix(PROMPT).unwrap();
        assert!(part.starts_with(NOTICE_START) && part.trim_end().ends_with(NOTICE_END));
        assert!(part.len() <= PART_BUDGET + NOTICE_START.len() + NOTICE_END.len() + 3);
    }
}

#[test]
fn with_no_model_or_no_local_runtime_that_can_run_it_the_pass_is_skipped() {
    let (_d, home, mut config) = home_with(&[("s1", "2026-11-10T10:00:00Z", SAID)]);
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
            "detail_model \"small:8b\" is not stored on this machine",
        ),
    ];
    for (fake, why) in cases {
        let r = find(&home, &config, &fake.runtime()).unwrap();
        assert_eq!(r.skipped.as_deref(), Some(why), "{r:?}");
        assert_eq!(r.summary(), format!("model detection skipped: {why}"));
        assert_eq!(fake.calls(), []);
    }
    config.steward.card_model = None;
    let fake = model(|_, _| careful());
    let r = find(&home, &config, &fake.runtime()).unwrap();
    assert_eq!(
        r.skipped.as_deref(),
        Some("no detail_model or card_model is set")
    );
    assert_eq!(fake.calls(), []);
    assert!(
        crate::readlog::list(&home, None, usize::MAX)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_pass_is_the_last_step_of_the_tick_and_status_shows_its_progress() {
    let (_d, home, mut config) = home_with(&[
        ("a", "2026-11-01T10:00:00Z", "first"),
        ("b", "2026-11-02T10:00:00Z", "second"),
    ]);
    assert_eq!(crate::tick::STEPS.last().unwrap().name, "details");
    crate::tick::tick_with(&home, &config, &FakeRuntime::stopped().runtime()).unwrap();
    let s = crate::status::status(&home).unwrap();
    assert_eq!(s.details.model.as_deref(), Some(MODEL));
    assert_eq!(
        s.details.skipped.as_deref(),
        Some("the local runtime is not running")
    );
    assert_eq!(
        (s.details.sessions, s.details.current, s.details.pending),
        (2, 0, 2)
    );

    config.steward.sessions_per_run = 1;
    let fake = model(|_, prompt| {
        if prompt.starts_with(PROMPT) {
            reply(&[])
        } else {
            r#"{"title":"T","summary":"S.","tags":[],"outcome":"done"}"#.into()
        }
    });
    let r = crate::tick::tick_with(&home, &config, &fake.runtime()).unwrap();
    let step = r.steps.last().unwrap();
    assert_eq!(step.name, "details");
    assert!(step.ok, "{step:?}");
    assert_eq!(step.report["written"], 1, "{step:?}");
    let s = crate::status::status(&home).unwrap();
    assert_eq!(s.details.skipped, None);
    assert_eq!((s.details.current, s.details.pending), (1, 1));

    config.steward.card_model = None;
    config.save(&home.config_path()).unwrap();
    let s = crate::status::status(&home).unwrap();
    assert_eq!(
        s.details.skipped.as_deref(),
        Some("no detail_model or card_model is set")
    );
}

#[test]
fn the_rotation_list_and_lookups_include_the_models_findings() {
    use crate::read::{FindingFilter, Kinds, Order, Page};
    let (_d, home, config) = home_with(&[("s1", "2026-11-10T10:00:00Z", SAID)]);
    find(&home, &config, &model(|_, _| careful()).runtime()).unwrap();
    let runtime = FakeRuntime::stopped().runtime();
    let filter = FindingFilter {
        kinds: Kinds::parse("person_name").unwrap(),
        ..Default::default()
    };
    let crate::read::Answer::Served(list) = crate::read::fingerprints(
        &home,
        &runtime,
        &Reader::Owner,
        &filter,
        Order::default(),
        Page::default(),
    )
    .unwrap() else {
        panic!("the owner is served the list")
    };
    assert_eq!(list.rows.len(), 1, "{list:?}");
    assert_eq!(list.rows[0].kinds, [Kind::PersonName]);
    let short = list.rows[0].fingerprint.short().to_string();
    assert_eq!(crate::findings::lookup(&home, &short).unwrap().len(), 1);
}
