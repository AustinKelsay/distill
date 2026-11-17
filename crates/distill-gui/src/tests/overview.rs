//! The overview renders the status report, and nothing of any session's text.

use std::sync::Arc;

use distill_core::runtime::FakeRuntime;
use distill_core::{Config, Harness, Home};

use super::*;
use crate::fixture;
use crate::jobs::{Core, Timer};
use crate::nav::Screen;

fn on(rt: &Runtime<State>, key: &str) -> bool {
    rt.find(key).is_some()
}

/// The text of the element with `key`, or of the first text inside it.
fn text_of(rt: &Runtime<State>, key: &str) -> String {
    let els = rt.elements();
    let parts: Vec<&str> = key.split('/').collect();
    let (last, outer) = parts.split_last().unwrap();
    let at = els
        .iter()
        .position(|e| {
            e.key.as_deref() == Some(*last)
                && outer.iter().all(|o| e.path.split('/').any(|seg| seg == *o))
        })
        .unwrap_or_else(|| panic!("no {key}"));
    let path = &els[at].path;
    els[at..]
        .iter()
        .take_while(|e| e.path.starts_with(path.as_str()))
        .find_map(|e| e.text.clone())
        .unwrap_or_default()
}

#[test]
fn the_overview_shows_sessions_and_capture_health_per_harness() {
    let rt = runtime_sized(fixture::running(), (1080.0, 2400.0));
    assert!(shows(&rt, "5,529"), "total sessions: {:?}", texts(&rt));
    assert!(shows(&rt, "2.6 GB"), "archive size");
    assert_eq!(text_of(&rt, "harness-claude-code/sessions"), "4,118");
    assert_eq!(text_of(&rt, "harness-claude-code/health"), "OK");
    assert_eq!(text_of(&rt, "harness-droid/health"), "Missing");
    assert!(shows(&rt, "not found on this machine"));
    // A switched-off harness is listed apart from the ones that capture.
    assert!(!on(&rt, "harness-goose"));
    assert!(shows(&rt, "Goose"), "switched-off harnesses are named");
}

#[test]
fn the_overview_shows_the_last_sweep_and_each_steps_progress() {
    let rt = runtime_sized(fixture::running(), (1080.0, 2400.0));
    assert_eq!(text_of(&rt, "last-sweep"), "Last sweep 12 min ago");
    assert_eq!(text_of(&rt, "step-scan/value"), "5,529 of 5,529 scanned");
    assert!(shows(&rt, "37 secrets found"));
    assert_eq!(text_of(&rt, "step-index/value"), "5,490 of 5,529 indexed");
    assert!(shows(&rt, "39 waiting"));
    assert_eq!(
        text_of(&rt, "step-cards/value"),
        "1,212 of 5,300 with a card"
    );
    assert!(shows(&rt, "6 failing"));
    assert!(shows(&rt, "qwen-small:4b"));
    assert_eq!(
        text_of(&rt, "step-vectors/value"),
        "900 of 5,300 with vectors"
    );
    assert!(shows(&rt, "embed-text:v1"));
    assert_eq!(
        text_of(&rt, "step-details/value"),
        "640 of 5,529 checked by the model"
    );
    assert!(shows(&rt, "88 personal details found"));
    assert_eq!(text_of(&rt, "drift"), "No format drift");
}

#[test]
fn stale_cards_count_as_shown_and_are_named_apart_from_the_waiting() {
    let mut s = fixture::running();
    let cards = &mut s.overview.status.as_mut().unwrap().cards;
    cards.current -= 200;
    cards.stale = 200;
    let rt = runtime_sized(s, (1080.0, 2400.0));
    assert_eq!(
        text_of(&rt, "step-cards/value"),
        "1,212 of 5,300 with a card"
    );
    assert!(shows(&rt, "200 stale"));
    assert!(shows(&rt, "4,088 waiting"));
}

#[test]
fn pending_reveal_requests_are_counted_and_lead_to_approvals() {
    let mut rt = runtime_sized(fixture::running(), (1080.0, 2400.0));
    assert!(shows(&rt, "2 reveal requests wait for you"));
    press(&mut rt, "open-approvals");
    assert_eq!(rt.state.screen, Screen::Approvals);
}

#[test]
fn the_overview_shows_the_exposure_check() {
    let rt = runtime_sized(fixture::running(), (1080.0, 2400.0));
    for rule in ["owner-only", "key-owner-only", "not-synced", "no-listener"] {
        assert!(on(&rt, &format!("rule-{rule}")), "{rule}");
    }
    assert_eq!(text_of(&rt, "rule-no-listener/outcome"), "Not checked");
    assert_eq!(text_of(&rt, "rule-owner-only/outcome"), "Pass");
    assert!(shows(&rt, "The disk is encrypted."));
    assert!(shows(&rt, "Backups carry the archive"));
    assert!(!on(&rt, "fix-exposure"), "nothing to fix");
}

#[test]
fn warnings_drift_and_failing_rules_are_shown_with_a_fix() {
    let rt = runtime_sized(fixture::troubled(), (1080.0, 2600.0));
    assert!(on(&rt, "warning-0") && on(&rt, "warning-1") && on(&rt, "warning-2"));
    assert!(shows(&rt, "Nothing has swept for 7 hours"));
    assert!(shows(
        &rt,
        "Claude Code deletes sessions untouched for 30 minutes"
    ));
    assert_eq!(text_of(&rt, "warning-0/kind"), "Sweep overdue");
    assert_eq!(text_of(&rt, "warning-1/kind"), "Prunes sooner");
    assert_eq!(text_of(&rt, "warning-2/kind"), "Exposure");
    assert_eq!(text_of(&rt, "drift"), "12 captures look like format drift");
    assert_eq!(text_of(&rt, "harness-codex/drift"), "12");
    assert_eq!(text_of(&rt, "rule-owner-only/outcome"), "Fail");
    assert!(shows(&rt, "~/distill/config.toml 0644"));
    assert!(
        shows(&rt, "~/distill/exports"),
        "the previous build's folder"
    );
    assert!(
        shows(&rt, "the local runtime is not running"),
        "why cards are skipped"
    );
    assert!(on(&rt, "fix-exposure"));
}

#[test]
fn a_step_with_nothing_to_report_is_not_yet_available() {
    let rt = runtime_sized(fixture::just_set_up(), (1080.0, 2400.0));
    assert_eq!(text_of(&rt, "last-sweep"), "No sweep yet");
    for step in ["scan", "index", "cards", "vectors", "details"] {
        assert_eq!(
            text_of(&rt, &format!("step-{step}/value")),
            "Not yet available",
            "{step}"
        );
    }
    assert!(shows(&rt, "no card_model is set"));
    assert!(shows(&rt, "No reveal requests"));
    assert_eq!(text_of(&rt, "harness-claude-code/last"), "Not yet");
}

/// A temporary home with one synthetic Claude Code session, and a fake runtime.
fn temp_home(dir: &std::path::Path) -> (Core, Config) {
    let claude = dir.join("harness/claude");
    std::fs::create_dir_all(claude.join("-w")).unwrap();
    std::fs::write(
        claude.join("-w/s1.jsonl"),
        r#"{"type":"user","cwd":"/w","timestamp":"2026-11-01T00:00:00Z","message":{"role":"user","content":"zebra-marmalade-unique-words"}}
"#,
    )
    .unwrap();
    let mut config = Config::default();
    for h in &mut config.harnesses {
        h.enabled = false;
        h.roots = vec![dir.join("harness/none")];
    }
    let h = config.harness_mut(Harness::ClaudeCode);
    h.enabled = true;
    h.roots = vec![claude];
    let home = Home::new(dir.join("home"));
    home.ensure().unwrap();
    config.save(&home.config_path()).unwrap();
    let core = Core {
        home,
        runtime: FakeRuntime::not_installed().runtime(),
        timer: Timer {
            install: Arc::new(|_, _| panic!("no test installs the timer here")),
            installed: Arc::new(|| false),
        },
    };
    (core, config)
}

#[test]
fn opening_the_overview_loads_the_status_report_from_core_and_shows_no_session_text() {
    let dir = tempfile::tempdir().unwrap();
    let (core, config) = temp_home(dir.path());
    distill_core::tick::tick_with(&core.home, &config, &core.runtime).unwrap();

    let mut s = State::new(core.home.clone(), Some(config));
    s.fixture = true;
    let mut rt = runtime_sized(s, (1080.0, 2400.0));
    rt.update(|s, _| crate::nav::go(s, Screen::Overview));
    let ran = run_jobs(&mut rt, &core);
    assert_eq!(ran, ["status"]);
    settle(&mut rt);

    let want = distill_core::status::status(&core.home).unwrap();
    assert_eq!(
        rt.state.overview.status.as_ref().unwrap().sessions,
        want.sessions
    );
    assert_eq!(text_of(&rt, "harness-claude-code/sessions"), "1");
    assert_eq!(text_of(&rt, "step-scan/value"), "1 of 1 scanned");
    assert!(
        !shows(&rt, "zebra-marmalade"),
        "no session text: {:?}",
        texts(&rt)
    );
}

#[test]
fn fix_tightens_the_homes_modes_and_reloads_the_report() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (core, config) = temp_home(dir.path());
    std::fs::set_permissions(
        core.home.config_path(),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    let mut s = State::new(core.home.clone(), Some(config));
    s.fixture = true;
    let mut rt = runtime_sized(s, (1080.0, 2400.0));
    rt.update(|s, _| crate::nav::go(s, Screen::Overview));
    run_jobs(&mut rt, &core);
    settle(&mut rt);
    assert!(on(&rt, "fix-exposure"), "{:?}", texts(&rt));

    press(&mut rt, "fix-exposure");
    assert_eq!(run_jobs(&mut rt, &core), ["fix exposure", "status"]);
    settle(&mut rt);
    let mode = std::fs::metadata(core.home.config_path())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0);
    assert!(
        !on(&rt, "fix-exposure"),
        "{:?}",
        rt.state.overview.status.as_ref().unwrap().exposure
    );
    assert!(
        shows(&rt, "Made 1 file or folder owner-only"),
        "{:?}",
        texts(&rt)
    );
}
