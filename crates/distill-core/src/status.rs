//! Status: one report built here, which `distill status` prints and the GUI overview renders.
//! Today it covers sessions and capture health per harness, format drift, the last sweep, the
//! scan's, the word index's, the cards', the vectors' and the model detection pass's progress, the reveal requests waiting for the owner, warnings
//! when a harness could delete sessions before they are swept or the timer has stopped sweeping,
//! and the exposure check's results with a warning for each rule that fails. Each later ticket
//! adds its own fields.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::archive::Archive;
use crate::cards::{self, CardProgress};
use crate::config::Config;
use crate::details::{self, DetailProgress};
use crate::exposure::{self, Exposure, Outcome};
use crate::findings::{self, ScanProgress};
use crate::harness::Harness;
use crate::home::Home;
use crate::index::vectors::VectorProgress;
use crate::index::{self, IndexProgress};
use crate::normalize;
use crate::pruning;
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
    /// What the owner should act on, so that nothing is lost.
    pub warnings: Vec<Warning>,
    /// How many sessions have current findings, and how many wait for a scan.
    pub scan: ScanProgress,
    /// How many sessions the word index holds current, and how many wait for it.
    #[serde(default)]
    pub index: IndexProgress,
    /// How many sessions have a current card, how many wait for one, and why the card step is
    /// skipped when it is.
    #[serde(default)]
    pub cards: CardProgress,
    /// How many sessions have current vectors for search by meaning, how many wait for them, and
    /// why the vectors step is skipped when it is.
    #[serde(default)]
    pub vectors: VectorProgress,
    /// How many sessions have current model findings (personal details found by the local
    /// model), how many wait for the pass, and why it is skipped when it is.
    #[serde(default)]
    pub details: DetailProgress,
    /// Reveal requests waiting for the owner to approve or deny them in the GUI.
    #[serde(default)]
    pub pending_reveals: u64,
    /// The exposure check's results. Each failing rule is also a warning.
    pub exposure: Exposure,
}

/// One thing the owner should act on, in a sentence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Warning {
    pub kind: WarningKind,
    /// The harness it is about, if it is about one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<Harness>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    /// The timer has not swept for longer than twice its interval.
    SweepOverdue,
    /// A harness's setting could delete sessions sooner than the timer sweeps.
    PrunesSooner,
    /// A rule of the exposure check fails.
    Exposure,
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
    // The newest sweep that ran to the end, even if some files failed to capture.
    let completed: Option<(String, SweepReport)> =
        runlog::recent(home, 500)?.into_iter().find_map(|e| {
            let report = serde_json::from_value::<SweepReport>(e.outcome("sweep")?.report).ok()?;
            Some((e.at, report))
        });
    let sweep_errors: BTreeMap<Harness, u64> = completed
        .as_ref()
        .map(|(_, r)| r.errors_by_harness.clone())
        .unwrap_or_default();
    let mut warnings = Vec::new();
    if let Some(config) = &config {
        let since = completed
            .as_ref()
            .and_then(|(at, _)| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|t| t.with_timezone(&chrono::Utc))
            .or_else(|| config_written(home));
        warnings.extend(sweep_overdue(config, since, chrono::Utc::now()));
    }

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
        let prunes = match &config {
            Some(c) if enabled && present => prunes_sooner(kind, c),
            _ => None,
        };
        let (health, note) = if !enabled {
            (Health::Disabled, None)
        } else if !present {
            (Health::Missing, Some("not found on this machine".into()))
        } else if prunes.is_some() {
            (
                Health::Warn,
                Some("deletes sessions sooner than the sweep runs".into()),
            )
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
        warnings.extend(prunes);
    }

    let exposure = exposure::check(home);
    warnings.extend(
        exposure
            .rules
            .iter()
            .filter(|r| r.outcome == Outcome::Fail)
            .map(|r| Warning {
                kind: WarningKind::Exposure,
                harness: None,
                message: format!("Exposure: {}", r.message),
            }),
    );

    Ok(Status {
        configured: config.is_some(),
        sessions: sessions.len() as u64,
        archive_bytes: archive.stored_bytes(),
        last_sweep: last.map(|(at, _)| at),
        harnesses,
        warnings,
        scan: findings::progress(home)?,
        index: index::progress(home)?,
        cards: cards::progress(home)?,
        vectors: index::vectors::progress(home)?,
        details: details::progress(home)?,
        pending_reveals: crate::reveal::pending_count(home),
        exposure,
    })
}

/// A span of minutes a person can read: minutes under two hours, hours under two days, else days.
fn span(minutes: i64) -> String {
    let m = minutes.max(0) as u64;
    match m {
        0..120 => crate::util::count(m, "minute"),
        120..2880 => crate::util::count(m / 60, "hour"),
        _ => crate::util::count(m / 1440, "day"),
    }
}

/// A warning when `kind`'s own setting deletes sessions sooner than the timer sweeps, so that
/// one could go before it is captured.
fn prunes_sooner(kind: Harness, config: &Config) -> Option<Warning> {
    let every = crate::schedule::interval_minutes(config);
    let r = pruning::retention(kind, &config.roots_for(kind))?;
    (r.minutes < u64::from(every)).then(|| Warning {
        kind: WarningKind::PrunesSooner,
        harness: Some(kind),
        message: format!(
            "{} deletes sessions {} ({}), sooner than the timer sweeps (every {}): {}.",
            kind.label(),
            if r.minutes == 0 {
                "as soon as they end".to_string()
            } else {
                format!("untouched for {}", span(r.minutes as i64))
            },
            r.setting,
            span(i64::from(every)),
            if r.fixed {
                "sweep more often"
            } else {
                "raise the setting or sweep more often"
            }
        ),
    })
}

/// When `config.toml` was last written: the time to count from before the first sweep.
fn config_written(home: &Home) -> Option<chrono::DateTime<chrono::Utc>> {
    let modified = std::fs::metadata(home.config_path())
        .ok()?
        .modified()
        .ok()?;
    Some(modified.into())
}

/// A warning when nothing has swept since `since` for longer than twice the timer's interval.
fn sweep_overdue(
    config: &Config,
    since: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<Warning> {
    let every = crate::schedule::interval_minutes(config);
    let since = since?;
    let quiet = (now - since).num_minutes();
    (quiet > 2 * i64::from(every)).then(|| Warning {
        kind: WarningKind::SweepOverdue,
        harness: None,
        message: format!(
            "Nothing has swept for {}, and the timer runs every {}: check that it is installed and running (`distill schedule`).",
            span(quiet),
            span(i64::from(every))
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

#[cfg(test)]
mod warning_tests {
    use super::*;
    use crate::runlog::{RunEntry, StepOutcome};
    use crate::sweep::SweepReport;

    fn home_with(config: &Config) -> (tempfile::TempDir, Home) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        config.save(&home.config_path()).unwrap();
        (dir, home)
    }

    fn swept(home: &Home, minutes_ago: i64, ok: bool) {
        let mut report = SweepReport::default();
        if !ok {
            report.errors.push("one file failed".into());
        }
        let mut e = RunEntry::of(
            "tick",
            vec![StepOutcome {
                name: "sweep".into(),
                ok,
                summary: "swept".into(),
                report: serde_json::to_value(&report).unwrap(),
                errors: report.errors.clone(),
            }],
        );
        e.at = (chrono::Utc::now() - chrono::Duration::minutes(minutes_ago))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        runlog::record(home, &e).unwrap();
    }

    fn overdue(s: &Status) -> bool {
        s.warnings
            .iter()
            .any(|w| w.kind == WarningKind::SweepOverdue)
    }

    #[test]
    fn status_warns_when_the_timer_has_not_swept_for_twice_its_interval() {
        let mut config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        config.schedule.sweep_every_minutes = 60;
        let (_dir, home) = home_with(&config);

        swept(&home, 100, true);
        assert!(!overdue(&status(&home).unwrap()));

        let (_dir, home) = home_with(&config);
        swept(&home, 125, true);
        let s = status(&home).unwrap();
        assert!(overdue(&s), "{s:?}");
        let w = s
            .warnings
            .iter()
            .find(|w| w.kind == WarningKind::SweepOverdue)
            .unwrap();
        assert_eq!(w.harness, None);
        assert!(w.message.contains("60 minutes"), "{}", w.message);

        // A sweep that captured all but one file still swept.
        swept(&home, 5, false);
        assert!(!overdue(&status(&home).unwrap()));
    }

    #[test]
    fn a_home_set_up_long_ago_that_never_swept_is_overdue_too() {
        let config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        let (_dir, home) = home_with(&config);
        // Just set up: the timer hasn't had its chance yet.
        assert!(!overdue(&status(&home).unwrap()));

        let f = std::fs::File::options()
            .write(true)
            .open(home.config_path())
            .unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 3600))
            .unwrap();
        let s = status(&home).unwrap();
        assert!(overdue(&s), "{s:?}");
        assert!(s.warnings[0].message.contains("3 hours"), "{s:?}");
    }

    #[test]
    fn status_carries_the_exposure_check_and_warns_on_each_failing_rule() {
        let config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        let (_dir, home) = home_with(&config);
        let s = status(&home).unwrap();
        assert_eq!(s.exposure.rules.len(), 4);
        assert!(s.exposure.passes(), "{:?}", s.exposure);
        assert!(!s.warnings.iter().any(|w| w.kind == WarningKind::Exposure));

        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(home.config_path(), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        let s = status(&home).unwrap();
        assert!(!s.exposure.passes());
        let w: Vec<_> = s
            .warnings
            .iter()
            .filter(|w| w.kind == WarningKind::Exposure)
            .collect();
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(
            w[0].message.contains("distill exposure --fix"),
            "{}",
            w[0].message
        );
    }

    fn prune_warning(s: &Status, kind: Harness) -> Option<&Warning> {
        s.warnings
            .iter()
            .find(|w| w.kind == WarningKind::PrunesSooner && w.harness == Some(kind))
    }

    #[test]
    fn status_warns_when_claude_code_deletes_sessions_sooner_than_the_sweep_runs() {
        let dir = tempfile::tempdir().unwrap();
        // Claude Code keeps its settings beside its projects folder.
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(claude.join("projects")).unwrap();
        let mut config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        let h = config.harness_mut(Harness::ClaudeCode);
        h.enabled = true;
        h.roots = vec![claude.join("projects")];
        config.schedule.sweep_every_minutes = 2 * 24 * 60;
        let (_home_dir, home) = home_with(&config);

        // No setting: the default thirty days is longer than two days.
        assert_eq!(
            prune_warning(&status(&home).unwrap(), Harness::ClaudeCode),
            None
        );

        std::fs::write(
            claude.join("settings.json"),
            r#"{"model":"x","cleanupPeriodDays":1}"#,
        )
        .unwrap();
        let s = status(&home).unwrap();
        let w = prune_warning(&s, Harness::ClaudeCode).expect("warns");
        assert!(w.message.contains("cleanupPeriodDays"), "{}", w.message);
        let c = s
            .harnesses
            .iter()
            .find(|h| h.harness == Harness::ClaudeCode)
            .unwrap();
        assert_eq!(c.health, Health::Warn);

        std::fs::write(claude.join("settings.json"), r#"{"cleanupPeriodDays":3}"#).unwrap();
        assert_eq!(
            prune_warning(&status(&home).unwrap(), Harness::ClaudeCode),
            None
        );
    }
}
