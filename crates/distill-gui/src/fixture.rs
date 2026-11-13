//! Fixture data for scenes (`--snap --scene`) and the headless tests. Nothing here touches disk.

use std::collections::BTreeMap;

use chrono::{Duration, NaiveDate, Utc};
use distill_core::Home;
use distill_core::config::Config;
use distill_core::runlog::RunEntry;
use distill_core::source::{Detection, SourceKind};
use distill_core::stats::{Dashboard, Health, NeedsYou, SourceHealth, Totals, WeekBucket};

use crate::{Screen, State};

/// A state that loads nothing: `config` None opens on Setup.
pub fn state(config: Option<Config>) -> State {
    let mut s = State::new(Home::new("~/distill"), config);
    s.fixture = true;
    s
}

pub fn detections() -> Vec<Detection> {
    let found = [
        (SourceKind::ClaudeCode, true, 1_840_000_000, 4_212),
        (SourceKind::Codex, true, 642_000_000, 1_103),
        (SourceKind::OpenCode, true, 92_400_000, 1),
        (SourceKind::Droid, false, 0, 0),
        (SourceKind::Pi, true, 12_300_000, 48),
        (SourceKind::Cursor, true, 210_000_000, 3),
    ];
    found
        .into_iter()
        .map(|(kind, found, bytes, files)| Detection {
            kind,
            roots: kind.default_roots(),
            found,
            bytes,
            files,
            has_parser: kind.has_parser(),
        })
        .collect()
}

/// Twelve Mondays from 2026-08-24, each with `counts(week, source)` sessions.
fn weeks(counts: impl Fn(usize, usize) -> u64) -> Vec<WeekBucket> {
    let first = NaiveDate::from_ymd_opt(2026, 8, 24).expect("valid date");
    (0..12)
        .map(|i| {
            let mut by_source = BTreeMap::new();
            for (j, kind) in SourceKind::ALL.into_iter().enumerate() {
                let n = counts(i, j);
                if n > 0 {
                    by_source.insert(kind, n);
                }
            }
            let total = by_source.values().sum();
            WeekBucket {
                week_start: (first + Duration::weeks(i as i64)).to_string(),
                by_source,
                total,
            }
        })
        .collect()
}

fn run(minutes_ago: i64, step: &str, ok: bool, summary: &str, cost: Option<f64>) -> RunEntry {
    let mut e = RunEntry::new(step, ok, summary);
    e.at = (Utc::now() - Duration::minutes(minutes_ago)).to_rfc3339();
    e.cost_usd = cost;
    e
}

fn health(
    kind: SourceKind,
    health: Health,
    sessions: u64,
    bytes: u64,
    failures: u64,
    note: Option<&str>,
) -> SourceHealth {
    SourceHealth {
        kind,
        label: kind.label().to_string(),
        health,
        sessions,
        archive_bytes: bytes,
        failures,
        has_parser: kind.has_parser(),
        note: note.map(str::to_string),
    }
}

/// A busy archive: twelve weeks of sessions, a dataset waiting and two sources needing work.
pub fn dashboard_data() -> Dashboard {
    // Claude Code dominant and growing, Codex steady, the rest occasional.
    let pattern = |i: usize, j: usize| -> u64 {
        let i = i as u64;
        match j {
            0 => 38 + i * 6 + (i * 7) % 11,
            1 => 14 + (i * 5) % 9,
            2 => [0, 3, 5, 2, 0, 4, 6, 3, 8, 5, 7, 9][i as usize],
            4 => {
                if i >= 7 {
                    2 + i % 3
                } else {
                    0
                }
            }
            5 => [2, 0, 1, 0, 3, 1, 0, 2, 0, 1, 2, 1][i as usize],
            _ => 0,
        }
    };
    let weeks = weeks(pattern);
    let sessions: u64 = 2_610 + weeks.iter().map(|w| w.total).sum::<u64>();
    Dashboard {
        totals: Totals {
            sessions,
            archive_bytes: 2_610_000_000,
            enriched: 0.83,
            datasets: 3,
            spent_today_usd: 0.42,
        },
        weeks,
        sources: vec![
            health(
                SourceKind::ClaudeCode,
                Health::Ok,
                2_288,
                1_640_000_000,
                0,
                None,
            ),
            health(SourceKind::Codex, Health::Ok, 861, 598_000_000, 0, None),
            health(
                SourceKind::OpenCode,
                Health::Warn,
                112,
                92_400_000,
                2,
                Some("2 failed to normalize"),
            ),
            health(
                SourceKind::Droid,
                Health::Missing,
                0,
                0,
                0,
                Some("not found on this machine"),
            ),
            health(SourceKind::Pi, Health::Ok, 31, 12_300_000, 0, None),
            health(
                SourceKind::Cursor,
                Health::Warn,
                0,
                210_000_000,
                0,
                Some("archived, no parser"),
            ),
        ],
        recent_runs: vec![
            run(
                12,
                "sweep",
                true,
                "Captured 14 new sessions and 3 changed ones (82 MB)",
                None,
            ),
            run(
                41,
                "steward",
                true,
                "Wrote cards for 9 sessions; tagged 2 as steward runs",
                Some(0.31),
            ),
            run(
                42,
                "normalize",
                true,
                "Normalized 17 sessions; 2 OpenCode captures failed",
                None,
            ),
            run(73, "sweep", true, "Captured 6 new sessions (19 MB)", None),
            run(
                190,
                "export",
                true,
                "Built dataset \"agentic-rust\": 412 records, 37 redactions",
                None,
            ),
            run(
                260,
                "steward",
                true,
                "Wrote cards for 4 sessions",
                Some(0.11),
            ),
            run(330, "sweep", true, "Nothing new", None),
        ],
        needs_you: vec![
            NeedsYou::ApproveExport {
                recipe: "agentic-rust".to_string(),
                build: "20261113-1402".to_string(),
                records: 412,
                redactions: 37,
            },
            NeedsYou::ParserFailures {
                source: SourceKind::OpenCode,
                failures: 2,
            },
            NeedsYou::NoParser {
                source: SourceKind::Cursor,
            },
        ],
        last_sweep: Some((Utc::now() - Duration::minutes(12)).to_rfc3339()),
    }
}

/// A fresh install: set up, nothing swept yet.
pub fn empty_dashboard_data() -> Dashboard {
    Dashboard {
        totals: Totals::default(),
        weeks: weeks(|_, _| 0),
        sources: SourceKind::ALL
            .into_iter()
            .map(|k| match k {
                SourceKind::Droid => health(k, Health::Disabled, 0, 0, 0, None),
                _ => health(k, Health::Ok, 0, 0, 0, None),
            })
            .collect(),
        recent_runs: Vec::new(),
        needs_you: Vec::new(),
        last_sweep: None,
    }
}

fn setup_at(s: &mut State, step: usize) {
    *s = state(None);
    s.detections = Some(detections());
    s.setup.step = step;
}

pub fn setup_sources(s: &mut State) {
    setup_at(s, 0);
}

pub fn setup_agent(s: &mut State) {
    setup_at(s, 2);
}

pub fn setup_privacy(s: &mut State) {
    setup_at(s, 3);
    s.setup.config.privacy.exclude_paths = vec![
        "~/work/client-acme".to_string(),
        "~/personal/journal".to_string(),
    ];
}

pub fn setup_schedule(s: &mut State) {
    setup_at(s, 4);
    s.schedule_installed = Some(false);
}

pub fn dashboard(s: &mut State) {
    *s = state(Some(Config::default()));
    s.screen = Screen::Dashboard;
    s.dash.data = Some(dashboard_data());
    s.dash.loading = false;
}

pub fn dashboard_empty(s: &mut State) {
    *s = state(Some(Config::default()));
    s.screen = Screen::Dashboard;
    s.dash.data = Some(empty_dashboard_data());
    s.dash.loading = false;
}
