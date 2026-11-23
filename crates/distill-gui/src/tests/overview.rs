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
    // Each step's meter says how much is done, and reads 100% only when all of it is.
    assert_eq!(text_of(&rt, "step-scan/percent"), "100%");
    assert_eq!(text_of(&rt, "step-index/percent"), "99%");
    assert_eq!(text_of(&rt, "step-cards/percent"), "22%");
    assert!(shows(&rt, "39 waiting"));
    assert_eq!(
        text_of(&rt, "step-cards/value"),
        "1,212 of 5,300 with a card"
    );
    assert!(shows(&rt, "6 to retry"));
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
fn captures_holding_record_types_the_parser_does_not_know_count_as_drift() {
    let mut s = fixture::running();
    let st = s.overview.status.as_mut().expect("a status");
    for h in &mut st.harnesses {
        if h.harness == Harness::Codex {
            h.health = distill_core::status::Health::Warn;
            h.partial_drift = 3;
            h.note = Some("record types the parser doesn't know: response_item:v2".into());
        }
    }
    let rt = runtime_sized(s, (1080.0, 2400.0));
    assert_eq!(text_of(&rt, "drift"), "3 captures look like format drift");
    assert_eq!(text_of(&rt, "harness-codex/drift"), "3");
    assert!(shows(
        &rt,
        "record types the parser doesn't know: response_item:v2"
    ));
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
    assert!(shows(
        &rt,
        "No runs yet: the timer's first scheduled run will show here."
    ));
    assert_eq!(text_of(&rt, "harness-claude-code/last"), "Not yet");
}

#[test]
fn recent_runs_fold_alike_runs_and_name_the_steps_that_did_not_simply_finish() {
    let rt = runtime_sized(fixture::running(), (1080.0, 2400.0));
    assert_eq!(text_of(&rt, "runs/cadence"), "Hourly · sweep to models");
    assert_eq!(text_of(&rt, "runs/when"), "40 min ago");
    assert!(!shows(&rt, "Tick"), "no bare Tick rows: {:?}", texts(&rt));
    assert!(
        on(&rt, "run-0") && on(&rt, "run-1") && on(&rt, "run-2"),
        "indexed run row keys"
    );
    assert!(
        on(&rt, "run-2/failure-0") && on(&rt, "run-2/failure-1"),
        "indexed failure keys"
    );
    assert!(
        !rt.elements().iter().any(|e| {
            e.path.split('/').any(|seg| seg == "runs")
                && e.key.as_deref().is_some_and(|k| k.contains("#dup"))
        }),
        "no duplicate sibling keys under runs"
    );
    assert!(
        shows(&rt, "6 h ago to 1 h ago") && shows(&rt, "6 scheduled runs"),
        "{:?}",
        texts(&rt)
    );
    assert!(shows(&rt, "cards: time limit") && shows(&rt, "details: failed"));
    assert!(shows(&rt, "cards failed: the provider did not answer"));
    assert!(shows(&rt, "All steps OK") && shows(&rt, "Sweep on its own"));
}

#[test]
fn recent_runs_collapse_same_relative_time_and_format_manual_groups_and_cadence() {
    let mut s = fixture::running();
    s.config.as_mut().unwrap().schedule.sweep_every_minutes = 15;
    s.overview.status.as_mut().unwrap().runs = serde_json::from_value(serde_json::json!([
        {
            "newest": fixture::at(60 * 26),
            "oldest": fixture::at(60 * 30),
            "runs": 3,
            "step": "tick",
            "steps": [{ "name": "sweep", "result": "ok" }]
        },
        {
            "newest": fixture::at(60 * 48),
            "oldest": fixture::at(60 * 72),
            "runs": 2,
            "step": "sweep",
            "steps": [{ "name": "sweep", "result": "ok" }]
        }
    ]))
    .unwrap();
    let rt = runtime_sized(s, (1080.0, 2400.0));
    assert_eq!(
        text_of(&rt, "runs/cadence"),
        "Every 15 min · sweep to models"
    );
    assert_eq!(text_of(&rt, "runs/when"), "yesterday");
    assert!(shows(&rt, "3 scheduled runs"), "{:?}", texts(&rt));
    assert!(shows(&rt, "Sweep on its own · 2 runs"), "{:?}", texts(&rt));

    let mut s_hours = fixture::running();
    s_hours
        .config
        .as_mut()
        .unwrap()
        .schedule
        .sweep_every_minutes = 120;
    let rt_hours = runtime_sized(s_hours, (1080.0, 2400.0));
    assert_eq!(
        text_of(&rt_hours, "runs/cadence"),
        "Every 2 h · sweep to models"
    );
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
        on(&rt, "run-0/when"),
        "run timestamp shown: {:?}",
        texts(&rt)
    );
    assert!(
        !on(&rt, "run-0/what"),
        "single tick needs no caption: {:?}",
        texts(&rt)
    );
    assert!(
        shows(&rt, "cards: skipped"),
        "no model runs: {:?}",
        texts(&rt)
    );
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

#[test]
fn back_up_now_starts_a_target_then_verify_checks_it_and_the_report_says_it_is_current() {
    let dir = tempfile::tempdir().unwrap();
    let (core, mut config) = temp_home(dir.path());
    distill_core::tick::tick_with(&core.home, &config, &core.runtime).unwrap();
    let drive = dir.path().join("drive");
    std::fs::create_dir(&drive).unwrap();
    distill_core::backup::add_folder(&mut config, "drive", drive.to_str().unwrap()).unwrap();
    config.save(&core.home.config_path()).unwrap();
    distill_core::backup::BackupKey::create(&core.home).unwrap();

    let mut s = State::new(core.home.clone(), Some(config));
    s.fixture = true;
    let mut rt = runtime_sized(s, (1080.0, 2400.0));
    rt.update(|s, _| crate::nav::go(s, Screen::Overview));
    run_jobs(&mut rt, &core);
    assert!(
        shows(&rt, "Never backed up"),
        "a target with no run: {:?}",
        texts(&rt)
    );
    assert!(
        shows(&rt, "Back up now in the window"),
        "the warning names the button"
    );

    press(&mut rt, "backup-drive/back-up");
    assert_eq!(run_jobs(&mut rt, &core), ["back up", "status"]);
    assert!(
        text_of(&rt, "backup-drive/result").starts_with("Backed up to drive (run 0)"),
        "{:?}",
        texts(&rt)
    );
    assert!(shows(&rt, "Up to date"), "{:?}", texts(&rt));
    assert!(drive.join("distill-backup").is_dir());

    press(&mut rt, "backup-drive/verify");
    assert_eq!(run_jobs(&mut rt, &core), ["verify backup"]);
    assert!(
        text_of(&rt, "backup-drive/result").contains("is whole"),
        "{:?}",
        texts(&rt)
    );

    // The drive goes away: Back up now says why, and the report shows the failure.
    std::fs::rename(&drive, dir.path().join("away")).unwrap();
    press(&mut rt, "backup-drive/back-up");
    run_jobs(&mut rt, &core);
    assert!(
        text_of(&rt, "backup-drive/result").contains("does not exist"),
        "{:?}",
        texts(&rt)
    );
    assert!(shows(&rt, "Last run failed"), "{:?}", texts(&rt));
}

#[test]
fn with_no_backup_target_the_overview_leads_to_setups_backups_step() {
    let mut rt = runtime_sized(fixture::just_set_up(), (1080.0, 2400.0));
    assert!(shows(&rt, "this archive is the only copy"));
    press(&mut rt, "set-up-backups");
    assert_eq!(rt.state.screen, Screen::Setup);
    assert_eq!(
        rt.state.setup.step,
        crate::screens::setup::BACKUPS,
        "{:?}",
        texts(&rt)
    );
    assert!(shows(&rt, "Backup folders"));
}

#[test]
fn the_overview_shows_where_each_backup_target_stands() {
    let rt = runtime_sized(fixture::troubled(), (1080.0, 2400.0));
    assert_eq!(text_of(&rt, "backup-drive/state"), "Up to date");
    assert_eq!(text_of(&rt, "backup-share/state"), "Last run failed");
    assert!(shows(&rt, "/Volumes/Share does not exist"));
    let rt = runtime_sized(fixture::running(), (1080.0, 2400.0));
    assert_eq!(text_of(&rt, "backup-share/state"), "212 captures behind");
    assert!(shows(&rt, "/Volumes/Backup"), "the target's folder");
}
