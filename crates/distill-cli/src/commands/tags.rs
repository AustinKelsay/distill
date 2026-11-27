//! `distill tags`: the tags on the matching sessions' cards, each with how many sessions carry
//! it, most used first, for a model at a named provider. Tags are written from session text, so
//! they come only through the read path, which logs the read.

use anyhow::{Result, bail};
use distill_core::read::{self, Tagged};
use distill_core::runtime::LocalRuntime;
use distill_core::trust::Reader;

use super::{Ctx, FilterArgs};
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// The provider the reading model runs at (default: $DISTILL_PROVIDER)
    #[arg(long)]
    provider: Option<String>,
    /// The reading model, when known (default: $DISTILL_MODEL)
    #[arg(long)]
    model: Option<String>,
    #[command(flatten)]
    filter: FilterArgs,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let reader = Reader::from_flags_or_env(a.provider.as_deref(), a.model.as_deref());
    let filter = a.filter.into_filter()?;
    match read::tags(&ctx.home, &LocalRuntime::ollama(), &reader, &filter)? {
        Tagged::Text(text) => out(ctx.json, &serde_json::json!({ "text": text }), || {
            print!("{text}")
        }),
        Tagged::Refused(why) => bail!("refused: {why}"),
        // Only the owner and DISTILL itself get counts, and neither reads from here.
        Tagged::Counts(_) => bail!("this reader is not served on the command line"),
    }
}
