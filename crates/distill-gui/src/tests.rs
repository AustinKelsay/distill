//! Headless runs of the real view over fixture state: no window, no disk.

use distill_core::config::{Config, Harness};
use distill_core::source::SourceKind;
use rune::{Env, Runtime};

use crate::data::{Reply, Request};
use crate::{Screen, State, fixture, view};

const SIZE: (f32, f32) = (1040.0, 760.0);

fn runtime(s: State) -> Runtime<State> {
    let mut rt = Runtime::new(s, view, SIZE, Env::new(true, 1.0));
    rt.frame(0.0);
    rt
}

fn settle(rt: &mut Runtime<State>) {
    for _ in 0..20 {
        rt.frame(1.0 / 60.0);
    }
}

fn press(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = rt
        .find(key)
        .unwrap_or_else(|| panic!("no element {key:?} on screen"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn texts(rt: &Runtime<State>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter_map(|e| e.text.or(e.label))
        .collect()
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    texts(rt).iter().any(|t| t.contains(needle))
}

#[test]
fn setup_steps_through_and_finish_queues_the_chosen_config() {
    let mut s = fixture::state(None);
    s.detections = Some(fixture::detections());
    let mut rt = runtime(s);
    assert_eq!(rt.state.screen, Screen::Setup);
    assert!(
        shows(&rt, "no parser yet, archived only"),
        "Cursor says it is archived only"
    );
    assert!(shows(&rt, "not found"), "Droid is missing in the fixture");

    // Step 1: turn Cursor off.
    press(&mut rt, "source-cursor/switch");
    assert!(!rt.state.setup.enabled(SourceKind::Cursor));

    // Next and Back move one step; Back stops at the first.
    press(&mut rt, "next");
    assert_eq!(rt.state.setup.step, 1);
    assert!(shows(&rt, "~/distill"), "storage shows the home path");
    press(&mut rt, "back");
    assert_eq!(rt.state.setup.step, 0);
    press(&mut rt, "next");
    press(&mut rt, "next");
    assert_eq!(rt.state.setup.step, 2);

    // Step 3: Codex, a model, a $5 budget. The Claude default model doesn't follow to Codex.
    press(&mut rt, "harness/segment1");
    assert_eq!(rt.state.setup.config.steward.harness, Harness::Codex);
    assert_eq!(rt.state.setup.model.text, "");
    rt.update(|s, _| s.setup.model.replace_text("gpt-5-codex"));
    press(&mut rt, "budget/segment3");
    assert_eq!(rt.state.setup.config.steward.daily_budget_usd, 5.0);

    // Step 4: exclude a path, turn redaction off.
    press(&mut rt, "next");
    assert_eq!(rt.state.setup.step, 3);
    rt.update(|s, _| s.setup.exclude.replace_text("  /work/secret  "));
    settle(&mut rt);
    press(&mut rt, "add-exclude");
    assert_eq!(
        rt.state.setup.config.privacy.exclude_paths,
        vec!["/work/secret".to_string()]
    );
    assert_eq!(
        rt.state.setup.exclude.text, "",
        "the field clears after Add"
    );
    press(&mut rt, "redact");
    assert!(!rt.state.setup.config.privacy.redact_exports);

    // Step 5: sweep every 30 minutes, enrich every 6 hours, then Finish.
    press(&mut rt, "next");
    assert_eq!(rt.state.setup.step, 4);
    assert!(
        rt.find("next").is_none() && rt.find("finish").is_some(),
        "the last step offers Finish"
    );
    press(&mut rt, "sweep/segment1");
    press(&mut rt, "enrich/segment0");
    assert!(
        rt.state.outbox.is_empty(),
        "nothing is written before Finish"
    );
    press(&mut rt, "finish");

    let mut want = Config::default();
    want.source_mut(SourceKind::Cursor).enabled = false;
    want.steward.harness = Harness::Codex;
    want.steward.model = Some("gpt-5-codex".to_string());
    want.steward.daily_budget_usd = 5.0;
    want.privacy.exclude_paths = vec!["/work/secret".to_string()];
    want.privacy.redact_exports = false;
    want.schedule.sweep_every_minutes = 30;
    want.schedule.enrich_every_hours = 6;
    assert_eq!(
        rt.state.outbox,
        vec![Request::SaveConfig(Box::new(want.clone()))]
    );
    assert_eq!(rt.state.config, want);
    assert_eq!(rt.state.screen, Screen::Dashboard);

    // The save landing asks for the dashboard.
    rt.state.outbox.clear();
    rt.state.land(Reply::Saved(Ok(())));
    assert!(rt.state.outbox.contains(&Request::LoadDashboard));
}

#[test]
fn rail_jumps_to_a_step() {
    let mut rt = runtime(fixture::state(None));
    press(&mut rt, "rail-4");
    assert_eq!(rt.state.setup.step, 4);
    press(&mut rt, "rail-2");
    assert_eq!(rt.state.setup.step, 2);
}

#[test]
fn dashboard_shows_tiles_and_approve_queues_the_build_once() {
    let mut s = fixture::state(Some(Config::default()));
    fixture::dashboard(&mut s);
    let sessions = s.dash.data.as_ref().unwrap().totals.sessions;
    let mut rt = runtime(s);

    for key in [
        "tile-sessions",
        "tile-archive",
        "tile-enriched",
        "tile-datasets",
        "chart",
        "sources",
        "runs",
        "needs-you",
    ] {
        assert!(rt.find(key).is_some(), "{key} is on screen");
    }
    assert!(shows(&rt, &crate::data::thousands(sessions)));
    assert!(shows(&rt, "2.6 GB"));
    assert!(shows(&rt, "83%"));
    assert!(shows(&rt, "Last sweep 12 min ago"));
    assert!(
        rt.find("week-11/stack").is_some(),
        "the newest week has a bar"
    );
    assert!(
        rt.find("run-5").is_some() && rt.find("run-6").is_none(),
        "six newest runs"
    );

    press(&mut rt, "approve-0");
    let want = Request::Approve {
        recipe: "agentic-rust".to_string(),
        build: "20261113-1402".to_string(),
    };
    assert_eq!(rt.state.outbox, vec![want.clone()]);
    assert!(shows(&rt, "Approving…"));

    // A second press while it is on its way does nothing.
    press(&mut rt, "approve-0");
    assert_eq!(rt.state.outbox, vec![want]);

    rt.update(|s, _| {
        s.outbox.clear();
        s.land(Reply::Approved {
            recipe: "agentic-rust".to_string(),
            build: "20261113-1402".to_string(),
            result: Ok(()),
        });
    });
    assert!(rt.state.dash.approving.is_empty());
    assert_eq!(rt.state.outbox, vec![Request::LoadDashboard]);
    settle(&mut rt);
    assert!(shows(&rt, "Approved \"agentic-rust\""));
}

#[test]
fn empty_dashboard_says_so() {
    let mut s = fixture::state(Some(Config::default()));
    fixture::dashboard_empty(&mut s);
    let rt = runtime(s);
    for key in ["chart-empty", "runs-empty", "needs-empty"] {
        assert!(rt.find(key).is_some(), "{key} is on screen");
    }
    assert!(shows(&rt, "No sweep yet"));
}

#[test]
fn settings_opens_setup_over_the_saved_config_and_cancel_returns() {
    let mut s = fixture::state(Some(Config::default()));
    fixture::dashboard(&mut s);
    s.config.schedule.sweep_every_minutes = 15;
    let mut rt = runtime(s);
    press(&mut rt, "settings");
    assert_eq!(rt.state.screen, Screen::Setup);
    assert_eq!(rt.state.setup.config.schedule.sweep_every_minutes, 15);
    press(&mut rt, "cancel");
    assert_eq!(rt.state.screen, Screen::Dashboard);
    assert!(rt.state.outbox.is_empty(), "cancel writes nothing");
}
