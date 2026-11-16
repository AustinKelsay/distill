//! A tick: one scheduled run of the steward's steps, in the order [`STEPS`] declares them. Each
//! tick writes one entry to the run log. Today the steps are sweep, parse, scan and the word
//! index; the spec's later steps (cards, vectors, model detection pass) are added to [`STEPS`] by
//! the tickets that build them.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::findings;
use crate::home::Home;
use crate::index;
use crate::normalize;
use crate::runlog::{self, RunEntry, StepOutcome};
use crate::sweep;

/// One step of a tick.
pub struct Step {
    pub name: &'static str,
    pub run: fn(&Home, &Config) -> Result<StepOutcome>,
}

/// The tick's steps, in order. Capture comes first and never waits on a model.
pub const STEPS: &[Step] = &[
    Step {
        name: "sweep",
        run: sweep_step,
    },
    Step {
        name: "parse",
        run: parse_step,
    },
    Step {
        name: "scan",
        run: scan_step,
    },
    Step {
        name: "index",
        run: index_step,
    },
];

/// Copy new and changed harness files into the raw archive.
pub fn sweep_step(home: &Home, config: &Config) -> Result<StepOutcome> {
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
pub fn parse_step(home: &Home, _config: &Config) -> Result<StepOutcome> {
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
pub fn scan_step(home: &Home, config: &Config) -> Result<StepOutcome> {
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
pub fn index_step(home: &Home, _config: &Config) -> Result<StepOutcome> {
    let r = index::build(home)?;
    Ok(StepOutcome {
        name: "index".into(),
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

/// Run every step in order and record the tick. A step that fails is recorded and the next one
/// still runs.
pub fn tick(home: &Home, config: &Config) -> Result<TickReport> {
    home.ensure()?;
    let steps: Vec<StepOutcome> = STEPS
        .iter()
        .map(|s| (s.run)(home, config).unwrap_or_else(|e| StepOutcome::failed(s.name, &e)))
        .collect();
    runlog::record(home, &RunEntry::of("tick", steps.clone()))?;
    Ok(TickReport { steps })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Harness;

    #[test]
    fn the_steps_are_sweep_parse_scan_then_index() {
        let names: Vec<_> = STEPS.iter().map(|s| s.name).collect();
        assert_eq!(names, ["sweep", "parse", "scan", "index"]);
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
        assert_eq!(names, ["sweep", "parse", "scan", "index"]);
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
    }
}
