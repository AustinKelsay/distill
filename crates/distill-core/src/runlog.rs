//! The run log: one entry for each tick, and for each step run on its own from the command line
//! (`distill sweep`, `distill parse`, `distill verify`). One JSONL file per UTC day under `runs/`.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::home::Home;
use crate::util::{append_jsonl, now_rfc3339, read_jsonl};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEntry {
    pub at: String,
    /// `tick`, or the one step that ran (`sweep`, `parse`, `verify`). Entries the previous build
    /// wrote name its steps (`normalize`, `steward`, `export`, …).
    pub step: String,
    pub ok: bool,
    /// One sentence a person can read.
    pub summary: String,
    /// What each step did, in the order they ran.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<StepOutcome>,
    /// The previous build's per-entry details. Read, never written.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

/// What one step of a run did: its counts, its failures and what it skipped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepOutcome {
    pub name: String,
    pub ok: bool,
    pub summary: String,
    /// The step's own report.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub report: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl StepOutcome {
    pub fn failed(name: &str, error: &anyhow::Error) -> Self {
        Self {
            name: name.to_string(),
            ok: false,
            summary: format!("{name} failed: {error:#}"),
            report: Value::Null,
            errors: vec![format!("{error:#}")],
        }
    }
}

impl RunEntry {
    /// An entry for a run made of `steps`.
    pub fn of(step: &str, steps: Vec<StepOutcome>) -> Self {
        let ok = steps.iter().all(|s| s.ok);
        let summary = steps
            .iter()
            .map(|s| s.summary.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        Self {
            at: now_rfc3339(),
            step: step.to_string(),
            ok,
            summary,
            steps,
            details: Value::Null,
        }
    }

    /// The outcome of a step in this run: one of its steps, or the run itself when it was that
    /// step alone and recorded none (as the previous build's entries do).
    pub fn outcome(&self, step: &str) -> Option<StepOutcome> {
        if let Some(s) = self.steps.iter().find(|s| s.name == step) {
            return Some(s.clone());
        }
        (self.step == step).then(|| StepOutcome {
            name: step.to_string(),
            ok: self.ok,
            summary: self.summary.clone(),
            report: self.details.clone(),
            errors: Vec::new(),
        })
    }
}

pub fn record(home: &Home, entry: &RunEntry) -> Result<()> {
    let day = &entry.at[..10.min(entry.at.len())];
    append_jsonl(&home.runs_dir().join(format!("{day}.jsonl")), entry)
}

/// The most recent entries, newest first.
pub fn recent(home: &Home, limit: usize) -> Result<Vec<RunEntry>> {
    let mut files: Vec<_> = std::fs::read_dir(home.runs_dir())
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e == "jsonl"));
    files.sort();
    let mut out = Vec::new();
    for f in files.iter().rev() {
        let mut entries: Vec<RunEntry> = read_jsonl(f)?;
        entries.reverse();
        out.extend(entries);
        if out.len() >= limit {
            break;
        }
    }
    out.truncate(limit);
    Ok(out)
}

/// The newest run that ran `step` and it succeeded, with when it ran.
pub fn last_ok(home: &Home, step: &str) -> Result<Option<(String, StepOutcome)>> {
    Ok(recent(home, 500)?
        .into_iter()
        .find_map(|e| e.outcome(step).filter(|o| o.ok).map(|o| (e.at.clone(), o))))
}

/// What a step came to in one run, for the run history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepResult {
    Ok,
    /// It stopped taking sessions at the tick's time limit, with some left for the next run.
    Stopped,
    /// It did not run: no model is set, or none can run it.
    Skipped,
    Failed,
}

impl StepOutcome {
    /// What the step came to, read from its report.
    pub fn result(&self) -> StepResult {
        if !self.ok {
            StepResult::Failed
        } else if self.report.get("skipped").is_some_and(|v| !v.is_null()) {
            StepResult::Skipped
        } else if self.report.get("stopped_at_deadline") == Some(&Value::Bool(true)) {
            StepResult::Stopped
        } else {
            StepResult::Ok
        }
    }
}

/// One step of a run in the run history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepMark {
    pub name: String,
    pub result: StepResult,
}

/// Runs in a row that came to the same results, for the run history (issue #160).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunGroup {
    /// When the newest and the oldest of them ran.
    pub newest: String,
    pub oldest: String,
    pub runs: u64,
    /// `tick`, or the one step that ran.
    pub step: String,
    /// Each step and what it came to, in the order they ran.
    pub steps: Vec<StepMark>,
    /// The newest run's sentence for each step that failed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

/// The last `runs` entries of the run log, newest first, with runs in a row that came to the same
/// results folded into one group.
pub fn history(home: &Home, runs: usize) -> Result<Vec<RunGroup>> {
    let mut groups: Vec<RunGroup> = Vec::new();
    for e in recent(home, runs)? {
        let outcomes = if e.steps.is_empty() {
            e.outcome(&e.step).into_iter().collect()
        } else {
            e.steps.clone()
        };
        let steps: Vec<StepMark> = outcomes
            .iter()
            .map(|s| StepMark {
                name: s.name.clone(),
                result: s.result(),
            })
            .collect();
        match groups.last_mut() {
            Some(g) if g.step == e.step && g.steps == steps => {
                g.oldest = e.at.clone();
                g.runs += 1;
            }
            _ => groups.push(RunGroup {
                newest: e.at.clone(),
                oldest: e.at.clone(),
                runs: 1,
                step: e.step.clone(),
                steps,
                failures: outcomes
                    .iter()
                    .filter(|s| !s.ok)
                    .map(|s| s.summary.clone())
                    .collect(),
            }),
        }
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_sweep_is_found_in_ticks_and_in_runs_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        let step = |name: &str, ok| StepOutcome {
            name: name.into(),
            ok,
            summary: name.into(),
            report: Value::Null,
            errors: vec![],
        };
        assert_eq!(last_ok(&home, "sweep").unwrap(), None);
        // An entry the previous build wrote for a sweep on its own.
        std::fs::create_dir_all(home.runs_dir()).unwrap();
        std::fs::write(
            home.runs_dir().join("2026-11-14.jsonl"),
            r#"{"at":"2026-11-14T10:00:00Z","step":"sweep","ok":true,"summary":"Swept","details":{"new":1}}
{"at":"2026-11-14T11:00:00Z","step":"steward","ok":true,"summary":"Wrote cards","cost_usd":0.01}
"#,
        )
        .unwrap();
        let (at, o) = last_ok(&home, "sweep").unwrap().unwrap();
        assert_eq!(at, "2026-11-14T10:00:00Z");
        assert_eq!(o.report["new"], 1);

        // A tick whose sweep succeeded counts; one whose sweep failed does not.
        let mut good = RunEntry::of("tick", vec![step("sweep", true), step("parse", false)]);
        good.at = "2026-11-15T10:00:00Z".into();
        record(&home, &good).unwrap();
        let mut bad = RunEntry::of("tick", vec![step("sweep", false), step("parse", true)]);
        bad.at = "2026-11-15T11:00:00Z".into();
        record(&home, &bad).unwrap();
        assert!(!good.ok, "a run is ok only when every step is");
        assert_eq!(
            last_ok(&home, "sweep").unwrap().unwrap().0,
            "2026-11-15T10:00:00Z"
        );
    }

    #[test]
    fn the_history_folds_runs_in_a_row_with_the_same_results() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        let step = |name: &str, ok, report: Value| StepOutcome {
            name: name.into(),
            ok,
            summary: format!("{name} said so"),
            report,
            errors: vec![],
        };
        let tick = |at: &str, cards: StepOutcome| RunEntry {
            at: at.into(),
            ..RunEntry::of(
                "tick",
                vec![
                    step("sweep", true, Value::Null),
                    cards,
                    step(
                        "vectors",
                        true,
                        serde_json::json!({"skipped": "no embedding_model is set"}),
                    ),
                ],
            )
        };
        let stopped = || {
            step(
                "cards",
                true,
                serde_json::json!({"stopped_at_deadline": true}),
            )
        };
        for e in [
            tick("2026-11-20T10:00:00Z", step("cards", false, Value::Null)),
            tick("2026-11-20T11:00:00Z", stopped()),
            tick("2026-11-20T12:00:00Z", stopped()),
            tick("2026-11-21T09:00:00Z", stopped()),
            tick(
                "2026-11-21T10:00:00Z",
                step("cards", true, serde_json::json!({})),
            ),
        ] {
            record(&home, &e).unwrap();
        }
        let h = history(&home, 50).unwrap();
        let marks = |g: &RunGroup| g.steps.iter().map(|s| s.result).collect::<Vec<_>>();
        use StepResult::*;
        assert_eq!(h.len(), 3, "{h:#?}");
        assert_eq!((h[0].runs, marks(&h[0])), (1, vec![Ok, Ok, Skipped]));
        assert_eq!(
            (h[1].runs, h[1].newest.as_str(), h[1].oldest.as_str()),
            (3, "2026-11-21T09:00:00Z", "2026-11-20T11:00:00Z"),
            "folded across the day's files"
        );
        assert_eq!(marks(&h[1]), vec![Ok, Stopped, Skipped]);
        assert_eq!((h[2].runs, marks(&h[2])), (1, vec![Ok, Failed, Skipped]));
        assert_eq!(h[2].failures, ["cards said so"]);
        assert_eq!(history(&home, 2).unwrap().len(), 2, "only the last runs");
    }
}
