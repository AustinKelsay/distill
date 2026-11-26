//! Status: one report built here, which `distill status` prints and the GUI overview renders.
//! Today it covers sessions and capture health per harness, format drift, the last sweep, the
//! scan's, the word index's, the cards', the vectors' and the model detection pass's progress, the reveal requests waiting for the owner, warnings
//! when a harness could delete sessions before they are swept or the timer has stopped sweeping,
//! the exposure check's results with a warning for each rule that fails, and on a hub each
//! spoke's captures and pulls (ADR 0010).

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
use crate::normalize::{self, Failures, UnknownType};
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
    /// This machine's own captures, per harness.
    pub harnesses: Vec<HarnessStatus>,
    /// On a hub, each spoke it pulls from or holds captures of (#140).
    #[serde(default)]
    pub machines: Vec<MachineStatus>,
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
    /// How many sessions have current model findings (personal details found by the detail
    /// model), how many wait for the pass, and why it is skipped when it is.
    #[serde(default)]
    pub details: DetailProgress,
    /// Reveal requests waiting for the owner to approve or deny them in the GUI.
    #[serde(default)]
    pub pending_reveals: u64,
    /// Each `[[backup]]` target and where it stands (issue #150).
    #[serde(default)]
    pub backups: Vec<crate::backup::TargetStatus>,
    /// The exposure check's results. Each failing rule is also a warning.
    pub exposure: Exposure,
    /// What left this machine over the last day and the last 7 days, by where it went (#162).
    #[serde(default)]
    pub left: crate::readlog::Left,
    /// The last [`HISTORY_RUNS`] runs, newest first, with runs in a row that came to the same
    /// results folded into one group (issue #160).
    #[serde(default)]
    pub runs: Vec<runlog::RunGroup>,
}

/// How many runs the status report's run history covers: about two days of the hourly timer.
pub const HISTORY_RUNS: usize = 48;

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
    /// A spoke in `[[machines]]` has not been pulled from for longer than twice the interval.
    PullOverdue,
    /// A harness's setting could delete sessions sooner than the timer sweeps.
    PrunesSooner,
    /// A rule of the exposure check fails.
    Exposure,
    /// No backup target is set up, or one has not had a whole run lately.
    Backup,
    /// The read log has been edited: its chain breaks, or a backup run found it rewritten
    /// (`readlog::edited`, issue #161).
    ReadLog,
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
    /// Captures that parsed but hold record types their parser doesn't know: format drift that
    /// drops some records while the rest still parse.
    #[serde(default)]
    pub partial_drift: u64,
    /// Those record types by name (never content), with the captures holding each and the
    /// records and bytes they hold.
    #[serde(default)]
    pub unknown_types: BTreeMap<String, UnknownType>,
    pub note: Option<String>,
}

/// A spoke as a hub holds it: its captures here, per harness, and its pulls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineStatus {
    pub machine: String,
    /// What ssh reaches it by; none when no `[[machines]]` entry names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub health: Health,
    pub sessions: u64,
    /// Files in its raw archive here (each origin's latest capture).
    pub captures: u64,
    pub archive_bytes: u64,
    /// When its newest capture here was taken, on the spoke.
    pub last_capture: Option<String>,
    /// When a pull last took from it without an error.
    pub last_pull: Option<String>,
    /// The newest pull's error, when that pull failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_error: Option<String>,
    /// Its harnesses that have captures here.
    pub harnesses: Vec<HarnessStatus>,
    pub note: Option<String>,
}

/// How many type names a harness's note lists before it counts the rest.
const NOTE_TYPES: usize = 5;

/// The note for a harness's format drift: whole captures that parsed to nothing, and captures
/// holding record types the parser doesn't know, named.
fn drift_note(f: &Failures) -> String {
    let mut parts = Vec::new();
    if f.drift > 0 {
        parts.push(format!("{} look like format drift", f.drift));
    }
    if !f.unknown.is_empty() {
        let mut names: Vec<&str> = f
            .unknown
            .keys()
            .map(String::as_str)
            .take(NOTE_TYPES)
            .collect();
        let more = f.unknown.len().saturating_sub(NOTE_TYPES);
        let more = (more > 0).then(|| format!("{more} more"));
        names.extend(more.as_deref());
        parts.push(format!(
            "record types the parser doesn't know: {}",
            names.join(", ")
        ));
    }
    parts.join("; ")
}

/// Why a harness's captures warn as they parse: format drift, or captures that failed.
fn parse_note(f: &Failures) -> Option<String> {
    if f.drift > 0 || !f.unknown.is_empty() {
        Some(drift_note(f))
    } else if f.failed > 0 {
        Some(format!("{} failed to parse", f.failed))
    } else {
        None
    }
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
        } else if let Some((_, deletes)) = &prunes {
            (
                Health::Warn,
                Some(format!("deletes {deletes} sooner than the sweep runs")),
            )
        } else if let Some(note) = parse_note(&f) {
            (Health::Warn, Some(note))
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
            sessions: sessions
                .iter()
                .filter(|m| m.machine.is_none() && m.harness == kind)
                .count() as u64,
            captures: captured.len() as u64,
            archive_bytes,
            last_capture: captured.iter().map(|r| r.captured_at.clone()).max(),
            capture_errors,
            parse_failures: f.failed,
            drift: f.drift,
            partial_drift: f.partial_drift,
            unknown_types: f.unknown.clone(),
            note,
        });
        warnings.extend(prunes.map(|(w, _)| w));
    }

    let machines = machines(home, config.as_ref(), &sessions)?;
    if let Some(c) = &config {
        warnings.extend(pull_overdue(
            c,
            &machines,
            config_written(home),
            chrono::Utc::now(),
        ));
    }

    let mut backups = Vec::new();
    if let Some(c) = &config {
        match crate::backup::status::status(home, c, archive.records().len() as u64) {
            Ok(b) => {
                warnings.extend(backup_warnings(c, &b, chrono::Utc::now()));
                backups = b;
            }
            Err(e) => warnings.push(Warning {
                kind: WarningKind::Backup,
                harness: None,
                message: format!("Backups can't run: {e:#}"),
            }),
        }
    }

    let edited = crate::readlog::edited(home)
        .unwrap_or_else(|e| Some(format!("The read log can't be checked: {e:#}")));
    if let Some(message) = &edited {
        warnings.push(Warning {
            kind: WarningKind::ReadLog,
            harness: None,
            message: message.clone(),
        });
    }
    let ollama = crate::runtime::LocalRuntime::ollama();
    let mut left = crate::readlog::left(home, chrono::Utc::now(), &|m| ollama.stored_here(m))?;
    left.maybe_short = edited.is_some();

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
        machines,
        warnings,
        scan: findings::progress(home)?,
        index: index::progress(home)?,
        cards: cards::progress(home)?,
        vectors: index::vectors::progress(home)?,
        details: details::progress(home)?,
        pending_reveals: crate::reveal::pending_count(home),
        backups,
        exposure,
        left,
        runs: runlog::history(home, HISTORY_RUNS)?,
    })
}

/// Each spoke in `[[machines]]` or with captures here, by name.
fn machines(
    home: &Home,
    config: Option<&Config>,
    sessions: &[crate::model::SessionMeta],
) -> Result<Vec<MachineStatus>> {
    let mut names: std::collections::BTreeMap<String, Option<String>> =
        home.machines()?.into_iter().map(|m| (m, None)).collect();
    for m in config.map_or(&[][..], |c| &c.machines[..]) {
        names.insert(m.name.clone(), Some(m.host.clone()));
    }
    if names.is_empty() {
        return Ok(Vec::new());
    }
    // Each one's newest pull, and the newest that took without an error.
    let mut newest: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut last_ok: BTreeMap<String, String> = BTreeMap::new();
    for e in runlog::recent(home, 500)? {
        let Some(o) = e.outcome("pull") else { continue };
        let Ok(r) = serde_json::from_value::<crate::machines::PullReport>(o.report) else {
            continue;
        };
        for p in r.machines {
            if p.error.is_none() {
                last_ok
                    .entry(p.machine.clone())
                    .or_insert_with(|| e.at.clone());
            }
            newest.entry(p.machine).or_insert(p.error);
        }
    }
    let mut out = Vec::new();
    for (machine, host) in names {
        let archive = Archive::open_of(home, Some(&machine))?;
        let failures = normalize::failures_of(home, Some(&machine));
        let mine: Vec<_> = sessions
            .iter()
            .filter(|s| s.machine.as_deref() == Some(machine.as_str()))
            .collect();
        let mut harnesses = Vec::new();
        for kind in Harness::ALL {
            let captured: Vec<_> = archive
                .latest()
                .values()
                .filter(|r| r.harness == kind)
                .collect();
            if captured.is_empty() {
                continue;
            }
            let f = failures.get(&kind).cloned().unwrap_or_default();
            let (health, note) = match parse_note(&f) {
                Some(note) => (Health::Warn, Some(note)),
                None if !kind.has_parser() => (Health::Warn, Some("archived, no parser".into())),
                None => (Health::Ok, None),
            };
            harnesses.push(HarnessStatus {
                harness: kind,
                label: kind.label().to_string(),
                enabled: true,
                health,
                sessions: mine.iter().filter(|s| s.harness == kind).count() as u64,
                captures: captured.len() as u64,
                archive_bytes: captured.iter().map(|r| r.size).sum(),
                last_capture: captured.iter().map(|r| r.captured_at.clone()).max(),
                capture_errors: 0,
                parse_failures: f.failed,
                drift: f.drift,
                partial_drift: f.partial_drift,
                unknown_types: f.unknown.clone(),
                note,
            });
        }
        let pull_error = newest.get(&machine).cloned().flatten();
        let last_pull = last_ok.get(&machine).cloned();
        let warned = harnesses.iter().find(|h| h.health == Health::Warn);
        let (health, note) = if let Some(e) = &pull_error {
            (Health::Warn, Some(format!("the last pull failed: {e}")))
        } else if let Some(h) = warned {
            (
                Health::Warn,
                Some(format!("{}: {}", h.label, h.note.as_deref().unwrap_or(""))),
            )
        } else if last_pull.is_none() && harnesses.is_empty() {
            (Health::Missing, Some("not pulled from yet".into()))
        } else {
            (Health::Ok, None)
        };
        out.push(MachineStatus {
            sessions: mine.len() as u64,
            captures: harnesses.iter().map(|h| h.captures).sum(),
            archive_bytes: harnesses.iter().map(|h| h.archive_bytes).sum(),
            last_capture: harnesses
                .iter()
                .filter_map(|h| h.last_capture.clone())
                .max(),
            machine,
            host,
            health,
            last_pull,
            pull_error,
            harnesses,
            note,
        });
    }
    Ok(out)
}

/// A warning for each spoke in `[[machines]]` that no pull has taken from for longer than twice
/// the timer's interval, counted from when `config.toml` was written if none ever has.
fn pull_overdue(
    config: &Config,
    machines: &[MachineStatus],
    written: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Warning> {
    let every = i64::from(crate::schedule::interval_minutes(config));
    machines
        .iter()
        .filter(|m| m.host.is_some())
        .filter_map(|m| {
            let since = match &m.last_pull {
                Some(at) => chrono::DateTime::parse_from_rfc3339(at)
                    .ok()?
                    .with_timezone(&chrono::Utc),
                None => written?,
            };
            let quiet = (now - since).num_minutes();
            (quiet > 2 * every).then(|| Warning {
                kind: WarningKind::PullOverdue,
                harness: None,
                message: format!(
                    "Nothing has been pulled from {} for {}, and the timer runs every {}: {}.",
                    m.machine,
                    span(quiet),
                    span(every),
                    match &m.pull_error {
                        Some(e) => format!("the last pull failed: {e}"),
                        None => format!(
                            "check that it is on and reachable (`distill pull {}`)",
                            m.machine
                        ),
                    }
                ),
            })
        })
        .collect()
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

/// A warning when `kind`'s own setting deletes sessions (or files of a session) sooner than the
/// timer sweeps, so that one could go before it is captured; with what it deletes.
fn prunes_sooner(kind: Harness, config: &Config) -> Option<(Warning, String)> {
    let every = crate::schedule::interval_minutes(config);
    let r = pruning::retention(kind, &config.roots_for(kind))?;
    (r.minutes < u64::from(every)).then(|| {
        let warning = Warning {
            kind: WarningKind::PrunesSooner,
            harness: Some(kind),
            message: format!(
                "{} deletes {} {} ({}), sooner than the timer sweeps (every {}): {}.",
                kind.label(),
                r.deletes,
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
        };
        (warning, r.deletes.clone())
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

/// A warning when no backup target is set up, and one for each target that has never had a whole
/// run, whose newest run failed, or that has had no whole run for longer than twice the timer's
/// interval.
fn backup_warnings(
    config: &Config,
    targets: &[crate::backup::TargetStatus],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Warning> {
    let warn = |message: String| Warning {
        kind: WarningKind::Backup,
        harness: None,
        message,
    };
    if targets.is_empty() {
        return vec![warn(
            "No backup target is set up in config.toml, so DISTILL keeps no copy of the raw archive. \
             Add one in Setup's Backups step, or add a [[backup]] target to config.toml and run \
             `distill backup init` and `distill backup run`."
                .into(),
        )];
    }
    let every = i64::from(crate::schedule::interval_minutes(config));
    let ago = |at: &str| {
        chrono::DateTime::parse_from_rfc3339(at)
            .ok()
            .map(|t| (now - t.with_timezone(&chrono::Utc)).num_minutes())
    };
    targets
        .iter()
        .filter_map(|t| {
            let since = match &t.last_ok {
                Some(at) => format!("its last backup was {} ago", span(ago(at)?)),
                None => "it has not been backed up to yet".into(),
            };
            Some(warn(match (&t.last_error, &t.last_ok) {
                (Some(e), _) => format!("The last backup to {} failed, and {since}: {e}", t.name),
                (None, None) => format!(
                    "Nothing has been backed up to {} yet: back up to it once (Back up now in the window, or `distill backup run --target {}`), and every scheduled run keeps it current after that.",
                    t.name, t.name
                ),
                (None, Some(at)) if ago(at)? > 2 * every => format!(
                    "Nothing has backed up to {} for {}, and the timer runs every {}: check that it is installed and running (`distill schedule`).",
                    t.name,
                    span(ago(at)?),
                    span(every)
                ),
                _ => return None,
            }))
        })
        .collect()
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

    #[test]
    fn status_names_record_types_a_parser_does_not_know_while_the_rest_parse() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(claude.join("-w")).unwrap();
        // A small capture: one record of a type the parser doesn't know among ones it reads,
        // and a title record, which it ignores on purpose.
        std::fs::write(
            claude.join("-w/s1.jsonl"),
            r#"{"type":"custom-title","customTitle":"Rotate the widget key","sessionId":"s1"}
{"type":"user","cwd":"/w","sessionId":"s1","timestamp":"2026-11-01T00:00:00Z","message":{"role":"user","content":"please rotate the widget key"}}
{"type":"tool-output.v2","timestamp":"2026-11-01T00:00:03Z","text":"the widget key is rotated"}
{"type":"assistant","timestamp":"2026-11-01T00:00:05Z","message":{"role":"assistant","content":[{"type":"text","text":"Rotated."}]}}
"#,
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
        home.ensure().unwrap();
        config.save(&home.config_path()).unwrap();

        crate::tick::tick(&home, &config).unwrap();
        let s = status(&home).unwrap();
        assert_eq!(s.sessions, 1);
        let c = s
            .harnesses
            .iter()
            .find(|h| h.harness == Harness::ClaudeCode)
            .unwrap();
        assert_eq!(c.health, Health::Warn);
        assert_eq!((c.parse_failures, c.drift, c.partial_drift), (0, 0, 1));
        assert_eq!(
            c.note.as_deref(),
            Some("record types the parser doesn't know: tool-output.v2")
        );
        let names: Vec<&str> = c.unknown_types.keys().map(String::as_str).collect();
        assert_eq!(names, ["tool-output.v2"]);
        assert_eq!(c.unknown_types["tool-output.v2"].records, 1);
        // The report names types and counts them; it holds no session text.
        let text = serde_json::to_string(&s).unwrap();
        for words in ["widget", "Rotated", "custom-title"] {
            assert!(!text.contains(words), "{words} in {text}");
        }
    }

    pub(super) fn pulled(home: &Home, machine: &str, minutes_ago: i64, error: Option<&str>) {
        let report = crate::machines::PullReport {
            machines: vec![crate::machines::Pulled {
                machine: machine.into(),
                took: Default::default(),
                error: error.map(Into::into),
            }],
            skipped: None,
        };
        let mut e = runlog::RunEntry::of("tick", vec![crate::tick::pull_outcome(&report).unwrap()]);
        e.at = (chrono::Utc::now() - chrono::Duration::minutes(minutes_ago))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        runlog::record(home, &e).unwrap();
    }

    #[test]
    fn a_hub_reports_each_spokes_captures_per_harness_and_its_pulls_apart_from_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        let config = Config {
            machines: vec![crate::config::MachineConfig {
                name: "nas".into(),
                host: "me@nas".into(),
                command: None,
            }],
            ..Config::default()
        };
        config.save(&home.config_path()).unwrap();
        let capture = br#"{"type":"user","cwd":"/w","sessionId":"s1","timestamp":"2026-11-01T00:00:00Z","message":{"role":"user","content":"hi"}}
"#;
        Archive::open_of(&home, Some("nas"))
            .unwrap()
            .put(Harness::ClaudeCode, "/x/-w/s1.jsonl", "1", &capture[..])
            .unwrap();
        normalize::normalize(&home, false).unwrap();

        let s = status(&home).unwrap();
        assert_eq!(s.sessions, 1);
        // This machine's own row doesn't count the spoke's session.
        let own = s
            .harnesses
            .iter()
            .find(|h| h.harness == Harness::ClaudeCode)
            .unwrap();
        assert_eq!((own.sessions, own.captures), (0, 0), "{own:?}");
        let [nas] = &s.machines[..] else {
            panic!("{:?}", s.machines)
        };
        assert_eq!(
            (nas.machine.as_str(), nas.host.as_deref()),
            ("nas", Some("me@nas"))
        );
        assert_eq!((nas.sessions, nas.captures, nas.health), (1, 1, Health::Ok));
        assert_eq!(nas.last_pull, None);
        let [c] = &nas.harnesses[..] else {
            panic!("{:?}", nas.harnesses)
        };
        assert_eq!(
            (c.harness, c.sessions, c.health),
            (Harness::ClaudeCode, 1, Health::Ok)
        );
        assert!(nas.last_capture.is_some());

        pulled(&home, "nas", 30, None);
        pulled(&home, "nas", 5, Some("ssh me@nas: Connection refused"));
        let s = status(&home).unwrap();
        let nas = &s.machines[0];
        assert!(nas.last_pull.is_some());
        assert_eq!(
            nas.pull_error.as_deref(),
            Some("ssh me@nas: Connection refused")
        );
        assert_eq!(nas.health, Health::Warn);
        assert!(nas.note.as_deref().unwrap().contains("Connection refused"));
    }

    #[test]
    fn a_drift_note_names_a_few_types_and_counts_the_rest() {
        let mut f = Failures {
            failed: 2,
            drift: 2,
            ..Default::default()
        };
        assert_eq!(drift_note(&f), "2 look like format drift");
        for ty in ["a", "b", "c", "d", "e", "f", "g"] {
            f.unknown.insert(ty.into(), UnknownType::default());
        }
        assert_eq!(
            drift_note(&f),
            "2 look like format drift; record types the parser doesn't know: a, b, c, d, e, 2 more"
        );
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

    #[test]
    fn backups_warn_when_none_is_set_up_never_ran_failed_or_fell_behind() {
        let config = Config::default();
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-22T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let t = |name: &str, ok: Option<&str>, error: Option<&str>| crate::backup::TargetStatus {
            name: name.into(),
            kind: "folder".into(),
            last_ok: ok.map(Into::into),
            last_error: error.map(Into::into),
            last_failed: error.map(|_| "2026-11-22T11:59:00Z".into()),
            captures_behind: None,
        };
        let none = backup_warnings(&config, &[], now);
        assert_eq!(none.len(), 1);
        assert!(none[0].message.starts_with("No backup target"), "{none:?}");
        let w = backup_warnings(
            &config,
            &[
                t("current", Some("2026-11-22T11:30:00Z"), None),
                t("never", None, None),
                t(
                    "failed",
                    Some("2026-11-22T11:00:00Z"),
                    Some("the folder is gone"),
                ),
                t("behind", Some("2026-11-21T12:00:00Z"), None),
            ],
            now,
        );
        let messages: Vec<_> = w.iter().map(|w| w.message.as_str()).collect();
        assert_eq!(w.len(), 3, "{messages:?}");
        assert!(w.iter().all(|w| w.kind == WarningKind::Backup));
        assert!(
            messages[0].contains("Nothing has been backed up to never yet: back up to it once (Back up now in the window, or `distill backup run --target never`)"),
            "{messages:?}"
        );
        assert!(
            messages[1]
                .contains("failed, and its last backup was 60 minutes ago: the folder is gone"),
            "{messages:?}"
        );
        assert!(
            messages[2].contains("Nothing has backed up to behind for 24 hours"),
            "{messages:?}"
        );
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

    fn pull_overdue_of(s: &Status) -> Vec<&str> {
        s.warnings
            .iter()
            .filter(|w| w.kind == WarningKind::PullOverdue)
            .map(|w| w.message.as_str())
            .collect()
    }

    #[test]
    fn status_warns_when_a_spoke_has_not_been_pulled_for_twice_the_interval() {
        let mut config = Config {
            harnesses: vec![],
            machines: vec![crate::config::MachineConfig {
                name: "nas".into(),
                host: "me@nas".into(),
                command: None,
            }],
            ..Default::default()
        };
        config.schedule.sweep_every_minutes = 60;
        let (_dir, home) = home_with(&config);
        // Just set up: no pull yet, and none due.
        assert!(pull_overdue_of(&status(&home).unwrap()).is_empty());

        let f = std::fs::File::options()
            .write(true)
            .open(home.config_path())
            .unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 3600))
            .unwrap();
        let s = status(&home).unwrap();
        let w = pull_overdue_of(&s);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(
            w[0].starts_with("Nothing has been pulled from nas for 3 hours"),
            "{w:?}"
        );
        assert!(w[0].contains("`distill pull nas`"), "{w:?}");

        // A pull that took but then failed: the warning counts from the one that took.
        super::tests::pulled(&home, "nas", 125, None);
        super::tests::pulled(&home, "nas", 10, Some("ssh me@nas: Connection refused"));
        let w = pull_overdue_of(&status(&home).unwrap()).join(" ");
        assert!(w.contains("for 2 hours"), "{w}");
        assert!(
            w.contains("the last pull failed: ssh me@nas: Connection refused"),
            "{w}"
        );

        super::tests::pulled(&home, "nas", 5, None);
        assert!(pull_overdue_of(&status(&home).unwrap()).is_empty());
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

    #[test]
    fn status_warns_when_the_sweep_is_too_slow_to_catch_opencodes_spilled_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("opencode");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("opencode.db"), b"").unwrap();
        let mut config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        let h = config.harness_mut(Harness::OpenCode);
        h.enabled = true;
        h.roots = vec![data.join("opencode.db")];
        // A spilled output lasts a week: sweeping every six days catches it.
        config.schedule.sweep_every_minutes = 6 * 24 * 60;
        let (_home_dir, home) = home_with(&config);
        assert_eq!(
            prune_warning(&status(&home).unwrap(), Harness::OpenCode),
            None
        );

        config.schedule.sweep_every_minutes = 8 * 24 * 60;
        let (_home_dir, home) = home_with(&config);
        let s = status(&home).unwrap();
        let w = prune_warning(&s, Harness::OpenCode).expect("warns");
        assert!(
            w.message
                .starts_with("OpenCode deletes spilled tool outputs untouched for 7 days"),
            "{}",
            w.message
        );
        assert!(w.message.contains("tool-output"), "{}", w.message);
        assert!(w.message.ends_with("sweep more often."), "{}", w.message);
        let o = s
            .harnesses
            .iter()
            .find(|h| h.harness == Harness::OpenCode)
            .unwrap();
        assert_eq!(o.health, Health::Warn);
        assert_eq!(
            o.note.as_deref(),
            Some("deletes spilled tool outputs sooner than the sweep runs")
        );
    }
}
