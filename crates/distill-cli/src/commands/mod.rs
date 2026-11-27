//! One module per command. Each has an `Args` and a `run`.

pub mod backup;
pub mod export;
pub mod exposure;
pub mod findings;
pub mod gui;
pub mod harnesses;
pub mod init;
pub mod parse;
pub mod providers;
pub mod pull;
pub mod read;
pub mod reads;
pub mod restore;
pub mod reveal;
pub mod runs;
pub mod scan;
pub mod schedule;
pub mod search;
pub mod status;
pub mod sweep;
pub mod tag;
pub mod tags;
pub mod tick;
pub mod verify;

use anyhow::{Context, Result};
use distill_core::cards::Outcome;
use distill_core::read::{Filter, date as parse_date};
use distill_core::{Harness, Home};

/// What every command gets.
pub struct Ctx {
    pub home: Home,
    pub json: bool,
}

/// Shared session filter flags for `search` and `tags`.
#[derive(clap::Args, Debug, Clone)]
pub struct FilterArgs {
    /// Only this harness's sessions
    #[arg(long)]
    pub harness: Option<String>,
    /// Only this machine's sessions: a spoke by its name, or `hub` for this hub's own
    #[arg(long)]
    pub machine: Option<String>,
    /// Only sessions that worked in this folder or one inside it
    #[arg(long)]
    pub project: Option<String>,
    /// Only sessions that used a model whose name contains this, ignoring case
    #[arg(long = "used-model")]
    pub used_model: Option<String>,
    /// Only sessions that started on or after this day, YYYY-MM-DD (UTC)
    #[arg(long)]
    pub since: Option<String>,
    /// Only sessions that started on or before this day, YYYY-MM-DD (UTC)
    #[arg(long)]
    pub until: Option<String>,
    /// Include mirrors of sessions already in the archive
    #[arg(long)]
    pub mirrors: bool,
    /// Include sessions the previous build's steward ran
    #[arg(long)]
    pub steward: bool,
    /// Only sessions whose card has this tag, ignoring case; repeat for sessions with all of them
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Leave out sessions whose card has this tag, ignoring case; repeat for any of them
    #[arg(long = "exclude-tag")]
    pub exclude_tags: Vec<String>,
    /// Leave out sessions that worked in this folder or one inside it; repeat for several
    #[arg(long = "exclude-project")]
    pub exclude_projects: Vec<String>,
    /// Leave out this harness's sessions; repeat for several
    #[arg(long = "exclude-harness")]
    pub exclude_harnesses: Vec<String>,
    /// Only sessions with (`--has-reasoning` / `--has-reasoning=true`) or without (`--has-reasoning=false`) reasoning events
    #[arg(
        long,
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true
    )]
    pub has_reasoning: Option<bool>,
    /// Only sessions with at least this many owner messages
    #[arg(long)]
    pub min_turns: Option<u64>,
    /// Only sessions with at most this many owner messages
    #[arg(long)]
    pub max_turns: Option<u64>,
    /// Only sessions with at least this many tool calls
    #[arg(long)]
    pub min_tool_calls: Option<u64>,
    /// Only sessions whose total content bytes are at most this many
    #[arg(long)]
    pub max_bytes: Option<u64>,
    /// Only sessions that called this tool, ignoring case; repeat for all of them
    #[arg(long = "tool")]
    pub tools: Vec<String>,
    /// Only sessions whose card has this outcome: done, partly_done, failed or unknown
    #[arg(long)]
    pub outcome: Option<String>,
}

impl FilterArgs {
    pub fn into_filter(self) -> Result<Filter> {
        let day = |d: &Option<String>| -> Result<_> {
            d.as_deref()
                .map(|d| {
                    parse_date(d).map_err(|_| anyhow::anyhow!("a day is YYYY-MM-DD, not {d:?}"))
                })
                .transpose()
        };
        let exclude_harnesses = self
            .exclude_harnesses
            .iter()
            .map(|h| parse_harness(h))
            .collect::<Result<Vec<_>>>()?;
        Ok(Filter {
            harness: self.harness.as_deref().map(parse_harness).transpose()?,
            machine: self.machine,
            project: self.project,
            model: self.used_model,
            since: day(&self.since)?,
            until: day(&self.until)?,
            mirrors: self.mirrors,
            steward: self.steward,
            tags: self.tags,
            exclude_tags: self.exclude_tags,
            exclude_projects: self.exclude_projects,
            exclude_harnesses,
            has_reasoning: self.has_reasoning,
            min_turns: self.min_turns,
            max_turns: self.max_turns,
            min_tool_calls: self.min_tool_calls,
            max_bytes: self.max_bytes,
            tools: self.tools,
            outcome: self
                .outcome
                .as_deref()
                .map(|o| {
                    Outcome::parse(o).ok_or_else(|| {
                        anyhow::anyhow!(
                            "unknown outcome {o:?}: done, partly_done, failed or unknown"
                        )
                    })
                })
                .transpose()?,
            session: None,
        })
    }
}

/// A harness named on the command line.
pub fn parse_harness(s: &str) -> Result<Harness> {
    Harness::parse(s).with_context(|| {
        format!(
            "unknown harness {s:?}; one of {}",
            Harness::ALL.map(|k| k.id()).join(", ")
        )
    })
}
