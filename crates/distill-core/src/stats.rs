//! Everything the dashboard shows, computed from the files on disk.

use std::collections::BTreeMap;

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::archive::Archive;
use crate::config::Config;
use crate::export;
use crate::home::Home;
use crate::normalize;
use crate::runlog::{self, RunEntry};
use crate::source::SourceKind;

pub const WEEKS: usize = 12;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dashboard {
    pub totals: Totals,
    /// Sessions started per week, oldest week first, `WEEKS` long.
    pub weeks: Vec<WeekBucket>,
    pub sources: Vec<SourceHealth>,
    /// Newest first.
    pub recent_runs: Vec<RunEntry>,
    pub needs_you: Vec<NeedsYou>,
    pub last_sweep: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Totals {
    pub sessions: u64,
    pub archive_bytes: u64,
    /// Share of non-steward sessions with a card, 0.0 to 1.0.
    pub enriched: f64,
    pub datasets: u64,
    pub spent_today_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeekBucket {
    /// Monday of the week, YYYY-MM-DD.
    pub week_start: String,
    pub by_source: BTreeMap<SourceKind, u64>,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Ok,
    /// Archived but no parser, or some captures failed to normalize.
    Warn,
    Disabled,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceHealth {
    pub kind: SourceKind,
    pub label: String,
    pub health: Health,
    pub sessions: u64,
    pub archive_bytes: u64,
    pub failures: u64,
    pub has_parser: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NeedsYou {
    /// A dataset build is waiting for approval before it can leave the machine.
    ApproveExport {
        recipe: String,
        build: String,
        records: u64,
        redactions: u64,
    },
    /// A source is archived but nothing can read it yet.
    NoParser { source: SourceKind },
    /// Captures failed to normalize; a parser may need repair.
    ParserFailures { source: SourceKind, failures: u64 },
    /// The last steward run failed.
    StewardFailed { at: String, summary: String },
    /// Setup has not been run.
    NotConfigured,
}

impl NeedsYou {
    pub fn title(&self) -> String {
        match self {
            NeedsYou::ApproveExport {
                recipe, records, ..
            } => {
                format!("Dataset \"{recipe}\" ({records} records) is ready to leave the machine")
            }
            NeedsYou::NoParser { source } => format!("{} has no parser", source.label()),
            NeedsYou::ParserFailures { source, failures } => {
                format!(
                    "{} sessions from {} failed to normalize",
                    failures,
                    source.label()
                )
            }
            NeedsYou::StewardFailed { summary, .. } => format!("Steward run failed: {summary}"),
            NeedsYou::NotConfigured => "Distill is not set up yet".to_string(),
        }
    }
}

pub fn dashboard(home: &Home) -> Result<Dashboard> {
    let config = home.load_config().ok();
    let metas = normalize::read_index(home)?;
    let archive = Archive::open(home)?;
    let latest = archive.latest();
    let failures = normalize::failures(home)?;
    let builds = export::list_builds(home)?;

    let mut totals = Totals {
        sessions: metas.len() as u64,
        ..Default::default()
    };
    totals.archive_bytes = archive.stored_bytes();
    let candidates: Vec<_> = metas.iter().filter(|m| !m.steward).collect();
    let carded = candidates
        .iter()
        .filter(|m| crate::card::load(&normalize::session_dir(home, m)).is_some())
        .count();
    totals.enriched = if candidates.is_empty() {
        0.0
    } else {
        carded as f64 / candidates.len() as f64
    };
    totals.datasets = builds
        .iter()
        .map(|b| b.recipe.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len() as u64;
    totals.spent_today_usd = runlog::spent_today(home)?;

    let weeks = week_buckets(&metas, Utc::now());

    let mut sources = Vec::new();
    for kind in SourceKind::ALL {
        let enabled = config
            .as_ref()
            .and_then(|c| c.source(kind))
            .is_some_and(|s| s.enabled);
        let sessions = metas.iter().filter(|m| m.source == kind).count() as u64;
        let archive_bytes: u64 = latest
            .values()
            .filter(|r| r.source == kind)
            .map(|r| r.size)
            .sum();
        let fails = failures.get(&kind).copied().unwrap_or(0);
        let present = archive_bytes > 0
            || config
                .as_ref()
                .is_some_and(|c| c.roots_for(kind).iter().any(|r| r.exists()));
        let (health, note) = if !enabled {
            (Health::Disabled, None)
        } else if !present {
            (
                Health::Missing,
                Some("not found on this machine".to_string()),
            )
        } else if !kind.has_parser() {
            (Health::Warn, Some("archived, no parser".to_string()))
        } else if fails > 0 {
            (Health::Warn, Some(format!("{fails} failed to normalize")))
        } else {
            (Health::Ok, None)
        };
        sources.push(SourceHealth {
            kind,
            label: kind.label().to_string(),
            health,
            sessions,
            archive_bytes,
            failures: fails,
            has_parser: kind.has_parser(),
            note,
        });
    }

    let recent_runs = runlog::recent(home, 20)?;
    let last_sweep = runlog::last_ok(home, "sweep")?;

    let mut needs_you = Vec::new();
    if config.is_none() {
        needs_you.push(NeedsYou::NotConfigured);
    }
    // Only a recipe's newest build asks for approval; older ones were superseded.
    let mut seen_recipes = std::collections::HashSet::new();
    let newest = builds
        .iter()
        .filter(|b| seen_recipes.insert(b.recipe.clone()));
    for b in newest.filter(|b| !b.approved) {
        needs_you.push(NeedsYou::ApproveExport {
            recipe: b.recipe.clone(),
            build: b.build.clone(),
            records: b.records,
            redactions: b.redactions,
        });
    }
    for s in &sources {
        if s.health == Health::Warn && !s.has_parser && s.archive_bytes > 0 {
            needs_you.push(NeedsYou::NoParser { source: s.kind });
        } else if s.health == Health::Warn && s.failures > 0 {
            needs_you.push(NeedsYou::ParserFailures {
                source: s.kind,
                failures: s.failures,
            });
        }
    }
    if let Some(last) = recent_runs.iter().find(|r| r.step == "steward")
        && !last.ok
    {
        needs_you.push(NeedsYou::StewardFailed {
            at: last.at.clone(),
            summary: last.summary.clone(),
        });
    }

    Ok(Dashboard {
        totals,
        weeks,
        sources,
        recent_runs,
        needs_you,
        last_sweep,
    })
}

fn monday(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

pub fn week_buckets(metas: &[crate::model::SessionMeta], now: DateTime<Utc>) -> Vec<WeekBucket> {
    let this_week = monday(now.date_naive());
    let first = this_week - Duration::weeks(WEEKS as i64 - 1);
    let mut buckets: Vec<WeekBucket> = (0..WEEKS)
        .map(|i| WeekBucket {
            week_start: (first + Duration::weeks(i as i64)).to_string(),
            by_source: BTreeMap::new(),
            total: 0,
        })
        .collect();
    for m in metas {
        let Some(ts) = m.started_at.as_deref() else {
            continue;
        };
        let Ok(t) = DateTime::parse_from_rfc3339(ts) else {
            continue;
        };
        let wk = monday(t.with_timezone(&Utc).date_naive());
        if wk < first || wk > this_week {
            continue;
        }
        let i = ((wk - first).num_days() / 7) as usize;
        *buckets[i].by_source.entry(m.source).or_default() += 1;
        buckets[i].total += 1;
    }
    buckets
}

/// Load the config, or the defaults with detected sources, for the setup screens.
pub fn config_or_default(home: &Home) -> Config {
    home.load_config().unwrap_or_default()
}
