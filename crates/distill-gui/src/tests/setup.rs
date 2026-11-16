//! Setup walks through harnesses, the home folder, providers, models, personal terms and the
//! schedule, and Finish writes the config and installs the timer (a stand-in here: installing
//! replaces the live `dev.distill.tick` job whatever HOME is).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use distill_core::config::ProviderKind;
use distill_core::runtime::{FakeRuntime, Stored};
use distill_core::trust::Trust;
use distill_core::{Config, Harness, Home};

use super::*;
use crate::fixture;
use crate::jobs::{Core, Timer};
use crate::nav::Screen;
use crate::screens::setup::STEPS;

fn on(rt: &Runtime<State>, key: &str) -> bool {
    rt.find(key).is_some()
}

fn step(rt: &mut Runtime<State>, name: &str) {
    let i = STEPS.iter().position(|(n, _)| *n == name).unwrap();
    press(rt, &format!("step-{i}"));
    assert_eq!(rt.state.setup.step, i);
}

/// Press the open menu's option labelled `label`.
fn choose(rt: &mut Runtime<State>, label: &str) {
    let e = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "option" && e.label.as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no option {label:?}: {:?}", texts(rt)));
    let (x, y) = (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn type_into(
    rt: &mut Runtime<State>,
    field: impl FnOnce(&mut State) -> &mut rune::TextEdit,
    s: &str,
) {
    rt.update(|st, _| field(st).replace_text(s));
    settle(rt);
}

/// A temporary home and harness folders, the fake runtime with two local models and a cloud
/// one, and a timer stand-in that records what it was asked to install.
struct Temp {
    _dir: tempfile::TempDir,
    core: Core,
    roots: PathBuf,
    installed: Arc<Mutex<Vec<(PathBuf, Config)>>>,
}

fn temp(timer_fails: bool) -> Temp {
    let dir = tempfile::tempdir().unwrap();
    let roots = dir.path().join("harnesses");
    std::fs::create_dir_all(roots.join("claude-code/-w")).unwrap();
    std::fs::write(roots.join("claude-code/-w/a.jsonl"), "{}\n").unwrap();
    let installed = Arc::new(Mutex::new(Vec::new()));
    let log = installed.clone();
    let fake = FakeRuntime::running()
        .with_model("qwen-small:4b", Stored::Here)
        .with_model("embed-text:v1", Stored::Here)
        .with_model("big-model:cloud", Stored::Cloud);
    let core = Core {
        home: Home::new(dir.path().join("home")),
        runtime: fake.runtime(),
        timer: Timer {
            install: Arc::new(move |home: &Home, config: &Config| {
                if timer_fails {
                    anyhow::bail!("launchctl bootstrap failed");
                }
                log.lock()
                    .unwrap()
                    .push((home.root.clone(), config.clone()));
                Ok(PathBuf::from("/stand-in/dev.distill.tick.plist"))
            }),
            installed: Arc::new(|| false),
        },
    };
    Temp {
        _dir: dir,
        core,
        roots,
        installed,
    }
}

/// A window over the temp home with nothing set up, its harness roots in the temp folder.
fn fresh_window(t: &Temp) -> Runtime<State> {
    let mut s = State::new(t.core.home.clone(), None);
    s.fixture = true;
    for h in &mut s.setup.draft.harnesses {
        h.roots = vec![t.roots.join(h.kind.id())];
    }
    let mut rt = runtime(s);
    rt.update(|s, _| crate::nav::go(s, Screen::Setup));
    rt
}

#[test]
fn opening_setup_detects_harnesses_the_local_models_and_the_timer() {
    let t = temp(false);
    let mut rt = fresh_window(&t);
    let mut ran = run_jobs(&mut rt, &t.core);
    ran.sort();
    assert_eq!(ran, ["detect harnesses", "local runtime", "timer"]);

    // Detected harnesses are ticked; the rest are not.
    assert!(rt.state.setup.draft.enabled(Harness::ClaudeCode));
    assert!(!rt.state.setup.draft.enabled(Harness::Codex));
    assert!(shows(&rt, "not found"));
    assert_eq!(rt.state.setup.timer_installed, Some(false));
    step(&mut rt, "Models");
    press(&mut rt, "card-model-trigger");
    assert!(shows(&rt, "qwen-small:4b") && shows(&rt, "embed-text:v1"));
    assert!(
        !shows(&rt, "big-model:cloud"),
        "a model served from a cloud is not offered"
    );
}

#[test]
fn harnesses_are_ticked_when_detected_and_unticked_to_skip() {
    let mut rt = runtime(fixture::fresh());
    assert!(rt.state.setup.draft.enabled(Harness::ClaudeCode));
    assert!(!rt.state.setup.draft.enabled(Harness::Droid), "not found");
    assert!(shows(&rt, "4,212 files"));
    press(&mut rt, "harness-codex/switch");
    assert!(!rt.state.setup.draft.enabled(Harness::Codex));
    press(&mut rt, "harness-codex/switch");
    assert!(rt.state.setup.draft.enabled(Harness::Codex));
}

#[test]
fn next_and_back_walk_the_steps_and_the_last_offers_finish() {
    let mut rt = runtime(fixture::fresh());
    let names: Vec<&str> = STEPS.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        names,
        [
            "Harnesses",
            "Home folder",
            "Providers",
            "Models",
            "Privacy",
            "Schedule"
        ]
    );
    press(&mut rt, "next");
    assert_eq!(rt.state.setup.step, 1);
    assert!(shows(&rt, "~/distill"), "the home folder");
    assert!(shows(&rt, "DISTILL_HOME"));
    press(&mut rt, "back");
    assert_eq!(rt.state.setup.step, 0);
    for _ in 0..5 {
        press(&mut rt, "next");
    }
    assert_eq!(rt.state.setup.step, 5);
    assert!(!on(&rt, "next") && on(&rt, "finish"));
}

#[test]
fn the_providers_shown_match_what_the_providers_command_reports() {
    let mut rt = runtime(fixture::fresh());
    step(&mut rt, "Providers");

    // The local provider is shown as trusted; add a remote one, then trust and untrust it.
    assert_eq!(rt.state.setup.draft.providers, vec![]);
    type_into(&mut rt, |s| &mut s.setup.provider, "Example-Cloud");
    press(&mut rt, "add-provider");
    assert_eq!(
        rt.state.setup.provider.text, "",
        "the field clears after Add"
    );
    press(&mut rt, "provider-example-cloud/switch");
    let trust = Trust::new(&rt.state.setup.draft);
    let p = trust.provider("example-cloud").unwrap();
    assert!(p.trusted && p.kind == ProviderKind::Remote);

    // A name the command would refuse is refused here too, with its reason.
    type_into(&mut rt, |s| &mut s.setup.provider, "ollama");
    press(&mut rt, "add-provider");
    assert!(
        shows(&rt, "built in as the local provider"),
        "{:?}",
        texts(&rt)
    );

    // Every provider `distill providers` lists, with its kind and trust, and the local models.
    let fake = FakeRuntime::running()
        .with_model("qwen-small:4b", Stored::Here)
        .with_model("embed-text:v1", Stored::Here)
        .with_model("big-model:cloud", Stored::Cloud);
    let listing = trust.listing(&fake.runtime());
    let shown: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with("provider-")))
        .filter_map(|e| e.label)
        .collect();
    let want: Vec<String> = listing
        .providers
        .iter()
        .map(|l| {
            format!(
                "{} {} {}",
                l.provider.name,
                match l.provider.kind {
                    ProviderKind::Local => "local",
                    ProviderKind::Remote => "remote",
                },
                if l.provider.trusted {
                    "trusted"
                } else {
                    "not trusted"
                }
            )
        })
        .collect();
    assert_eq!(shown, want);
    assert!(shows(&rt, "stored on this machine"));
    assert!(shows(&rt, "served from a cloud (reads as ollama-cloud)"));

    press(&mut rt, "provider-example-cloud/switch");
    assert!(
        !Trust::new(&rt.state.setup.draft)
            .provider("example-cloud")
            .unwrap()
            .trusted
    );
}

#[test]
fn models_are_chosen_from_the_local_runtimes_installed_models() {
    let mut rt = runtime(fixture::fresh());
    step(&mut rt, "Models");
    press(&mut rt, "card-model-trigger");
    choose(&mut rt, "qwen-small:4b");
    press(&mut rt, "embedding-model-trigger");
    choose(&mut rt, "embed-text:v1");
    let st = &rt.state.setup.draft.steward;
    assert_eq!(st.card_model.as_deref(), Some("qwen-small:4b"));
    assert_eq!(st.embedding_model.as_deref(), Some("embed-text:v1"));
    assert_eq!(
        st.detail_model, None,
        "the detection pass follows the card model"
    );
    assert_eq!(st.detail_model(), Some("qwen-small:4b"));

    press(&mut rt, "detail-model-trigger");
    choose(&mut rt, "embed-text:v1");
    assert_eq!(
        rt.state.setup.draft.steward.detail_model.as_deref(),
        Some("embed-text:v1")
    );
    press(&mut rt, "card-model-trigger");
    choose(&mut rt, "Not set");
    assert_eq!(rt.state.setup.draft.steward.card_model, None);
}

#[test]
fn a_stopped_runtime_says_so_and_offers_no_models() {
    let mut s = fixture::fresh();
    s.setup.runtime = Some(distill_core::runtime::RuntimeState::NotRunning);
    let mut rt = runtime(s);
    step(&mut rt, "Models");
    assert!(shows(&rt, "not running"), "{:?}", texts(&rt));
}

#[test]
fn personal_terms_and_excluded_paths_are_added_and_removed() {
    let mut rt = runtime(fixture::fresh());
    step(&mut rt, "Privacy");
    type_into(&mut rt, |s| &mut s.setup.term, "  Northwind ");
    press(&mut rt, "add-term");
    type_into(&mut rt, |s| &mut s.setup.term, "Jo Example");
    press(&mut rt, "add-term");
    assert_eq!(
        rt.state.setup.draft.privacy.personal_terms,
        ["Northwind", "Jo Example"]
    );
    press(&mut rt, "term-0/remove");
    assert_eq!(rt.state.setup.draft.privacy.personal_terms, ["Jo Example"]);

    type_into(&mut rt, |s| &mut s.setup.exclude, "/work/client-x");
    press(&mut rt, "add-exclude");
    assert_eq!(
        rt.state.setup.draft.privacy.exclude_paths,
        ["/work/client-x"]
    );
}

#[test]
fn finishing_setup_writes_the_config_and_installs_the_timer_then_shows_the_overview() {
    let t = temp(false);
    let mut rt = fresh_window(&t);
    run_jobs(&mut rt, &t.core);

    press(&mut rt, "harness-claude-code/switch");
    press(&mut rt, "harness-claude-code/switch");
    step(&mut rt, "Providers");
    type_into(&mut rt, |s| &mut s.setup.provider, "example-cloud");
    press(&mut rt, "add-provider");
    press(&mut rt, "provider-example-cloud/switch");
    step(&mut rt, "Models");
    press(&mut rt, "card-model-trigger");
    choose(&mut rt, "qwen-small:4b");
    step(&mut rt, "Privacy");
    type_into(&mut rt, |s| &mut s.setup.term, "Northwind");
    press(&mut rt, "add-term");
    step(&mut rt, "Schedule");
    press(&mut rt, "sweep/segment1");
    assert_eq!(rt.state.setup.draft.schedule.sweep_every_minutes, 30);

    assert!(
        rt.state.outbox.is_empty(),
        "nothing is written before Finish"
    );
    assert!(!t.core.home.config_path().exists());
    press(&mut rt, "finish");
    assert_eq!(run_jobs(&mut rt, &t.core), ["finish setup", "status"]);

    let text = std::fs::read_to_string(t.core.home.config_path()).unwrap();
    for name in [
        "[[harnesses]]",
        "[privacy]",
        "personal_terms",
        "[[providers]]",
        "[steward]",
        "card_model",
        "[schedule]",
    ] {
        assert!(text.contains(name), "{name} in {text}");
    }
    let saved = Config::load(&t.core.home.config_path()).unwrap();
    assert_eq!(saved.privacy.personal_terms, ["Northwind"]);
    assert_eq!(saved.steward.card_model.as_deref(), Some("qwen-small:4b"));
    assert_eq!(saved.schedule.sweep_every_minutes, 30);
    assert!(
        Trust::new(&saved)
            .provider("example-cloud")
            .unwrap()
            .trusted
    );
    assert!(saved.enabled(Harness::ClaudeCode) && !saved.enabled(Harness::Codex));

    let installed = t.installed.lock().unwrap().clone();
    assert_eq!(installed, vec![(t.core.home.root.clone(), saved.clone())]);
    assert_eq!(rt.state.config, Some(saved));
    assert_eq!(rt.state.screen, Screen::Overview);
    assert!(rt.state.overview.status.as_ref().unwrap().configured);
}

#[test]
fn a_timer_that_fails_to_install_is_reported_on_setup() {
    let t = temp(true);
    let mut rt = fresh_window(&t);
    run_jobs(&mut rt, &t.core);
    step(&mut rt, "Schedule");
    press(&mut rt, "finish");
    run_jobs(&mut rt, &t.core);
    assert_eq!(rt.state.screen, Screen::Setup);
    assert!(shows(&rt, "launchctl bootstrap failed"), "{:?}", texts(&rt));
    assert!(
        t.core.home.config_path().exists(),
        "the config is written before the timer"
    );
}
