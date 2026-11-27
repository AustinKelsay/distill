//! `distill tags`: the tags on the matching sessions' cards, each with how many sessions carry
//! it, most used first, for a model at a named provider. Tags are written from session text, so
//! they come only through the read path, which logs the read.

use anyhow::{Result, bail};
use distill_core::cards::Outcome;
use distill_core::read::{self, Filter, Tagged};
use distill_core::runtime::LocalRuntime;
use distill_core::trust::Reader;

use super::{Ctx, parse_harness};
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// The provider the reading model runs at (default: $DISTILL_PROVIDER)
    #[arg(long)]
    provider: Option<String>,
    /// The reading model, when known (default: $DISTILL_MODEL)
    #[arg(long)]
    model: Option<String>,
    /// Only this harness's sessions
    #[arg(long)]
    harness: Option<String>,
    /// Only this machine's sessions: a spoke by its name, or `hub` for this hub's own
    #[arg(long)]
    machine: Option<String>,
    /// Only sessions that worked in this folder or one inside it
    #[arg(long)]
    project: Option<String>,
    /// Only sessions that used a model whose name contains this, ignoring case
    #[arg(long = "used-model")]
    used_model: Option<String>,
    /// Only sessions that started on or after this day, YYYY-MM-DD (UTC)
    #[arg(long)]
    since: Option<String>,
    /// Only sessions that started on or before this day, YYYY-MM-DD (UTC)
    #[arg(long)]
    until: Option<String>,
    /// Include mirrors of sessions already in the archive
    #[arg(long)]
    mirrors: bool,
    /// Include sessions the previous build's steward ran
    #[arg(long)]
    steward: bool,
    /// Only sessions whose card has this tag: the tags they carry with it; repeat for all of them
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// Only sessions whose card has this outcome: done, partly_done, failed or unknown
    #[arg(long)]
    outcome: Option<String>,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let reader = Reader::from_flags_or_env(a.provider.as_deref(), a.model.as_deref());
    let day = |d: &Option<String>| -> Result<_> {
        d.as_deref()
            .map(|d| read::date(d).map_err(|_| anyhow::anyhow!("a day is YYYY-MM-DD, not {d:?}")))
            .transpose()
    };
    let filter = Filter {
        harness: a.harness.as_deref().map(parse_harness).transpose()?,
        machine: a.machine,
        project: a.project,
        model: a.used_model,
        since: day(&a.since)?,
        until: day(&a.until)?,
        mirrors: a.mirrors,
        steward: a.steward,
        tags: a.tags,
        outcome: a
            .outcome
            .as_deref()
            .map(|o| {
                Outcome::parse(o).ok_or_else(|| {
                    anyhow::anyhow!("unknown outcome {o:?}: done, partly_done, failed or unknown")
                })
            })
            .transpose()?,
        session: None,
    };
    match read::tags(&ctx.home, &LocalRuntime::ollama(), &reader, &filter)? {
        Tagged::Text(text) => out(ctx.json, &serde_json::json!({ "text": text }), || {
            print!("{text}")
        }),
        Tagged::Refused(why) => bail!("refused: {why}"),
        // Only the owner and DISTILL itself get counts, and neither reads from here.
        Tagged::Counts(_) => bail!("this reader is not served on the command line"),
    }
}
