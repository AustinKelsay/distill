//! Sessions: the owner searches and reads their sessions, every finding covered until clicked.
//! The tests that read go through core's read path against a temporary home with synthetic
//! sessions; the rest use fixture state.

use std::sync::Arc;

use distill_core::readlog::{self, ReadResult};
use distill_core::runtime::{FakeRuntime, Stored};
use distill_core::trust::Reader;
use distill_core::{Config, Harness, Home};

use super::*;
use crate::fixture;
use crate::jobs::{Core, Timer};
use crate::nav::Screen;
use crate::screens::sessions;

/// A made-up key in the shape of a real one, built at run time.
fn fake_key() -> String {
    ["sk-", "ant-", "api03-", &"Gu1".repeat(9)].concat()
}

fn fake_email() -> String {
    ["avery.example", "@", "example.com"].concat()
}

fn claude(cwd: &str, at: &str, said: &str, result: &str) -> String {
    let said = serde_json::to_string(said).unwrap();
    let result = serde_json::to_string(result).unwrap();
    format!(
        r#"{{"type":"user","cwd":"{cwd}","version":"2.1.0","timestamp":"{at}","message":{{"role":"user","content":{said}}}}}
{{"type":"assistant","timestamp":"{at}","message":{{"role":"assistant","model":"claude-opus-5-5","content":[{{"type":"thinking","thinking":"look at the tests first"}},{{"type":"text","text":"Running the tests."}},{{"type":"tool_use","id":"t1","name":"Bash","input":{{"command":"cargo test --workspace"}}}}]}}}}
{{"type":"user","timestamp":"{at}","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":{result},"is_error":true}}]}}}}
{{"type":"assistant","timestamp":"{at}","message":{{"role":"assistant","model":"claude-opus-5-5","content":[{{"type":"text","text":"One test fails; fixing it."}}]}}}}
"#
    )
}

/// A temporary home with two synthetic Claude Code sessions, scanned, indexed and carded by a
/// fake local model. The first holds a fake key in what the owner wrote and an email in a tool
/// result; its card's title names the email.
fn archive(dir: &std::path::Path) -> Core {
    let root = dir.join("harness/claude");
    std::fs::create_dir_all(root.join("-w")).unwrap();
    std::fs::write(
        root.join("-w/aaaa.jsonl"),
        claude(
            "/work/app",
            "2026-11-10T10:00:00Z",
            &format!("deploy the app with {} please", fake_key()),
            &format!("1 failed\nmail {} about it", fake_email()),
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("-w/bbbb.jsonl"),
        claude(
            "/work/notes",
            "2026-11-12T10:00:00Z",
            "tidy the notes folder",
            "ok",
        ),
    )
    .unwrap();
    let mut config = Config::default();
    for h in &mut config.harnesses {
        h.enabled = false;
        h.roots = vec![dir.join("harness/none")];
    }
    let h = config.harness_mut(Harness::ClaudeCode);
    h.enabled = true;
    h.roots = vec![root];
    config.steward.card_model = Some("small:4b".into());
    let home = Home::new(dir.join("home"));
    home.ensure().unwrap();
    config.save(&home.config_path()).unwrap();
    let email = fake_email();
    let fake = FakeRuntime::running()
        .with_model("small:4b", Stored::Here)
        .replying(move |_, prompt| {
            if prompt.contains("deploy the app") {
                format!(
                    r#"{{"title":"Deploy and mail {email}","summary":"Deployed the app; a test failed and {email} was told.","tags":["deploy"],"outcome":"partly_done"}}"#
                )
            } else {
                r#"{"title":"Tidy the notes","summary":"Tidied the notes folder.","tags":["notes"],"outcome":"done"}"#.into()
            }
        });
    let runtime = fake.runtime();
    distill_core::tick::tick_with(&home, &config, &runtime).unwrap();
    Core {
        home,
        runtime,
        timer: Timer {
            install: Arc::new(|_, _| panic!("no test installs the timer here")),
            installed: Arc::new(|| false),
        },
        places: Arc::new(Vec::new),
    }
}

fn window(core: &Core) -> Runtime<State> {
    let config = core.home.load_config().unwrap();
    let mut s = State::new(core.home.clone(), Some(config));
    s.fixture = true;
    let mut rt = runtime_sized(s, (1080.0, 1400.0));
    press(&mut rt, "nav-sessions");
    run_jobs(&mut rt, core);
    rt
}

/// Every string the window shows or names, joined.
fn everything(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .flat_map(|e| [e.text, e.label])
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
        // The pads beside a chip are drawn, not read: `crate::cover::PAD`.
        .replace(crate::cover::PAD, "")
}

fn owner_lines(home: &Home) -> Vec<ReadResult> {
    readlog::list(home, Some("owner"), 100)
        .unwrap()
        .into_iter()
        .filter(|e| e.reader == Reader::Owner)
        .map(|e| e.result)
        .collect()
}

/// Press points across the element `key` until a press changes what is revealed: Rune exposes
/// no rect for a link's glyphs, so this is the headless stand-in for clicking a chip.
fn click_a_chip(rt: &mut Runtime<State>, key: &str) -> Option<(f32, f32)> {
    let e = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))?;
    let [x, y, w, h] = e.rect;
    let before = revealed(rt);
    let mut py = y + 4.0;
    while py < y + h {
        let mut px = x + 3.0;
        while px < x + w {
            rt.pointer_down(px, py);
            rt.pointer_up(px, py);
            rt.frame(1.0 / 60.0);
            if revealed(rt) != before {
                settle(rt);
                return Some((px, py));
            }
            px += 5.0;
        }
        py += 6.0;
    }
    None
}

fn revealed(rt: &Runtime<State>) -> usize {
    rt.state
        .sessions
        .open
        .as_ref()
        .map_or(0, |o| o.revealed.len())
}

#[test]
fn the_sessions_screen_lists_the_archive_with_cards_and_logs_the_owners_list() {
    let dir = tempfile::tempdir().unwrap();
    let core = archive(dir.path());
    let rt = window(&core);
    assert_eq!(rt.state.screen, Screen::Sessions);
    let all = everything(&rt);
    assert!(all.contains("Tidy the notes"), "{all}");
    assert!(all.contains("Tidied the notes folder."), "{all}");
    // The card's title and summary cover the email they name.
    assert!(all.contains("Deploy and mail ‹email›"), "{all}");
    assert!(!all.contains(&fake_email()), "{all}");
    assert!(all.contains("2 sessions"), "{all}");
    assert!(
        owner_lines(&core.home)
            .iter()
            .any(|r| matches!(r, ReadResult::Listed { sessions: 2 })),
        "{:?}",
        owner_lines(&core.home)
    );
}

#[test]
fn a_search_shows_the_matching_passage_with_its_findings_covered() {
    let dir = tempfile::tempdir().unwrap();
    let core = archive(dir.path());
    let mut rt = window(&core);
    rt.update(|s, _| s.sessions.query.replace_text("failed"));
    press(&mut rt, "search-button");
    assert_eq!(run_jobs(&mut rt, &core), ["tags", "sessions"]);
    let all = everything(&rt);
    assert!(all.contains("1 session matches"), "{all}");
    assert!(all.contains("1 failed"), "{all}");
    assert!(all.contains("mail ‹email› about it"), "{all}");
    assert!(!all.contains("Tidy the notes"), "{all}");
    assert!(!all.contains(&fake_email()), "{all}");
}

#[test]
fn filters_narrow_the_list_and_a_bad_date_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let core = archive(dir.path());
    let mut rt = window(&core);
    rt.update(|s, _| s.sessions.filters.project.replace_text("/work/notes"));
    press(&mut rt, "search-button");
    run_jobs(&mut rt, &core);
    let all = everything(&rt);
    assert!(all.contains("Tidy the notes"), "{all}");
    assert!(!all.contains("Deploy and mail"), "{all}");

    rt.update(|s, _| {
        s.sessions.filters.project.replace_text("");
        s.sessions.filters.outcome = Some(distill_core::cards::Outcome::PartlyDone);
    });
    press(&mut rt, "search-button");
    run_jobs(&mut rt, &core);
    let all = everything(&rt);
    assert!(all.contains("Deploy and mail"), "{all}");
    assert!(!all.contains("Tidy the notes"), "{all}");

    rt.update(|s, _| s.sessions.filters.since.replace_text("last week"));
    press(&mut rt, "search-button");
    assert!(run_jobs(&mut rt, &core).is_empty(), "nothing is asked");
    assert!(shows(&rt, "Write dates as YYYY-MM-DD"), "{:?}", texts(&rt));
}

#[test]
fn the_tags_in_use_are_chips_and_a_press_on_one_or_on_a_sessions_tag_filters_by_it() {
    let dir = tempfile::tempdir().unwrap();
    let core = archive(dir.path());
    let mut rt = window(&core);
    let all = everything(&rt);
    assert!(all.contains("#deploy · 1"), "{all}");
    assert!(all.contains("#notes · 1"), "{all}");
    assert!(
        owner_lines(&core.home).iter().any(|r| matches!(
            r,
            ReadResult::Tags {
                tags: 2,
                sessions: 2
            }
        )),
        "{:?}",
        owner_lines(&core.home)
    );

    let notes = rt
        .state
        .sessions
        .tags
        .iter()
        .position(|t| t.tag == "notes")
        .unwrap();
    press(&mut rt, &format!("top-tag-{notes}"));
    assert_eq!(
        run_jobs(&mut rt, &core),
        ["tags", "sessions"],
        "the chips are asked for again"
    );
    assert_eq!(rt.state.sessions.filters.tag.text, "notes");
    let all = everything(&rt);
    assert!(all.contains("Tidy the notes"), "{all}");
    assert!(!all.contains("Deploy and mail"), "{all}");
    press(&mut rt, &format!("top-tag-{notes}"));
    run_jobs(&mut rt, &core);
    assert_eq!(
        rt.state.sessions.filters.tag.text, "",
        "a second press stops"
    );

    // Several chips filter by all of their tags (#159), and a press on one of them drops it.
    let chip = |rt: &mut Runtime<State>, tag: &str| {
        let i = rt.state.sessions.tags.iter().position(|t| t.tag == tag);
        press(rt, &format!("top-tag-{}", i.unwrap()));
    };
    for tag in ["notes", "deploy"] {
        chip(&mut rt, tag);
        run_jobs(&mut rt, &core);
    }
    assert_eq!(rt.state.sessions.filters.tag.text, "notes deploy");
    assert_eq!(rt.state.sessions.results.as_ref().unwrap().total, 0);
    chip(&mut rt, "notes");
    run_jobs(&mut rt, &core);
    assert_eq!(rt.state.sessions.filters.tag.text, "deploy");
    let all = everything(&rt);
    assert!(all.contains("Deploy and mail"), "{all}");
    assert!(!all.contains("Tidy the notes"), "{all}");
    chip(&mut rt, "deploy");
    run_jobs(&mut rt, &core);
    assert_eq!(rt.state.sessions.filters.tag.text, "");

    rt.update(|s, _| sessions::open_row(s, 0));
    run_jobs(&mut rt, &core);
    let tag = rt
        .state
        .sessions
        .open
        .as_ref()
        .unwrap()
        .row
        .card
        .as_ref()
        .unwrap()
        .card
        .tags[0]
        .clone();
    press(&mut rt, "session-tag-0");
    run_jobs(&mut rt, &core);
    assert!(rt.state.sessions.open.is_none(), "back on the list");
    assert_eq!(rt.state.sessions.filters.tag.text, tag);
    assert_eq!(rt.state.sessions.results.as_ref().unwrap().total, 1);

    // While a search runs a chip can't be pressed, so the results never land under a filter
    // they didn't apply.
    press(&mut rt, "clear");
    assert!(rt.state.sessions.loading);
    press(&mut rt, &format!("top-tag-{notes}"));
    assert_eq!(rt.state.sessions.filters.tag.text, "");
    run_jobs(&mut rt, &core);

    // Back before the tags asked on opening land: the chips wait for them.
    rt.update(|s, _| sessions::open_row(s, 0));
    settle(&mut rt);
    press(&mut rt, "back");
    let stale = rt
        .state
        .sessions
        .tags
        .iter()
        .position(|t| t.tag == "notes")
        .unwrap();
    press(&mut rt, &format!("top-tag-{stale}"));
    assert_eq!(rt.state.sessions.filters.tag.text, "");
    run_jobs(&mut rt, &core);

    // A tag that becomes a personal term after the list was read can't be pressed in the session.
    let mut config = core.home.load_config().unwrap();
    config.privacy.personal_terms.push("deploy".into());
    config.save(&core.home.config_path()).unwrap();
    let deploy = rt
        .state
        .sessions
        .results
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .position(|r| {
            r.card
                .as_ref()
                .is_some_and(|c| c.card.tags.iter().any(|t| t == "deploy"))
        });
    rt.update(|s, _| sessions::open_row(s, deploy.unwrap()));
    assert_eq!(run_jobs(&mut rt, &core), ["tags", "read session"]);
    assert!(rt.find("session-tag-0").is_none(), "{:?}", texts(&rt));
    press(&mut rt, "back");
    run_jobs(&mut rt, &core);

    // A tag that becomes a personal term leaves the chips at the next search.
    let mut config = core.home.load_config().unwrap();
    config.privacy.personal_terms.push("notes".into());
    config.save(&core.home.config_path()).unwrap();
    press(&mut rt, "search-button");
    run_jobs(&mut rt, &core);
    assert!(rt.state.sessions.tags.iter().all(|t| t.tag != "notes"));
    assert!(!everything(&rt).contains("#notes · 1"));
}

#[test]
fn the_owner_adds_and_hides_a_tag_on_an_open_session() {
    let dir = tempfile::tempdir().unwrap();
    let core = archive(dir.path());
    let mut rt = window(&core);
    let row = rt
        .state
        .sessions
        .results
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .position(|r| r.card.as_ref().is_some_and(|c| c.card.tags == ["deploy"]));
    rt.update(|s, _| sessions::open_row(s, row.unwrap()));
    run_jobs(&mut rt, &core);
    let tags = |rt: &Runtime<State>| {
        rt.state
            .sessions
            .open
            .as_ref()
            .unwrap()
            .row
            .card
            .as_ref()
            .unwrap()
            .card
            .tags
            .clone()
    };

    rt.update(|s, _| {
        s.sessions
            .open
            .as_mut()
            .unwrap()
            .new_tag
            .replace_text("Ops Work")
    });
    settle(&mut rt);
    press(&mut rt, "add-tag");
    assert_eq!(
        run_jobs(&mut rt, &core),
        ["tag session", "tags", "sessions"],
        "then the list and the chips again"
    );
    assert_eq!(tags(&rt), ["deploy", "ops-work"]);
    assert_eq!(rt.state.sessions.open.as_ref().unwrap().new_tag.text, "");
    run_jobs(&mut rt, &core);
    assert!(
        rt.state.sessions.tags.iter().any(|t| t.tag == "ops-work"),
        "the chips follow"
    );

    // What the owner types while an edit is saved stays typed.
    let type_tag = |rt: &mut Runtime<State>, t: &str| {
        rt.update(|s, _| s.sessions.open.as_mut().unwrap().new_tag.replace_text(t));
        settle(rt);
    };
    type_tag(&mut rt, "ops-work");
    press(&mut rt, "add-tag");
    type_tag(&mut rt, "next");
    run_jobs(&mut rt, &core);
    assert_eq!(
        rt.state.sessions.open.as_ref().unwrap().new_tag.text,
        "next"
    );

    press(&mut rt, "hide-tag-0");
    run_jobs(&mut rt, &core);
    assert_eq!(tags(&rt), ["ops-work"]);
    let owner = distill_core::tags::OwnerTags::load(&core.home).unwrap();
    let edit = owner.sessions.values().next().unwrap();
    assert_eq!(
        (&edit.add[..], &edit.hide[..]),
        (&["ops-work".to_string()][..], &["deploy".to_string()][..])
    );

    // A secret is refused, and stays out of the file.
    rt.update(|s, _| {
        s.sessions
            .open
            .as_mut()
            .unwrap()
            .new_tag
            .replace_text(fake_key())
    });
    settle(&mut rt);
    press(&mut rt, "add-tag");
    run_jobs(&mut rt, &core);
    assert!(
        shows(&rt, "a tag can't hold a secret or a personal detail"),
        "{:?}",
        texts(&rt)
    );
    assert!(
        !std::fs::read_to_string(core.home.owner_tags_path())
            .unwrap()
            .contains(&fake_key())
    );
}

#[test]
fn a_tag_edit_shows_nothing_of_a_card_rewritten_meanwhile_and_its_refresh_waits_for_a_page() {
    let dir = tempfile::tempdir().unwrap();
    let core = archive(dir.path());
    let mut rt = window(&core);
    let row = rt
        .state
        .sessions
        .results
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .position(|r| r.facts.session_id == "aaaa")
        .unwrap();
    rt.update(|s, _| sessions::open_row(s, row));
    run_jobs(&mut rt, &core);
    click_a_chip(&mut rt, "session-title").expect("the title's email is covered");

    // The card is rewritten with another email while the session is open, then tagged.
    let other = ["another.synthetic", "@", "example.com"].concat();
    let said = other.clone();
    let config = core.home.load_config().unwrap();
    let fake = FakeRuntime::running()
        .with_model("small:4b", Stored::Here)
        .replying(move |_, _| format!(
            r#"{{"title":"Contact {said}","summary":"Updated card.","tags":["deploy"],"outcome":"done"}}"#
        ));
    std::fs::remove_file(core.home.card_path(None, Harness::ClaudeCode, "aaaa")).unwrap();
    let written = distill_core::cards::write(
        &core.home,
        &config,
        &fake.runtime(),
        distill_core::tick::Deadline::NONE,
    )
    .unwrap();
    assert_eq!(written.written, 1);
    rt.update(|s, _| sessions::retag(s, Some("rollout".into()), None));
    run_jobs(&mut rt, &core);
    assert!(
        !everything(&rt).contains(&other),
        "a reveal on the old card shows nothing on the new"
    );

    // Back and More before the edit lands: the list is read again once that page has.
    rt.update(|s, _| {
        sessions::retag(s, Some("cleanup".into()), None);
        sessions::close(s);
        let r = s.sessions.results.as_mut().unwrap();
        r.rows.truncate(1);
        r.next = Some(1);
        sessions::more(s);
    });
    run_jobs(&mut rt, &core);
    assert!(rt.state.sessions.tags.iter().any(|t| t.tag == "cleanup"));
}

#[test]
fn every_filter_the_spec_names_is_on_the_screen() {
    let rt = runtime_sized(fixture::sessions_listed(), (1080.0, 1400.0));
    for key in [
        "search",
        "filter-harness-trigger",
        "filter-model",
        "filter-project",
        "filter-since",
        "filter-until",
        "filter-tag",
        "filter-outcome-trigger",
    ] {
        assert!(rt.find(key).is_some(), "no {key}: {:?}", texts(&rt));
    }
}

#[test]
fn opening_a_result_shows_its_events_in_order_told_apart_and_is_in_the_read_log() {
    let dir = tempfile::tempdir().unwrap();
    let core = archive(dir.path());
    let mut rt = window(&core);
    let row = rt
        .state
        .sessions
        .results
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .position(|r| r.facts.session_id == "aaaa")
        .unwrap();
    press(&mut rt, &format!("result-{row}"));
    assert_eq!(run_jobs(&mut rt, &core), ["tags", "read session"]);
    let heads: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| e.key.as_deref() == Some("event-head"))
        .filter_map(|e| e.label)
        .collect();
    assert_eq!(
        heads,
        [
            "Event 0, you",
            "Event 1, reasoning",
            "Event 2, assistant",
            "Event 3, tool call, Bash",
            "Event 4, tool result, error",
            "Event 5, assistant",
        ]
    );
    let all = everything(&rt);
    assert!(all.contains("command: cargo test --workspace"), "{all}");
    assert!(
        owner_lines(&core.home)
            .iter()
            .any(|r| matches!(r, ReadResult::Served { events: 6 })),
        "{:?}",
        owner_lines(&core.home)
    );
}

#[test]
fn a_click_shows_one_finding_and_closing_the_session_covers_it_again() {
    let dir = tempfile::tempdir().unwrap();
    let core = archive(dir.path());
    let mut rt = window(&core);
    let row = rt
        .state
        .sessions
        .results
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .position(|r| r.facts.session_id == "aaaa")
        .unwrap();
    press(&mut rt, &format!("result-{row}"));
    run_jobs(&mut rt, &core);
    let all = everything(&rt);
    assert!(
        all.contains("deploy the app with ‹anthropic key› please"),
        "{all}"
    );
    assert!(all.contains("mail ‹email› about it"), "{all}");
    assert!(!all.contains(&fake_key()) && !all.contains(&fake_email()));

    // The key is in event 0's text, the first text block.
    let at = click_a_chip(&mut rt, "text-0").expect("a chip to click");
    assert_eq!(revealed(&rt), 1);
    let all = everything(&rt);
    assert!(all.contains(&fake_key()), "the clicked finding shows");
    assert!(!all.contains(&fake_email()), "the other stays covered");
    // A second click covers it again.
    rt.pointer_down(at.0, at.1);
    rt.pointer_up(at.0, at.1);
    settle(&mut rt);
    assert_eq!(revealed(&rt), 0);
    assert!(!everything(&rt).contains(&fake_key()));

    // Shown, then closed and opened again: covered.
    click_a_chip(&mut rt, "text-0").unwrap();
    assert!(everything(&rt).contains(&fake_key()));
    press(&mut rt, "back");
    assert!(rt.state.sessions.open.is_none());
    press(&mut rt, &format!("result-{row}"));
    run_jobs(&mut rt, &core);
    assert_eq!(revealed(&rt), 0);
    assert!(!everything(&rt).contains(&fake_key()));
}

#[test]
fn leaving_the_screen_covers_every_finding_again() {
    let mut rt = runtime_sized(fixture::session_revealed(), (1080.0, 1400.0));
    assert_eq!(revealed(&rt), 1);
    press(&mut rt, "nav-overview");
    press(&mut rt, "nav-sessions");
    assert_eq!(revealed(&rt), 0);
}

#[test]
fn a_covered_value_is_never_in_the_element_tree_or_the_snapshot_text() {
    for (scene, s) in [
        ("listed", fixture::sessions_listed()),
        ("searched", fixture::sessions_searched()),
        ("session", fixture::session_covered()),
        ("revealed", fixture::session_revealed()),
    ] {
        let values = sessions::values(&s.sessions).0;
        let shown = sessions::values(&s.sessions).1;
        assert!(!values.is_empty(), "{scene} has covered findings");
        let rt = runtime_sized(s, super::snapshots::SNAP_SIZE);
        let tree = everything(&rt);
        // Every field of every element, which is what `--tree` prints.
        let fields = format!("{:?}", rt.elements());
        let snap = super::snapshots::dump(&rt);
        for v in &values {
            let once = shown.iter().filter(|x| *x == v).count();
            if once == 0 {
                assert!(!tree.contains(v.as_str()), "{scene}: {v:?} in the tree");
                assert!(!fields.contains(v.as_str()), "{scene}: {v:?} in an element");
                assert!(!snap.contains(v.as_str()), "{scene}: {v:?} in the snapshot");
                continue;
            }
            // The same value shown where the owner clicked it: in that one text, nowhere else.
            let texts = rt
                .elements()
                .into_iter()
                .filter(|e| e.role == "text")
                .filter(|e| e.text.as_deref().is_some_and(|t| t.contains(v.as_str())))
                .count();
            assert_eq!(texts, once, "{scene}: {v:?} shows in {texts} texts");
        }
        assert!(snap.contains('‹'), "{scene} shows chips");
    }
}

#[test]
fn the_revealed_scene_shows_only_the_finding_clicked() {
    let s = fixture::session_revealed();
    let shown = sessions::values(&s.sessions).1;
    assert_eq!(shown.len(), 1);
    let rt = runtime_sized(s, super::snapshots::SNAP_SIZE);
    assert!(everything(&rt).contains(shown[0].as_str()));
}

#[test]
fn a_finding_that_cannot_be_placed_covers_the_whole_event() {
    let mut x = fixture::deploy_excerpt();
    // The bearer token in the tool call's input, said to be at a place the input lacks.
    let f = x
        .findings
        .iter_mut()
        .find(|f| f.kind == distill_core::detect::Kind::Bearer)
        .unwrap();
    f.field = distill_core::findings::Field::Input {
        path: vec![9],
        key: false,
    };
    let rt = runtime_sized(fixture::session_with(x), (1080.0, 1400.0));
    let all = everything(&rt);
    assert!(!all.contains(&fixture::fake_bearer()), "{all}");
    assert!(
        !all.contains("curl -H"),
        "the whole event is covered: {all}"
    );
    assert!(all.contains("‹bearer›"), "{all}");
}

#[test]
fn long_events_are_split_into_rows_of_at_most_forty_lines_and_never_inside_a_finding() {
    let s = fixture::session_long(60, 3);
    let open = s.sessions.open.as_ref().unwrap();
    let doc = open.doc().unwrap();
    for b in &doc.blocks {
        let e = &doc.events[b.event];
        let lines = e.text[b.start..b.end].matches('\n').count() + 1;
        assert!(lines <= sessions::BLOCK_LINES, "{lines} lines in a row");
        for m in &e.marks {
            assert!(
                m.end <= b.start || m.start >= b.end || (m.start >= b.start && m.end <= b.end),
                "a finding cut by a row"
            );
        }
    }
    assert!(
        doc.blocks.len() > doc.events.len(),
        "long events take several rows"
    );
    let mut rt = runtime_sized(s, (1080.0, 900.0));
    // Scrolls to the end without trouble.
    for _ in 0..200 {
        rt.wheel(700.0, 500.0, -400.0);
        rt.frame(1.0 / 60.0);
    }
    assert!(shows(&rt, "‹"), "{:?}", texts(&rt));
}

/// CPU frame times for a long synthetic session (update, layout and scene; no GPU). Run with
/// `cargo test -p distill-gui --release -- --ignored frame_times --nocapture`.
#[test]
#[ignore]
fn frame_times() {
    use std::time::Instant;
    let stats = |label: &str, mut ms: Vec<f64>| {
        ms.sort_by(f64::total_cmp);
        let q = |p: f64| ms[((ms.len() - 1) as f64 * p).round() as usize];
        println!(
            "{label:<40} n={:<4} p50 {:>6.2} ms  p95 {:>6.2} ms  max {:>6.2} ms",
            ms.len(),
            q(0.5),
            q(0.95),
            q(1.0)
        );
    };
    let t = Instant::now();
    let s = fixture::session_long(5_000, 97);
    println!(
        "built and cut into rows in {:.0} ms (on the worker)",
        t.elapsed().as_secs_f64() * 1e3
    );
    let open = s.sessions.open.as_ref().unwrap();
    let doc = open.doc().unwrap();
    println!(
        "{} events, {} rows, {} findings, longest event {} KB",
        doc.events.len(),
        doc.blocks.len(),
        doc.events.iter().map(|e| e.marks.len()).sum::<usize>(),
        doc.events.iter().map(|e| e.text.len()).max().unwrap() / 1024
    );
    let mut rt = Runtime::new(s, crate::view, (1180.0, 760.0), Env::new(true, 2.0));
    rt.frame(0.0);
    settle(&mut rt);
    let timed = |rt: &mut Runtime<State>, f: &mut dyn FnMut(&mut Runtime<State>), n: usize| {
        (0..n)
            .map(|_| {
                let t = Instant::now();
                f(rt);
                rt.frame(1.0 / 120.0);
                t.elapsed().as_secs_f64() * 1e3
            })
            .collect::<Vec<_>>()
    };
    stats(
        "wheel scroll, 40 pt a frame",
        timed(&mut rt, &mut |rt| rt.wheel(700.0, 400.0, -40.0), 600),
    );
    let before = texts(&rt);
    stats(
        "fling, 400 pt a frame",
        timed(&mut rt, &mut |rt| rt.wheel(700.0, 400.0, -400.0), 300),
    );
    assert_ne!(texts(&rt), before, "the session scrolled");
    let mut far = 0usize;
    stats(
        "jump to an unvisited row each frame",
        timed(
            &mut rt,
            &mut |rt| {
                far = (far + 2_311) % 6_000;
                let key = format!("block-{far}");
                rt.update(|_, tx| tx.scroll_into_view(&key));
            },
            120,
        ),
    );
    rt.update(|_, tx| tx.scroll_into_view("block-0"));
    settle(&mut rt);
    stats(
        "show or cover a finding",
        timed(
            &mut rt,
            &mut |rt| rt.update(|s, _| sessions::toggle(s, "f0")),
            60,
        ),
    );
}

#[test]
fn a_search_says_when_search_by_meaning_covered_only_part_of_the_archive() {
    use crate::screens::sessions::meaning_note;
    use distill_core::read::Meaning;
    assert_eq!(meaning_note(&Meaning::Used { unembedded: 0 }), None);
    assert_eq!(
        meaning_note(&Meaning::Used { unembedded: 4_088 }).as_deref(),
        Some("4,088 sessions have no vectors yet, so only their words find them.")
    );
    assert_eq!(
        meaning_note(&Meaning::Used { unembedded: 1 }).as_deref(),
        Some("1 session has no vectors yet, so only its words find it.")
    );
    assert_eq!(
        meaning_note(&Meaning::Skipped("no embedding_model is set".into())).as_deref(),
        Some("Search by meaning is off (no embedding_model is set): word matches only.")
    );
}

#[test]
fn a_tag_with_a_comma_in_it_filters_whole_and_its_owner_can_hide_it() {
    let dir = tempfile::tempdir().unwrap();
    let core = archive(dir.path());
    let tag = "notes,archive";
    let saved = serde_json::json!({"sessions": {"claude-code/bbbb": {"add": [tag]}}});
    std::fs::write(
        core.home.owner_tags_path(),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    let mut rt = window(&core);
    let chip = |rt: &mut Runtime<State>| {
        let i = rt.state.sessions.tags.iter().position(|c| c.tag == tag);
        press(rt, &format!("top-tag-{}", i.unwrap()));
        run_jobs(rt, &core);
    };
    chip(&mut rt);
    assert_eq!(rt.state.sessions.results.as_ref().unwrap().total, 1);
    chip(&mut rt);
    assert_eq!(
        rt.state.sessions.filters.tag.text, "",
        "a second press stops"
    );

    rt.update(|s, _| {
        let row = s
            .sessions
            .results
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .position(|r| {
                r.card
                    .as_ref()
                    .is_some_and(|c| c.card.tags.iter().any(|t| t == tag))
            });
        sessions::open_row(s, row.unwrap())
    });
    run_jobs(&mut rt, &core);
    let place = |rt: &Runtime<State>| {
        let o = rt.state.sessions.open.as_ref().unwrap();
        o.row
            .card
            .as_ref()
            .unwrap()
            .card
            .tags
            .iter()
            .position(|t| t == tag)
    };
    let n = place(&rt).unwrap();
    press(&mut rt, &format!("session-tag-{n}"));
    run_jobs(&mut rt, &core);
    assert_eq!(rt.state.sessions.results.as_ref().unwrap().total, 1);

    rt.update(|s, _| sessions::open_row(s, 0));
    run_jobs(&mut rt, &core);
    let n = place(&rt).unwrap();
    press(&mut rt, &format!("hide-tag-{n}"));
    run_jobs(&mut rt, &core);
    assert_eq!(place(&rt), None);
    assert!(rt.state.sessions.open.as_ref().unwrap().tag_error.is_none());
}

/// Copy a folder's files, keeping its layout.
fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

#[test]
fn on_a_hub_the_list_filters_by_machine_and_names_a_spokes_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let core = archive(dir.path());
    // The same captures as a spoke's, parsed into sessions of their own.
    copy_tree(
        &core.home.raw_dir_of(None),
        &core.home.raw_dir_of(Some("nas")),
    );
    let config = core.home.load_config().unwrap();
    distill_core::tick::tick_with(&core.home, &config, &core.runtime).unwrap();
    // Opening Sessions first reads the status report, which names the spokes.
    let mut rt = window(&core);
    assert!(
        rt.find("filter-machine-trigger").is_some(),
        "{:?}",
        texts(&rt)
    );

    fn pick(rt: &mut Runtime<State>, core: &Core, m: Option<&str>) -> Vec<Option<String>> {
        rt.update(|s, _| {
            s.sessions.filters.machine = m.map(str::to_string);
            sessions::search(s);
        });
        run_jobs(rt, core);
        let r = rt.state.sessions.results.as_ref().unwrap();
        r.rows.iter().map(|r| r.facts.machine.clone()).collect()
    }
    assert_eq!(pick(&mut rt, &core, None).len(), 4);
    let nas = Some("nas".to_string());
    assert_eq!(pick(&mut rt, &core, Some("nas")), [nas.clone(), nas]);
    let all = everything(&rt);
    assert!(all.contains("nas · Claude Code · /work/"), "{all}");
    assert_eq!(
        pick(&mut rt, &core, Some(distill_core::home::HUB)),
        [None, None]
    );
}
