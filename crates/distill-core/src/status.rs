//! Status: one report built here, which `distill status` prints and the GUI overview will render.
//! Today it covers sessions and capture health per harness, format drift, the last sweep and the
//! scan's progress. Each later ticket adds its own fields.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::archive::Archive;
use crate::findings::{self, ScanProgress};
use crate::harness::Harness;
use crate::home::Home;
use crate::normalize;
use crate::runlog;
use crate::sweep::SweepReport;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// Whether `config.toml` exists and loads.
    pub configured: bool,
    /// Every session in the session list.
    pub sessions: u64,
    /// Bytes on disk in the raw archive.
    pub archive_bytes: u64,
    /// When the last successful sweep ran.
    pub last_sweep: Option<String>,
    pub harnesses: Vec<HarnessStatus>,
    /// How many sessions have current findings, and how many wait for a scan.
    pub scan: ScanProgress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Ok,
    /// The last sweep failed to capture some of its files, or some captures fail to parse.
    Warn,
    Disabled,
    /// Enabled, but nothing of it is on this machine or in the archive.
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessStatus {
    pub harness: Harness,
    pub label: String,
    pub enabled: bool,
    pub health: Health,
    /// Sessions in the session list.
    pub sessions: u64,
    /// Files in the raw archive (each origin's latest capture).
    pub captures: u64,
    pub archive_bytes: u64,
    /// When the newest capture was taken.
    pub last_capture: Option<String>,
    /// Files the last sweep failed to capture.
    pub capture_errors: u64,
    /// Captures that failed to parse.
    pub parse_failures: u64,
    /// Of those, captures that look like format drift.
    pub drift: u64,
    pub note: Option<String>,
}

pub fn status(home: &Home) -> Result<Status> {
    let config = home.load_config().ok();
    let sessions = normalize::read_session_list(home)?;
    let archive = Archive::open(home)?;
    let failures = normalize::failures(home);
    let last = runlog::last_ok(home, "sweep")?;
    // The last sweep's errors per harness, from the newest run that swept at all.
    let sweep_errors: BTreeMap<Harness, u64> = runlog::recent(home, 500)?
        .into_iter()
        .find_map(|e| e.outcome("sweep"))
        .and_then(|o| serde_json::from_value::<SweepReport>(o.report).ok())
        .map(|r| r.errors_by_harness)
        .unwrap_or_default();

    let mut harnesses = Vec::new();
    for kind in Harness::ALL {
        let enabled = config.as_ref().is_some_and(|c| c.enabled(kind));
        let captured: Vec<_> = archive
            .latest()
            .values()
            .filter(|r| r.harness == kind)
            .collect();
        let archive_bytes = captured.iter().map(|r| r.size).sum();
        let present = !captured.is_empty()
            || config
                .as_ref()
                .is_some_and(|c| c.roots_for(kind).iter().any(|r| r.exists()));
        let f = failures.get(&kind).cloned().unwrap_or_default();
        let capture_errors = sweep_errors.get(&kind).copied().unwrap_or(0);
        let (health, note) = if !enabled {
            (Health::Disabled, None)
        } else if !present {
            (Health::Missing, Some("not found on this machine".into()))
        } else if f.drift > 0 {
            (
                Health::Warn,
                Some(format!("{} look like format drift", f.drift)),
            )
        } else if f.failed > 0 {
            (Health::Warn, Some(format!("{} failed to parse", f.failed)))
        } else if capture_errors > 0 {
            (
                Health::Warn,
                Some(format!("{capture_errors} failed to capture")),
            )
        } else if !kind.has_parser() {
            (Health::Warn, Some("archived, no parser".into()))
        } else {
            (Health::Ok, None)
        };
        harnesses.push(HarnessStatus {
            harness: kind,
            label: kind.label().to_string(),
            enabled,
            health,
            sessions: sessions.iter().filter(|m| m.harness == kind).count() as u64,
            captures: captured.len() as u64,
            archive_bytes,
            last_capture: captured.iter().map(|r| r.captured_at.clone()).max(),
            capture_errors,
            parse_failures: f.failed,
            drift: f.drift,
            note,
        });
    }

    Ok(Status {
        configured: config.is_some(),
        sessions: sessions.len() as u64,
        archive_bytes: archive.stored_bytes(),
        last_sweep: last.map(|(at, _)| at),
        harnesses,
        scan: findings::progress(home)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn status_reports_sessions_capture_health_drift_and_the_last_sweep() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(claude.join("-w")).unwrap();
        std::fs::write(
            claude.join("-w/ok.jsonl"),
            r#"{"type":"user","cwd":"/w","timestamp":"2026-11-01T00:00:00Z","message":{"role":"user","content":"hi"}}
"#,
        )
        .unwrap();
        // A sizable file in a shape the parser doesn't know: format drift.
        let line = r#"{"type":"turn.v2","payload":{"role":"user","text":"hello there"}}"#;
        std::fs::write(
            claude.join("-w/drift.jsonl"),
            std::iter::repeat_n(format!("{line}\n"), 400).collect::<String>(),
        )
        .unwrap();
        let home = Home::new(dir.path().join("home"));
        let mut config = Config::default();
        for h in &mut config.harnesses {
            h.enabled = false;
        }
        let h = config.harness_mut(Harness::ClaudeCode);
        h.enabled = true;
        h.roots = vec![claude];
        h.roots.push(dir.path().join("nowhere"));
        let g = config.harness_mut(Harness::Goose);
        g.enabled = true;
        g.roots = vec![dir.path().join("nowhere")];
        home.ensure().unwrap();
        config.save(&home.config_path()).unwrap();

        let before = status(&home).unwrap();
        assert!(before.configured);
        assert_eq!(before.last_sweep, None);
        assert_eq!(before.scan, ScanProgress::default());

        crate::tick::tick(&home, &config).unwrap();
        let s = status(&home).unwrap();
        assert_eq!(s.sessions, 1);
        assert!(s.last_sweep.is_some());
        let of = |k: Harness| s.harnesses.iter().find(|h| h.harness == k).unwrap();
        let c = of(Harness::ClaudeCode);
        assert_eq!((c.sessions, c.captures), (1, 2));
        assert_eq!((c.parse_failures, c.drift), (1, 1));
        assert_eq!(c.health, Health::Warn);
        assert!(c.last_capture.is_some());
        assert_eq!(of(Harness::Goose).health, Health::Missing);
        assert_eq!(of(Harness::Codex).health, Health::Disabled);
        assert_eq!(s.harnesses.len(), Harness::ALL.len());
        assert_eq!((s.scan.sessions, s.scan.current, s.scan.pending), (1, 1, 0));
    }
}
