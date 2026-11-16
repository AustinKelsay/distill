//! `distill reads`: the read log, newest first. It holds no session text.

use anyhow::Result;
use distill_core::readlog::{self, ReadResult};

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// Only this reader's reads: `owner`, `distill`, a provider, or `provider/model`
    #[arg(long)]
    reader: Option<String>,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let log = readlog::list(&ctx.home, a.reader.as_deref(), a.limit)?;
    out(ctx.json, &log, || {
        for e in &log {
            let result = match &e.result {
                ReadResult::Served { events } => {
                    format!("served {}", distill_core::util::count(*events, "event"))
                }
                ReadResult::Refused { reason } => format!("refused: {reason}"),
                ReadResult::Failed { error } => format!("failed: {error}"),
            };
            println!(
                "{}  {:<28} {:<36} {result}",
                e.at,
                e.reader.to_string(),
                e.session
            );
        }
    })
}
