//! `distill reads`: the read log, newest first. It holds no session text.

use anyhow::Result;
use distill_core::read;
use distill_core::readlog::{self, LogFilter, ReadResult};

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// Only this reader's reads: `owner`, `distill`, a provider, or `provider/model`
    #[arg(long)]
    reader: Option<String>,
    /// Only lines written on or after this day, YYYY-MM-DD (UTC)
    #[arg(long)]
    since: Option<String>,
    /// Only lines written on or before this day, YYYY-MM-DD (UTC)
    #[arg(long)]
    until: Option<String>,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let day = |d: &Option<String>| {
        d.as_deref()
            .map(|d| read::date(d).map_err(|_| anyhow::anyhow!("not a day as YYYY-MM-DD: {d:?}")))
            .transpose()
    };
    let filter = LogFilter {
        reader: a.reader.clone(),
        since: day(&a.since)?,
        until: day(&a.until)?,
    };
    let log = readlog::find(&ctx.home, &filter, a.limit)?;
    out(ctx.json, &log, || {
        for e in &log {
            println!(
                "{}  {:<28} {:<36} {}",
                e.at,
                e.reader.to_string(),
                e.session.as_deref().unwrap_or(match e.result {
                    ReadResult::Listed { .. } => "(a list of sessions)",
                    ReadResult::StreamStarted { .. } | ReadResult::StreamCompleted { .. } => {
                        "(structured stream)"
                    }
                    ReadResult::Findings { .. } => "(findings)",
                    ReadResult::Tags { .. } => "(tags)",
                    _ => "-",
                }),
                e.result.text()
            );
        }
    })
}
