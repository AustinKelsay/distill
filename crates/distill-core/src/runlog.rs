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
}
