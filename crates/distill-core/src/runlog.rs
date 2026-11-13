//! A small log of what each run did, so the dashboard can show it and the steward's work is
//! visible. One JSONL file per UTC day.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::home::Home;
use crate::util::{append_jsonl, now_rfc3339, read_jsonl};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEntry {
    pub at: String,
    /// sweep, normalize, steward, export, verify, repair
    pub step: String,
    pub ok: bool,
    /// One human sentence, shown on the dashboard.
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

impl RunEntry {
    pub fn new(step: &str, ok: bool, summary: impl Into<String>) -> Self {
        Self {
            at: now_rfc3339(),
            step: step.to_string(),
            ok,
            summary: summary.into(),
            cost_usd: None,
            details: Value::Null,
        }
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

/// Steward spend recorded today (UTC).
pub fn spent_today(home: &Home) -> Result<f64> {
    let entries = today(home)?;
    // `+ 0.0` turns the empty sum's -0.0 into 0.0.
    Ok(entries.iter().filter_map(|e| e.cost_usd).sum::<f64>() + 0.0)
}

/// Every entry from today (UTC).
pub fn today(home: &Home) -> Result<Vec<RunEntry>> {
    let day = &now_rfc3339()[..10];
    read_jsonl(&home.runs_dir().join(format!("{day}.jsonl")))
}

/// When a step last ran successfully.
pub fn last_ok(home: &Home, step: &str) -> Result<Option<String>> {
    Ok(recent(home, 500)?
        .into_iter()
        .find(|e| e.step == step && e.ok)
        .map(|e| e.at))
}
