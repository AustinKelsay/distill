//! A tick: one scheduled run of the steward's steps, in the order [`STEPS`] declares them. Each
//! tick writes one entry to the run log. Today the steps are sweep, parse, scan, the word index,
//! cards, vectors and the model detection pass (`details`), which stays last. A step that calls a
//! model gets the runtime for that step, and the steps before it never wait on one.
//!
//! The model steps share one time budget per tick for starting work, [`model_budget`], counted
//! from the tick's start, so a slow or stuck model holds a tick past it by at most one model call
//! (issue #100). Each model
//! step gets an even share of what is left of it when the step starts, so time a step does not use
//! passes to the ones after it, and stops taking sessions at its [`Deadline`]. The sessions it did
//! not reach stay pending for the next tick and are not failures.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::cards;
use crate::config::{Config, GenerationConfig, ProviderKind};
use crate::details;
use crate::findings;
use crate::home::Home;
use crate::index;
use crate::normalize;
use crate::runlog::{self, RunEntry, StepOutcome};
use crate::runtime::LocalRuntime;
use crate::schedule;
use crate::sweep;
use crate::trust::Trust;

/// When a model step stops taking sessions, and when the tick's time is over. The session under
/// way when the step's share passes is finished, since cutting it off would start it over next
/// tick. Only once the tick's whole budget is over does the model detection pass stop between a
/// session's parts, recording the session as too long for one tick so it goes behind the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadline {
    start_by: Option<Instant>,
    finish_by: Option<Instant>,
}

impl Deadline {
    /// No deadline: the step takes up to `sessions_per_run` sessions however long they take.
    pub const NONE: Deadline = Deadline {
        start_by: None,
        finish_by: None,
    };

    /// No session is started, and none is carried on, `budget` from now.
    pub fn after(budget: Duration) -> Self {
        Self::between(budget, budget)
    }

    /// No session is started `start` from now, and none is carried on `finish` from now.
    pub fn between(start: Duration, finish: Duration) -> Self {
        let now = Instant::now();
        Deadline {
            start_by: now.checked_add(start),
            finish_by: now.checked_add(finish.max(start)),
        }
    }

    /// The step takes no more sessions.
    pub fn passed(&self) -> bool {
        self.start_by.is_some_and(|at| Instant::now() >= at)
    }

    /// The tick's budget is over: work under way stops where it can.
    pub fn over(&self) -> bool {
        self.finish_by.is_some_and(|at| Instant::now() >= at)
    }
}

/// How long the model steps of one tick may run, counted from the tick's start: half the timer's
/// interval. Each model call is bounded by the runtime's own timeout, so the model steps end
/// within the budget and one call, well before the timer's next tick on the hourly default.
pub fn model_budget(config: &Config) -> Duration {
    Duration::from_secs(u64::from(schedule::interval_minutes(config)) * 60 / 2)
}

/// One step of a tick.
pub struct Step {
    pub name: &'static str,
    /// The step calls a model, so it gets a share of the tick's [`model_budget`].
    pub model: bool,
    generation: bool,
    pub run: fn(&Home, &Config, &LocalRuntime, Deadline) -> Result<StepOutcome>,
}

/// The tick's steps, in order. Capture comes first and never waits on a model.
pub const STEPS: &[Step] = &[
    Step {
        name: "sweep",
        model: false,
        generation: false,
        run: sweep_step,
    },
    Step {
        name: "parse",
        model: false,
        generation: false,
        run: parse_step,
    },
    Step {
        name: "scan",
        model: false,
        generation: false,
        run: scan_step,
    },
    Step {
        name: "index",
        model: false,
        generation: false,
        run: index_step,
    },
    Step {
        name: "cards",
        model: true,
        generation: true,
        run: cards_step,
    },
    Step {
        name: "vectors",
        model: true,
        generation: false,
        run: vectors_step,
    },
    Step {
        name: "details",
        model: true,
        generation: true,
        run: details_step,
    },
];

/// Copy new and changed harness files into the raw archive.
pub fn sweep_step(
    home: &Home,
    config: &Config,
    _: &LocalRuntime,
    _: Deadline,
) -> Result<StepOutcome> {
    let r = sweep::sweep(home, config, None)?;
    Ok(StepOutcome {
        name: "sweep".into(),
        ok: r.errors.is_empty(),
        summary: r.summary(),
        errors: r.errors.clone(),
        report: serde_json::to_value(&r)?,
    })
}

/// Turn captures into events and session facts.
pub fn parse_step(
    home: &Home,
    _config: &Config,
    _: &LocalRuntime,
    _: Deadline,
) -> Result<StepOutcome> {
    parse_outcome(&normalize::normalize(home, false)?)
}

pub fn parse_outcome(r: &normalize::NormalizeReport) -> Result<StepOutcome> {
    Ok(StepOutcome {
        name: "parse".into(),
        ok: r.failed == 0,
        summary: r.summary(),
        errors: Vec::new(),
        report: serde_json::to_value(r)?,
    })
}

/// Scan every session whose findings are missing or out of date, with no limit per tick.
pub fn scan_step(
    home: &Home,
    config: &Config,
    _: &LocalRuntime,
    _: Deadline,
) -> Result<StepOutcome> {
    scan_outcome(&findings::scan_all(home, config)?)
}

pub fn scan_outcome(r: &findings::ScanReport) -> Result<StepOutcome> {
    Ok(StepOutcome {
        name: "scan".into(),
        ok: r.failed == 0,
        summary: r.summary(),
        errors: r.errors.clone(),
        report: serde_json::to_value(r)?,
    })
}

/// Bring the word index up to date, reading through the read path as Distill itself.
pub fn index_step(
    home: &Home,
    _config: &Config,
    _: &LocalRuntime,
    _: Deadline,
) -> Result<StepOutcome> {
    let r = index::build(home)?;
    Ok(StepOutcome {
        name: "index".into(),
        ok: r.failed == 0,
        summary: r.summary(),
        errors: r.errors.clone(),
        report: serde_json::to_value(&r)?,
    })
}

/// Write cards for the sessions whose card is missing or out of date, newest first, up to
/// `sessions_per_run`; skipped when `card_model` is not set or no local runtime can run it.
pub fn cards_step(
    home: &Home,
    config: &Config,
    runtime: &LocalRuntime,
    deadline: Deadline,
) -> Result<StepOutcome> {
    let r = cards::write(home, config, runtime, deadline)?;
    Ok(StepOutcome {
        name: "cards".into(),
        ok: r.failed == 0,
        summary: r.summary(),
        errors: r.errors.clone(),
        report: serde_json::to_value(&r)?,
    })
}

/// Embed the sessions whose vectors are missing or out of date, newest first, up to
/// `sessions_per_run`; skipped when `embedding_model` is not set or no local runtime can run it.
pub fn vectors_step(
    home: &Home,
    config: &Config,
    runtime: &LocalRuntime,
    deadline: Deadline,
) -> Result<StepOutcome> {
    let r = index::vectors::build(home, config, runtime, deadline)?;
    Ok(StepOutcome {
        name: "vectors".into(),
        ok: r.failed == 0,
        summary: r.summary(),
        errors: r.errors.clone(),
        report: serde_json::to_value(&r)?,
    })
}

/// Find personal details with the local model in the sessions whose model findings are missing
/// or out of date, newest first, up to `sessions_per_run`; skipped when no model is set or no
/// local runtime can run it.
pub fn details_step(
    home: &Home,
    config: &Config,
    runtime: &LocalRuntime,
    deadline: Deadline,
) -> Result<StepOutcome> {
    let r = details::find(home, config, runtime, deadline)?;
    Ok(StepOutcome {
        name: "details".into(),
        ok: r.failed == 0,
        summary: r.summary(),
        errors: r.errors.clone(),
        report: serde_json::to_value(&r)?,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TickReport {
    pub steps: Vec<StepOutcome>,
}

/// Run the scheduled steps with local embeddings and the configured generation provider.
pub fn tick(home: &Home, config: &Config) -> Result<TickReport> {
    let local = LocalRuntime::ollama();
    let generation = match &config.steward.generation {
        Some(provider) => generation_runtime(config, provider),
        None => Ok(local.clone()),
    };
    tick_with_runtimes(home, config, &generation, &local, model_budget(config))
}

fn generation_runtime(config: &Config, generation: &GenerationConfig) -> Result<LocalRuntime> {
    if !Trust::new(config)
        .provider(&generation.provider)
        .is_some_and(|p| p.kind == ProviderKind::Remote && p.trusted)
    {
        bail!(
            "generation provider {:?} must be a trusted remote provider",
            generation.provider
        );
    }
    LocalRuntime::remote_generation(generation)
}

/// Run every step in order with `runtime`, within the config's [`model_budget`], and record the
/// tick.
pub fn tick_with(home: &Home, config: &Config, runtime: &LocalRuntime) -> Result<TickReport> {
    tick_within(home, config, runtime, model_budget(config))
}

/// Run every step in order with `runtime`, the model steps within `budget` of the tick's start,
/// and record the tick. A step that fails is recorded and the next one still runs.
pub fn tick_within(
    home: &Home,
    config: &Config,
    runtime: &LocalRuntime,
    budget: Duration,
) -> Result<TickReport> {
    tick_with_runtimes(home, config, &Ok(runtime.clone()), runtime, budget)
}

fn tick_with_runtimes(
    home: &Home,
    config: &Config,
    generation: &Result<LocalRuntime>,
    local: &LocalRuntime,
    budget: Duration,
) -> Result<TickReport> {
    // A budget too long to be an instant is none.
    let end = Instant::now().checked_add(budget);
    home.ensure()?;
    let mut steps = Vec::with_capacity(STEPS.len());
    for (i, s) in STEPS.iter().enumerate() {
        let deadline = match end {
            Some(end) if s.model => {
                let sharing = STEPS[i..].iter().filter(|s| s.model).count() as u32;
                let left = end.saturating_duration_since(Instant::now());
                Deadline::between(left / sharing, left)
            }
            _ => Deadline::NONE,
        };
        let runtime = if s.generation {
            generation.as_ref()
        } else {
            Ok(local)
        };
        let outcome = match runtime {
            Ok(runtime) => (s.run)(home, config, runtime, deadline)
                .unwrap_or_else(|e| StepOutcome::failed(s.name, &e)),
            Err(e) => StepOutcome::failed(s.name, e),
        };
        steps.push(outcome);
    }
    runlog::record(home, &RunEntry::of("tick", steps.clone()))?;
    Ok(TickReport { steps })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Harness;

    #[test]
    fn the_steps_are_sweep_parse_scan_index_cards_vectors_then_details() {
        let names: Vec<_> = STEPS.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            [
                "sweep", "parse", "scan", "index", "cards", "vectors", "details"
            ]
        );
    }

    #[test]
    fn an_untrusted_generation_provider_does_not_stop_the_vector_step() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        let mut config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        config.steward.generation = Some(GenerationConfig {
            provider: "remote".into(),
            base_url: "https://example.com/v1".into(),
            api_key_file: dir.path().join("key"),
        });
        let report = tick(&home, &config).unwrap();
        for name in ["cards", "details"] {
            let step = report.steps.iter().find(|s| s.name == name).unwrap();
            assert!(!step.ok);
            assert!(step.summary.contains("must be a trusted remote provider"));
        }
        let vectors = report.steps.iter().find(|s| s.name == "vectors").unwrap();
        assert!(vectors.ok);
        assert_eq!(vectors.report["skipped"], "no embedding_model is set");
    }

    #[test]
    fn a_tick_sends_generation_and_embeddings_to_their_own_runtimes() {
        use crate::runtime::{Call, FakeRuntime, Stored};
        use crate::trust::Reader;

        let dir = tempfile::tempdir().unwrap();
        let pi = dir.path().join("pi");
        std::fs::create_dir(&pi).unwrap();
        pi_session(&pi, "p1", "rename the widget", 1794436930000);
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        let mut config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        let harness = config.harness_mut(Harness::Pi);
        harness.enabled = true;
        harness.roots = vec![pi];
        crate::trust::add(&mut config, "remote", true).unwrap();
        config.steward.card_model = Some("card".into());
        config.steward.detail_model = Some("detail".into());
        config.steward.embedding_model = Some("embedding".into());
        config.save(&home.config_path()).unwrap();

        let generation = FakeRuntime::running()
            .with_provider("remote")
            .with_model("card", Stored::Here)
            .with_model("detail", Stored::Here)
            .replying(|model, _| match model {
                "card" => r#"{"title":"Rename the widget","summary":"Renamed it.","tags":["ui"],"outcome":"done"}"#.into(),
                "detail" => r#"{"details":[]}"#.into(),
                _ => panic!("unexpected model {model}"),
            });
        let local = FakeRuntime::running().with_model("embedding", Stored::Here);
        let report = tick_with_runtimes(
            &home,
            &config,
            &Ok(generation.runtime()),
            &local.runtime(),
            model_budget(&config),
        )
        .unwrap();
        assert!(report.steps.iter().all(|s| s.ok), "{report:?}");
        assert!(matches!(
            generation.calls().as_slice(),
            [Call::Generate { model: card, .. }, Call::Generate { model: detail, .. }]
                if card == "card" && detail == "detail"
        ));
        assert!(matches!(
            local.calls().as_slice(),
            [Call::Embed { model, .. }] if model == "embedding"
        ));
        let readers: Vec<_> = crate::readlog::list(&home, None, 20)
            .unwrap()
            .into_iter()
            .filter_map(|e| match e.reader {
                Reader::Model { .. } => Some(e.reader.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(
            readers,
            ["remote/detail", "ollama/embedding", "remote/card"]
        );
    }

    #[test]
    fn a_missing_or_empty_generation_credential_is_reported_by_the_tick() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        let key = dir.path().join("key");
        let mut config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        crate::trust::add(&mut config, "remote", true).unwrap();
        config.steward.generation = Some(GenerationConfig {
            provider: "remote".into(),
            base_url: "https://example.com/v1".into(),
            api_key_file: key.clone(),
        });

        for error in [
            "cannot read the generation API key file",
            "invalid generation API key",
        ] {
            let report = tick(&home, &config).unwrap();
            for name in ["cards", "details"] {
                let step = report.steps.iter().find(|s| s.name == name).unwrap();
                assert!(!step.ok);
                assert!(step.summary.contains(error), "{step:?}");
            }
            assert!(
                report
                    .steps
                    .iter()
                    .find(|s| s.name == "vectors")
                    .unwrap()
                    .ok
            );
            std::fs::write(&key, "\n").unwrap();
        }
    }

    #[test]
    fn a_tick_sweeps_parses_scans_and_indexes_and_writes_one_run_log_entry() {
        let dir = tempfile::tempdir().unwrap();
        let pi = dir.path().join("pi");
        std::fs::create_dir_all(&pi).unwrap();
        std::fs::write(
            pi.join("2026-11-01_p1.jsonl"),
            r#"{"type":"session","id":"p1","cwd":"/r","version":3}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"rename the widget"}],"timestamp":1794436930000}}
"#,
        )
        .unwrap();
        let home = Home::new(dir.path().join("home"));
        // Folders a previous build left in the home stay as they were.
        for old in ["recipes", "exports/good-sessions/b1", "steward"] {
            std::fs::create_dir_all(home.root.join(old)).unwrap();
            std::fs::write(home.root.join(old).join("keep.txt"), "old").unwrap();
        }
        let mut config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        let h = config.harness_mut(Harness::Pi);
        h.enabled = true;
        h.roots = vec![pi];

        let r = tick(&home, &config).unwrap();
        let names: Vec<_> = r.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "sweep", "parse", "scan", "index", "cards", "vectors", "details"
            ]
        );
        assert!(r.steps.iter().all(|s| s.ok), "{r:?}");
        assert_eq!(r.steps[0].report["new"], 1);
        assert_eq!(r.steps[1].report["sessions"], 1);
        assert_eq!(r.steps[2].report["scanned"], 1);
        assert_eq!(r.steps[3].report["indexed"], 1);
        assert!(home.meta_path(Harness::Pi, "p1").exists());
        assert!(home.findings_path(Harness::Pi, "p1").exists());

        let runs = runlog::recent(&home, 10).unwrap();
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].step, "tick");
        assert_eq!(runs[0].steps, r.steps);

        // Nothing but the kept features was written, and the old folders are untouched.
        let mut top: Vec<String> = std::fs::read_dir(&home.root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        top.sort();
        assert_eq!(
            top,
            [
                "exports", "index", "keys", "log", "raw", "recipes", "runs", "sessions", "state",
                "steward"
            ]
        );
        for old in ["recipes", "exports/good-sessions/b1", "steward"] {
            assert!(home.root.join(old).join("keep.txt").exists(), "{old}");
        }
        assert!(!home.card_path(Harness::Pi, "p1").exists());
        // With no card_model, the card step is skipped, and the run log says why.
        assert_eq!(r.steps[4].summary, "cards skipped: no card_model is set");
        assert_eq!(r.steps[4].report["skipped"], "no card_model is set");
        // So is the vectors step, with no embedding_model, and nothing is written for it.
        assert_eq!(
            r.steps[5].summary,
            "vectors skipped: no embedding_model is set"
        );
        assert!(!home.vectors_path().exists());
        assert_eq!(
            r.steps[6].summary,
            "model detection skipped: no detail_model or card_model is set"
        );
    }

    #[test]
    fn a_tick_with_a_card_model_writes_cards_and_logs_their_counts_and_failures() {
        use crate::runtime::{FakeRuntime, Stored};
        let dir = tempfile::tempdir().unwrap();
        let pi = dir.path().join("pi");
        std::fs::create_dir_all(&pi).unwrap();
        for (id, said, at) in [
            ("p1", "rename the widget", 1794436930000_u64),
            ("p2", "fix the gadget", 1794436990000),
        ] {
            std::fs::write(
                pi.join(format!("2026-11-01_{id}.jsonl")),
                format!(
                    r#"{{"type":"session","id":"{id}","cwd":"/r","version":3}}
{{"type":"message","message":{{"role":"user","content":[{{"type":"text","text":"{said}"}}],"timestamp":{at}}}}}
"#
                ),
            )
            .unwrap();
        }
        let home = Home::new(dir.path().join("home"));
        let mut config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        let h = config.harness_mut(Harness::Pi);
        h.enabled = true;
        h.roots = vec![pi];
        config.steward.card_model = Some("small:8b".into());
        let fake = FakeRuntime::running()
            .with_model("small:8b", Stored::Here)
            .replying(|_, prompt| {
                if prompt.contains("gadget") {
                    "not a card".into()
                } else {
                    r#"{"title":"Rename the widget","summary":"Renamed it.","tags":["pi"],"outcome":"done"}"#.into()
                }
            });

        let r = tick_with(&home, &config, &fake.runtime()).unwrap();
        let cards = &r.steps[4];
        assert_eq!(cards.name, "cards");
        assert!(!cards.ok, "{cards:?}");
        assert_eq!(cards.report["written"], 1, "{cards:?}");
        assert_eq!(cards.report["failed"], 1, "{cards:?}");
        assert!(
            cards.errors[0].starts_with("pi/p2: the model's answer is not a usable card"),
            "{cards:?}"
        );
        assert!(home.card_path(Harness::Pi, "p1").exists());
        // The steps before it all ran and succeeded.
        assert!(r.steps[..4].iter().all(|s| s.ok), "{r:?}");
        assert_eq!(r.steps[5].name, "vectors");
        assert_eq!(r.steps[6].name, "details");
        let runs = runlog::recent(&home, 10).unwrap();
        assert_eq!(runs[0].steps, r.steps);
    }

    /// A Pi session `id` saying `said`, at `at` milliseconds.
    fn pi_session(dir: &std::path::Path, id: &str, said: &str, at: u64) {
        std::fs::write(
            dir.join(format!("2026-11-01_{id}.jsonl")),
            format!(
                r#"{{"type":"session","id":"{id}","cwd":"/r","version":3}}
{{"type":"message","message":{{"role":"user","content":[{{"type":"text","text":"{said}"}}],"timestamp":{at}}}}}
"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn a_slow_model_stops_at_the_time_limit_and_the_next_tick_sweeps_and_catches_up() {
        use crate::runtime::{FakeRuntime, Stored};
        const SESSIONS: u64 = 12;
        let dir = tempfile::tempdir().unwrap();
        let pi = dir.path().join("pi");
        std::fs::create_dir_all(&pi).unwrap();
        for i in 0..SESSIONS {
            pi_session(
                &pi,
                &format!("p{i}"),
                "rename the widget",
                1794436930000 + i,
            );
        }
        let home = Home::new(dir.path().join("home"));
        let mut config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        let h = config.harness_mut(Harness::Pi);
        h.enabled = true;
        h.roots = vec![pi.clone()];
        config.steward.card_model = Some("small:8b".into());
        config.save(&home.config_path()).unwrap();
        // A card takes the model a quarter of a second; the model detection pass is quick.
        let answer = |slow: bool| {
            move |_: &str, prompt: &str| {
                if prompt.starts_with(details::PROMPT) {
                    return r#"{"details":[]}"#.to_string();
                }
                if slow {
                    std::thread::sleep(Duration::from_millis(250));
                }
                r#"{"title":"Rename the widget","summary":"Renamed it.","tags":["pi"],"outcome":"done"}"#.to_string()
            }
        };
        let slow = FakeRuntime::running()
            .with_model("small:8b", Stored::Here)
            .replying(answer(true));

        // Twelve cards take three seconds; the tick's budget is under one.
        let r = tick_within(&home, &config, &slow.runtime(), Duration::from_millis(900)).unwrap();
        assert!(
            r.steps[..4].iter().all(|s| s.ok),
            "capture ran first: {r:?}"
        );
        let cards = &r.steps[4];
        let written = cards.report["written"].as_u64().unwrap();
        assert!(written < SESSIONS, "{cards:?}");
        assert_eq!(cards.report["stopped_at_deadline"], true, "{cards:?}");
        assert_eq!(cards.report["failed"], 0, "{cards:?}");
        assert!(
            cards.ok,
            "stopping at the time limit is not a failure: {cards:?}"
        );
        assert!(
            cards
                .summary
                .ends_with(", stopped at the tick's time limit"),
            "{cards:?}"
        );
        // The run log has the tick's one entry, which says so.
        let runs = runlog::recent(&home, 10).unwrap();
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].steps, r.steps);
        assert!(runs[0].summary.contains("stopped at the tick's time limit"));
        // The sessions it did not reach wait, none of them as a failure, and status says where
        // the step stopped.
        assert!(crate::model_step::load_failures(&home.card_failures_path()).is_empty());
        let s = crate::status::status(&home).unwrap();
        assert_eq!(
            (s.cards.current, s.cards.pending, s.cards.failing),
            (written, SESSIONS - written, 0)
        );
        assert_eq!(s.cards.stopped_after, Some(written));

        // The next tick sweeps the session that came in meanwhile, then takes every session
        // still waiting.
        pi_session(&pi, "late", "fix the gadget", 1794436990000);
        let fast = FakeRuntime::running()
            .with_model("small:8b", Stored::Here)
            .replying(answer(false));
        let r = tick_with(&home, &config, &fast.runtime()).unwrap();
        assert_eq!(r.steps[0].report["new"], 1, "{r:?}");
        let cards = &r.steps[4];
        assert_eq!(
            cards.report["written"].as_u64().unwrap(),
            SESSIONS - written + 1,
            "{cards:?}"
        );
        assert!(
            cards.report.get("stopped_at_deadline").is_none(),
            "{cards:?}"
        );
        let s = crate::status::status(&home).unwrap();
        assert_eq!((s.cards.current, s.cards.pending), (SESSIONS + 1, 0));
        assert_eq!(s.cards.stopped_after, None);
    }

    #[test]
    fn the_model_steps_share_what_is_left_of_the_budget_and_capture_has_none() {
        let budget = model_budget(&Config::default());
        assert_eq!(
            budget,
            Duration::from_secs(30 * 60),
            "half the hourly timer"
        );
        let mut config = Config::default();
        config.schedule.sweep_every_minutes = 1;
        assert_eq!(
            model_budget(&config),
            Duration::from_secs(150),
            "half of five minutes"
        );
        let model: Vec<_> = STEPS.iter().filter(|s| s.model).map(|s| s.name).collect();
        assert_eq!(model, ["cards", "vectors", "details"]);
        assert!(!Deadline::NONE.passed() && !Deadline::NONE.over());
        assert!(Deadline::after(Duration::ZERO).passed());
        assert!(Deadline::after(Duration::ZERO).over());
        assert!(!Deadline::after(Duration::from_secs(60)).passed());
        let share = Deadline::between(Duration::ZERO, Duration::from_secs(60));
        assert!(share.passed() && !share.over());
        assert_eq!(Deadline::after(Duration::MAX), Deadline::NONE);
    }
}
