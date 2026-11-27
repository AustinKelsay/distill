//! Setup walks through harnesses, the home folder, trusted AI, models, privacy and the
//! schedule, and Finish writes the config and installs the timer (a stand-in here: installing
//! replaces the live `dev.distill.tick` job whatever HOME is).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use distill_core::backup::{self, Place};
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
/// one, a timer stand-in that records what it was asked to install, and two backup places in
/// the temp folder (a drive and a cloud folder, neither made).
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
    let (drive, cloud) = (
        dir.path().join("Backup"),
        dir.path()
            .join("Library/CloudStorage/GoogleDrive-a@example.com/My Drive"),
    );
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
        places: Arc::new(move || {
            vec![
                Place {
                    name: "Backup".into(),
                    path: drive.clone(),
                    cloud: false,
                },
                Place {
                    name: "Google Drive".into(),
                    path: cloud.clone(),
                    cloud: true,
                },
            ]
        }),
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
    assert_eq!(
        ran,
        [
            "backup key",
            "backup places",
            "detect harnesses",
            "local runtime",
            "timer"
        ]
    );

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
            "Trusted AI",
            "Models",
            "Privacy",
            "Backups",
            "Machines",
            "Schedule"
        ]
    );
    press(&mut rt, "next");
    assert_eq!(rt.state.setup.step, 1);
    assert!(shows(&rt, "~/distill"), "the home folder");
    assert!(shows(&rt, "DISTILL_HOME"));
    press(&mut rt, "back");
    assert_eq!(rt.state.setup.step, 0);
    for _ in 0..7 {
        press(&mut rt, "next");
    }
    assert_eq!(rt.state.setup.step, 7);
    assert!(!on(&rt, "next") && on(&rt, "finish"));
}

#[test]
fn the_providers_shown_match_what_the_providers_command_reports() {
    // Tall enough that the field under the services list is on screen.
    let mut rt = runtime_sized(fixture::fresh(), (1080.0, 1400.0));
    step(&mut rt, "Trusted AI");

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
    assert!(
        shows(
            &rt,
            "run in Ollama on this machine: no session text leaves it"
        ),
        "{:?}",
        texts(&rt)
    );
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
    step(&mut rt, "Trusted AI");
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

#[test]
fn common_services_are_offered_untrusted_and_trusting_one_adds_it() {
    let mut rt = runtime(fixture::fresh());
    step(&mut rt, "Trusted AI");
    assert!(on(&rt, "service-anthropic") && on(&rt, "provider-ollama-cloud"));
    assert!(
        !on(&rt, "service-ollama-cloud"),
        "a built-in service is listed once"
    );

    press(&mut rt, "service-anthropic/switch");
    let trust = Trust::new(&rt.state.setup.draft);
    let p = trust.provider("anthropic").unwrap();
    assert!(p.trusted && p.kind == ProviderKind::Remote);
    // Now it is a provider like any other, shown once, and untrusting keeps it listed.
    assert!(on(&rt, "provider-anthropic") && !on(&rt, "service-anthropic"));
    assert!(shows(&rt, "Anthropic"));
    press(&mut rt, "provider-anthropic/switch");
    assert!(
        !Trust::new(&rt.state.setup.draft)
            .provider("anthropic")
            .unwrap()
            .trusted
    );
}

#[test]
fn privacy_says_what_is_found_automatically() {
    let mut rt = runtime(fixture::fresh());
    rt.update(|s, _| s.setup.draft.steward = Default::default());
    step(&mut rt, "Privacy");
    assert!(on(&rt, "secrets") && shows(&rt, "Always on"));
    assert!(shows(&rt, "Patterns only"), "{:?}", texts(&rt));

    // The detection pass uses the detail model, else the card model.
    rt.update(|s, _| s.setup.draft.steward.card_model = Some("qwen-small:4b".into()));
    settle(&mut rt);
    assert!(shows(&rt, "Patterns and qwen-small:4b"), "{:?}", texts(&rt));
}

#[test]
fn models_can_run_at_a_trusted_service_which_setup_writes_as_generation() {
    let t = temp(false);
    let mut rt = runtime_sized(fresh_window(&t).state, (1080.0, 1200.0));
    run_jobs(&mut rt, &t.core);
    step(&mut rt, "Trusted AI");
    press(&mut rt, "service-openrouter/switch");
    step(&mut rt, "Models");
    press(&mut rt, "card-model-trigger");
    choose(&mut rt, "qwen-small:4b");
    assert!(!on(&rt, "base-url"), "on this machine there is no address");

    // Only trusted services are offered; choosing one clears models the old runtime served.
    press(&mut rt, "runs-at-trigger");
    assert!(
        !rt.elements()
            .iter()
            .any(|e| e.label.as_deref() == Some("anthropic"))
    );
    choose(&mut rt, "openrouter");
    assert_eq!(rt.state.setup.runs_at(), Some("openrouter"));
    let per_run = rt.state.setup.draft.steward.sessions_per_run;
    assert!(
        shows(
            &rt,
            &format!("sends the text of up to {per_run} sessions to openrouter")
        ),
        "{:?}",
        texts(&rt)
    );
    assert!(shows(&rt, "personal details are not"));
    assert_eq!(rt.state.setup.draft.steward.card_model, None);
    assert!(on(&rt, "base-url") && on(&rt, "key-file"));

    // Finish needs the address and key file.
    step(&mut rt, "Schedule");
    press(&mut rt, "finish");
    assert!(
        shows(&rt, "give openrouter's address and key file"),
        "{:?}",
        texts(&rt)
    );
    assert!(!t.core.home.config_path().exists());

    // Check asks the service; a key file that can't be read says so, without any request.
    step(&mut rt, "Models");
    type_into(&mut rt, |s| &mut s.setup.base_url, "https://example.com/v1");
    let key = t.roots.join("no-such-key");
    type_into(
        &mut rt,
        |s| &mut s.setup.key_file,
        &key.display().to_string(),
    );
    press(&mut rt, "check");
    assert_eq!(run_jobs(&mut rt, &t.core), ["service models"]);
    assert!(shows(&rt, "API key file"), "{:?}", texts(&rt));

    // What the service lists is offered for cards; embeddings stay with Ollama's models.
    rt.update(|s, _| s.setup.remote_models = Some(Ok(vec!["big-remote".into()])));
    settle(&mut rt);
    assert!(shows(&rt, "openrouter serves 1 model"));
    press(&mut rt, "card-model-trigger");
    choose(&mut rt, "big-remote");
    press(&mut rt, "embedding-model-trigger");
    assert!(
        !rt.elements()
            .iter()
            .any(|e| e.role == "option" && e.label.as_deref() == Some("big-remote"))
    );
    choose(&mut rt, "embed-text:v1");

    step(&mut rt, "Schedule");
    press(&mut rt, "finish");
    run_jobs(&mut rt, &t.core);
    let saved = Config::load(&t.core.home.config_path()).unwrap();
    let g = saved
        .steward
        .generation
        .as_ref()
        .expect("generation written");
    assert_eq!(
        (
            g.provider.as_str(),
            g.base_url.as_str(),
            g.api_key_file.clone()
        ),
        ("openrouter", "https://example.com/v1", key)
    );
    assert_eq!(saved.steward.card_model.as_deref(), Some("big-remote"));
    assert_eq!(
        saved.steward.embedding_model.as_deref(),
        Some("embed-text:v1")
    );
}

#[test]
fn sessions_per_run_is_chosen_on_the_models_step_and_saved() {
    let t = temp(false);
    let mut rt = runtime_sized(fresh_window(&t).state, (1080.0, 1300.0));
    run_jobs(&mut rt, &t.core);
    step(&mut rt, "Models");
    assert_eq!(
        rt.state.setup.draft.steward.sessions_per_run, 25,
        "the default"
    );
    assert!(shows(&rt, "stop after 30 minutes"), "{:?}", texts(&rt));
    press(&mut rt, "per-run/segment2");
    assert_eq!(rt.state.setup.draft.steward.sessions_per_run, 100);

    step(&mut rt, "Schedule");
    press(&mut rt, "finish");
    run_jobs(&mut rt, &t.core);
    let saved = Config::load(&t.core.home.config_path()).unwrap();
    assert_eq!(saved.steward.sessions_per_run, 100);
}

#[test]
fn the_backups_step_adds_and_tests_folders_and_makes_a_backup_key_to_keep() {
    let t = temp(false);
    let mut s = State::new(t.core.home.clone(), None);
    s.fixture = true;
    for h in &mut s.setup.draft.harnesses {
        h.roots = vec![t.roots.join(h.kind.id())];
    }
    let mut rt = runtime_sized(s, (1080.0, 1600.0));
    rt.update(|s, _| crate::nav::go(s, Screen::Setup));
    run_jobs(&mut rt, &t.core);
    assert_eq!(rt.state.setup.has_key, Some(false));
    step(&mut rt, "Backups");

    // A folder that isn't there yet can be added; its test says so.
    let drive = t.roots.parent().unwrap().join("drive");
    type_into(&mut rt, |s| &mut s.setup.backup_name, "drive");
    type_into(
        &mut rt,
        |s| &mut s.setup.backup_path,
        drive.to_str().unwrap(),
    );
    press(&mut rt, "add-backup");
    assert_eq!(rt.state.setup.draft.backups.len(), 1);
    assert_eq!(rt.state.setup.backup_name.text, "", "the fields clear");
    type_into(&mut rt, |s| &mut s.setup.backup_name, "drive");
    type_into(&mut rt, |s| &mut s.setup.backup_path, "/elsewhere");
    press(&mut rt, "add-backup");
    assert!(shows(&rt, "already named"), "{:?}", texts(&rt));

    press(&mut rt, "backup-0/test");
    assert_eq!(run_jobs(&mut rt, &t.core), ["test backup target"]);
    assert!(shows(&rt, "does not exist"), "{:?}", texts(&rt));
    std::fs::create_dir(&drive).unwrap();
    press(&mut rt, "backup-0/test");
    run_jobs(&mut rt, &t.core);
    assert!(
        shows(&rt, "Ready. Nothing is copied until the first backup."),
        "{:?}",
        texts(&rt)
    );
    assert_eq!(
        std::fs::read_dir(&drive).unwrap().count(),
        0,
        "a test leaves nothing"
    );

    // Without the key, Finish says to make it: it backs up to each folder.
    step(&mut rt, "Schedule");
    press(&mut rt, "finish");
    assert!(
        shows(&rt, "Backups: make the backup key"),
        "{:?}",
        texts(&rt)
    );
    assert!(run_jobs(&mut rt, &t.core).is_empty(), "nothing was saved");
    step(&mut rt, "Backups");

    // Making the key writes it now and shows it, and Show brings it back.
    press(&mut rt, "make-key");
    assert_eq!(run_jobs(&mut rt, &t.core), ["make backup key"]);
    let kept = distill_core::backup::BackupKey::existing(&t.core.home)
        .unwrap()
        .expect("the key was made");
    let recovery = distill_core::backup::expose(&kept.recovery()).to_string();
    assert_eq!(rt.state.setup.recovery.as_deref(), Some(recovery.as_str()));
    assert!(shows(&rt, &recovery));
    press(&mut rt, "copy-key");
    assert!(rt.state.setup.copied);
    press(&mut rt, "hide-key");
    assert!(rt.state.setup.recovery.is_none() && !shows(&rt, &recovery));
    press(&mut rt, "show-key");
    run_jobs(&mut rt, &t.core);
    assert!(shows(&rt, &recovery));

    // Leaving the step hides it; Finish saves the folder and a spoke added on the Machines step,
    // and starts the folder's first backup, which the overview shows running until it lands.
    step(&mut rt, "Machines");
    assert!(rt.state.setup.recovery.is_none());
    type_into(&mut rt, |s| &mut s.setup.machine_name, "nas");
    type_into(&mut rt, |s| &mut s.setup.machine_host, "me@nas");
    type_into(
        &mut rt,
        |s| &mut s.setup.machine_command,
        "/opt/bin/distill",
    );
    press(&mut rt, "add-machine");
    step(&mut rt, "Schedule");
    press(&mut rt, "finish");
    let core = t.core.clone();
    rt.update(|s, _| {
        for job in std::mem::take(&mut s.outbox) {
            job.run(&core)(s);
        }
    });
    settle(&mut rt);
    let saved = t.core.home.load_config().unwrap();
    assert_eq!(saved.backups.len(), 1);
    assert_eq!(saved.backups[0].path.as_deref(), Some(drive.as_path()));
    assert_eq!(
        saved
            .machines
            .iter()
            .map(|m| (m.name.as_str(), m.host.as_str(), m.command.as_deref()))
            .collect::<Vec<_>>(),
        [("nas", "me@nas", Some("/opt/bin/distill"))]
    );
    assert_eq!(rt.state.screen, Screen::Overview);
    assert!(rt.state.overview.backing_up.contains("drive"));
    // The overview's report lands while the backup still runs.
    rt.update(|s, _| {
        let i = s.outbox.iter().position(|j| j.what == "status").unwrap();
        s.outbox.remove(i).run(&core)(s);
    });
    settle(&mut rt);
    assert_eq!(text_of(&rt, "backup-drive/state"), "First backup running…");
    assert!(
        !shows(&rt, "Back up now in the window"),
        "no warning to start it while it runs"
    );
    let ran = run_jobs(&mut rt, &t.core);
    assert!(ran.contains(&"back up"), "{ran:?}");
    assert!(
        drive.join("distill-backup").is_dir(),
        "the first backup ran"
    );
    assert_eq!(text_of(&rt, "backup-drive/state"), "Up to date");

    // Finishing again starts no backup: the timer keeps a backed-up folder current.
    press(&mut rt, "nav-setup");
    run_jobs(&mut rt, &t.core);
    step(&mut rt, "Schedule");
    press(&mut rt, "finish");
    let ran = run_jobs(&mut rt, &t.core);
    assert!(
        ran.contains(&"finish setup") && !ran.contains(&"back up"),
        "{ran:?}"
    );

    // Another folder under the backed-up folder's name would take its runs: Finish refuses it.
    press(&mut rt, "nav-setup");
    run_jobs(&mut rt, &t.core);
    step(&mut rt, "Backups");
    press(&mut rt, "backup-0/remove");
    let other = t.roots.parent().unwrap().join("other");
    type_into(&mut rt, |s| &mut s.setup.backup_name, "drive");
    type_into(
        &mut rt,
        |s| &mut s.setup.backup_path,
        other.to_str().unwrap(),
    );
    press(&mut rt, "add-backup");
    step(&mut rt, "Schedule");
    press(&mut rt, "finish");
    run_jobs(&mut rt, &t.core);
    assert!(
        shows(&rt, "was the name of another backup folder"),
        "{:?}",
        texts(&rt)
    );
    let saved = t.core.home.load_config().unwrap();
    assert_eq!(
        saved.backups[0].path.as_deref(),
        Some(drive.as_path()),
        "nothing saved"
    );
}

#[test]
fn the_backups_step_offers_the_drives_and_cloud_folders_it_finds() {
    let t = temp(false);
    let mut rt = runtime_sized(fresh_window(&t).state, (1080.0, 1600.0));
    rt.update(|s, _| crate::nav::go(s, Screen::Setup));
    run_jobs(&mut rt, &t.core);
    step(&mut rt, "Backups");
    assert_eq!(text_of(&rt, "place-0"), "Backup · drive");
    assert_eq!(text_of(&rt, "place-1"), "Google Drive · cloud folder");

    // One press adds a place under its own name (numbered when a folder already has it), and its
    // chip then says so.
    rt.update(|s, _| {
        backup::add_folder(&mut s.setup.draft, "Google Drive", "/elsewhere").unwrap();
    });
    press(&mut rt, "place-1");
    let added = &rt.state.setup.draft.backups;
    assert_eq!(added.len(), 2);
    assert_eq!(added[1].name, "Google Drive 2");
    press(&mut rt, "backup-0/remove");
    assert_eq!(text_of(&rt, "place-1"), "Google Drive · added");
    press(&mut rt, "place-1");
    assert_eq!(rt.state.setup.draft.backups.len(), 1, "added once");

    // A cloud folder says that its sync app, not Verify, sees the upload.
    assert!(
        shows(
            &rt,
            "Verify checks the copy on this machine; your sync app uploads it."
        ),
        "{:?}",
        texts(&rt)
    );
    press(&mut rt, "place-0");
    assert_eq!(rt.state.setup.draft.backups.len(), 2);
    assert!(on(&rt, "backup-0/synced") && !on(&rt, "backup-1/synced"));
}

#[test]
fn a_test_that_lands_after_its_target_changed_is_dropped() {
    let t = temp(false);
    let mut s = State::new(t.core.home.clone(), None);
    s.fixture = true;
    let mut rt = runtime_sized(s, (1080.0, 1600.0));
    rt.update(|s, _| crate::nav::go(s, Screen::Setup));
    run_jobs(&mut rt, &t.core);
    step(&mut rt, "Backups");
    let add = |rt: &mut Runtime<State>, path: &str| {
        type_into(rt, |s| &mut s.setup.backup_name, "drive");
        type_into(rt, |s| &mut s.setup.backup_path, path);
        press(rt, "add-backup");
    };
    add(&mut rt, "/nowhere/a");
    press(&mut rt, "backup-0/test");
    // Removed and added again at another folder before the test of the first lands.
    press(&mut rt, "backup-0/remove");
    add(&mut rt, "/nowhere/b");
    run_jobs(&mut rt, &t.core);
    assert!(rt.state.setup.testing.is_empty());
    assert!(
        rt.state.setup.tests.is_empty(),
        "{:?}",
        rt.state.setup.tests
    );
}

#[test]
fn the_backup_key_shows_only_on_the_backups_step_after_a_press() {
    let t = temp(false);
    distill_core::backup::BackupKey::create(&t.core.home).unwrap();
    let mut s = State::new(t.core.home.clone(), None);
    s.fixture = true;
    let mut rt = runtime_sized(s, (1080.0, 1600.0));
    rt.update(|s, _| crate::nav::go(s, Screen::Setup));
    run_jobs(&mut rt, &t.core);
    step(&mut rt, "Backups");
    press(&mut rt, "show-key");
    run_jobs(&mut rt, &t.core);
    assert!(rt.state.setup.recovery.is_some());

    // Away by the sidebar and back: hidden.
    press(&mut rt, "nav-overview");
    press(&mut rt, "nav-setup");
    assert!(rt.state.setup.recovery.is_none());
    assert_eq!(rt.state.setup.step, crate::screens::setup::BACKUPS);

    // A read that lands after the owner left is dropped.
    press(&mut rt, "show-key");
    press(&mut rt, "nav-overview");
    run_jobs(&mut rt, &t.core);
    assert!(rt.state.setup.recovery.is_none());

    // Even when they have come back to the Backups step before it lands.
    press(&mut rt, "nav-setup");
    run_jobs(&mut rt, &t.core);
    press(&mut rt, "show-key");
    press(&mut rt, "nav-overview");
    press(&mut rt, "nav-setup");
    assert_eq!(rt.state.setup.step, crate::screens::setup::BACKUPS);
    let ran = run_jobs(&mut rt, &t.core);
    assert!(ran.contains(&"show backup key"), "{ran:?}");
    assert!(rt.state.setup.recovery.is_none());
    assert!(!rt.state.setup.key_busy, "Show can be pressed again");
}

#[test]
fn a_job_that_panics_still_lands_an_error() {
    let r: Result<(), String> = crate::jobs::caught(|| panic!("boom"));
    assert_eq!(r, Err("it stopped unexpectedly: boom".into()));
    let r = crate::jobs::caught(|| anyhow::bail!("no drive"));
    assert_eq!(r, Err::<(), _>("no drive".into()));
}

#[test]
fn setup_copy_uses_scheduled_runs_while_keeping_the_literal_command() {
    let mut s = fixture::fresh();
    distill_core::backup::add_folder(&mut s.setup.draft, "drive", "/Volumes/Backup").unwrap();
    let mut rt = runtime_sized(s, (1080.0, 1400.0));
    step(&mut rt, "Backups");
    assert!(
        shows(&rt, "every scheduled run keeps it current."),
        "{:?}",
        texts(&rt)
    );

    step(&mut rt, "Schedule");
    assert!(
        shows(&rt, "How often the timer runs: sweep the harnesses"),
        "{:?}",
        texts(&rt)
    );
    assert!(
        shows(&rt, "`distill tick`"),
        "literal CLI command stays: {:?}",
        texts(&rt)
    );
}

#[test]
fn finish_refuses_two_backup_folders_of_one_name_and_saves_nothing() {
    let t = temp(false);
    let mut rt = fresh_window(&t);
    run_jobs(&mut rt, &t.core);
    distill_core::backup::BackupKey::create(&t.core.home).unwrap();
    rt.update(|s, _| {
        // As a hand-edited config.toml can have them.
        backup::add_folder(&mut s.setup.draft, "drive", "/nowhere/a").unwrap();
        let twin = s.setup.draft.backups[0].clone();
        s.setup.draft.backups.push(twin);
        s.setup.has_key = Some(true);
    });
    step(&mut rt, "Schedule");
    press(&mut rt, "finish");
    run_jobs(&mut rt, &t.core);
    assert!(
        rt.state
            .setup
            .error
            .as_deref()
            .is_some_and(|e| e.contains("drive")),
        "{:?}",
        rt.state.setup.error
    );
    assert!(!t.core.home.config_path().exists(), "nothing saved");
    assert!(t.installed.lock().unwrap().is_empty(), "no timer installed");
}

#[test]
fn a_new_backup_key_is_confirmed_shown_once_and_starts_each_folder_over() {
    use distill_core::backup::BackupKey;
    let t = temp(false);
    let home = &t.core.home;
    home.ensure().unwrap();
    let old = BackupKey::create(home).unwrap();
    let drive = home.root.parent().unwrap().join("drive");
    std::fs::create_dir(&drive).unwrap();
    let mut config = Config::default();
    backup::add_folder(&mut config, "drive", &drive.display().to_string()).unwrap();
    config.save(&home.config_path()).unwrap();
    assert!(backup::run(home, &config.backups[0]).unwrap().ok);

    let mut s = State::new(home.clone(), Some(config));
    s.fixture = true;
    let mut rt = runtime_sized(s, (1080.0, 1600.0));
    rt.update(|s, _| crate::nav::go(s, Screen::Setup));
    run_jobs(&mut rt, &t.core);
    step(&mut rt, "Backups");

    // Asked first; Cancel keeps the key.
    press(&mut rt, "new-key");
    assert!(
        shows(&rt, "every backup folder starts over"),
        "{:?}",
        texts(&rt)
    );
    press(&mut rt, "cancel-new-key");
    assert!(on(&rt, "new-key"));

    press(&mut rt, "new-key");
    press(&mut rt, "confirm-new-key");
    let ran = run_jobs(&mut rt, &t.core);
    assert!(ran.contains(&"new backup key"), "{ran:?}");
    let new = BackupKey::existing(home).unwrap().unwrap();
    let new = backup::expose(&new.recovery()).to_string();
    assert_ne!(new, backup::expose(&old.recovery()));
    assert_eq!(rt.state.setup.recovery.as_deref(), Some(new.as_str()));
    assert!(shows(&rt, "This is the new backup key"), "{:?}", texts(&rt));

    // The saved folder's backup started over under the new key.
    assert!(ran.contains(&"back up"), "{ran:?}");
    let log = distill_core::backup::run::log(home).unwrap();
    assert_eq!(
        (log.len(), log[0].ok, log[0].run, log[0].blobs_sent),
        (1, true, 0, 0),
        "{log:?}"
    );

    // A new key made while the owner left the step is not shown, but they are told to show it.
    press(&mut rt, "hide-key");
    press(&mut rt, "new-key");
    press(&mut rt, "confirm-new-key");
    press(&mut rt, "nav-overview");
    run_jobs(&mut rt, &t.core);
    press(&mut rt, "nav-setup");
    assert!(rt.state.setup.recovery.is_none());
    assert!(shows(&rt, "A new backup key was made"), "{:?}", texts(&rt));
    press(&mut rt, "show-key");
    run_jobs(&mut rt, &t.core);
    assert!(shows(&rt, "This is the new backup key"), "{:?}", texts(&rt));
}

#[test]
fn the_machines_step_adds_and_removes_a_spoke_by_name_and_ssh_host() {
    let mut rt = runtime_sized(fixture::fresh(), (1080.0, 1600.0));
    step(&mut rt, "Machines");
    assert!(shows(&rt, "No other machine yet."), "{:?}", texts(&rt));
    type_into(&mut rt, |s| &mut s.setup.machine_name, "nas");
    type_into(&mut rt, |s| &mut s.setup.machine_host, "me@nas");
    press(&mut rt, "add-machine");
    let m = &rt.state.setup.draft.machines;
    assert_eq!(
        m.iter()
            .map(|m| (m.name.as_str(), m.host.as_str()))
            .collect::<Vec<_>>(),
        [("nas", "me@nas")]
    );
    assert_eq!(rt.state.setup.machine_name.text, "", "the fields clear");
    assert!(shows(&rt, "me@nas") && on(&rt, "check"), "{:?}", texts(&rt));

    // As config.toml would refuse it: a name taken, and a host ssh would read as an option.
    type_into(&mut rt, |s| &mut s.setup.machine_name, "nas");
    type_into(&mut rt, |s| &mut s.setup.machine_host, "me@other");
    press(&mut rt, "add-machine");
    assert!(shows(&rt, "a machine is already named"), "{:?}", texts(&rt));
    type_into(&mut rt, |s| &mut s.setup.machine_name, "box");
    type_into(&mut rt, |s| &mut s.setup.machine_host, "-oProxyCommand=x");
    press(&mut rt, "add-machine");
    assert!(shows(&rt, "is not a host"), "{:?}", texts(&rt));
    type_into(&mut rt, |s| &mut s.setup.machine_host, "me@box");
    type_into(
        &mut rt,
        |s| &mut s.setup.machine_command,
        "-oProxyCommand=x",
    );
    press(&mut rt, "add-machine");
    assert!(shows(&rt, "is not a command"), "{:?}", texts(&rt));
    assert_eq!(rt.state.setup.draft.machines.len(), 1);
    // Where distill is on the spoke, when it isn't on the PATH SSH gives it.
    type_into(
        &mut rt,
        |s| &mut s.setup.machine_command,
        "/opt/bin/distill",
    );
    press(&mut rt, "add-machine");
    assert_eq!(
        rt.state.setup.draft.machines[1].command.as_deref(),
        Some("/opt/bin/distill")
    );
    assert_eq!(text_of(&rt, "machine-1/host"), "me@box · /opt/bin/distill");
    assert_eq!(text_of(&rt, "machine-0/host"), "me@nas");
    assert_eq!(rt.state.setup.machine_command.text, "", "the fields clear");

    press(&mut rt, "machine-1/remove");
    press(&mut rt, "remove");
    assert!(rt.state.setup.draft.machines.is_empty());
    assert!(shows(&rt, "No other machine yet."));
}

#[test]
fn a_check_lands_only_on_the_spoke_as_it_was_when_pressed() {
    let mut rt = runtime_sized(fixture::fresh(), (1080.0, 1600.0));
    step(&mut rt, "Machines");
    let add = |rt: &mut Runtime<State>, host: &str| {
        type_into(rt, |s| &mut s.setup.machine_name, "nas");
        type_into(rt, |s| &mut s.setup.machine_host, host);
        press(rt, "add-machine");
    };
    add(&mut rt, "me@nas");
    let first = rt.state.setup.draft.machines[0].clone();
    press(&mut rt, "check");
    assert_eq!(text_of(&rt, "machine-0/check"), "Checking…");

    // Removed while it is checked, and added again at the right host: it can be checked at once.
    press(&mut rt, "remove");
    add(&mut rt, "me@nas.lan");
    assert_eq!(text_of(&rt, "machine-0/check"), "Check");
    press(&mut rt, "check");
    assert_eq!(text_of(&rt, "machine-0/check"), "Checking…");
    let second = rt.state.setup.draft.machines[0].clone();
    // The checks would run ssh; their results are handed in as they would land.
    let asked = std::mem::take(&mut rt.state.outbox);
    assert_eq!(
        asked.iter().map(|j| j.what).collect::<Vec<_>>(),
        ["check machine", "check machine"]
    );
    let land = |rt: &mut Runtime<State>, m: &distill_core::config::MachineConfig, r| {
        rt.update(|s, _| crate::screens::setup::land_check(s, m, r));
        settle(rt);
    };
    land(&mut rt, &first, Ok("me@nas answers as nas".into()));
    assert!(!on(&rt, "checked"), "the first spoke's result is dropped");
    assert_eq!(text_of(&rt, "machine-0/check"), "Checking…");
    land(&mut rt, &second, Err("ssh me@nas.lan: refused".into()));
    assert_eq!(text_of(&rt, "machine-0/checked"), "ssh me@nas.lan: refused");
    assert_eq!(text_of(&rt, "machine-0/check"), "Check");
}
