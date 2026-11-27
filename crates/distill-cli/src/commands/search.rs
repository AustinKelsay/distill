//! `distill search`: sessions that have every word of a query, or are close to it in meaning, for
//! a model at a named provider, best first in one list, with each one's facts, card (or opening)
//! and matching passage. Without an `embedding_model` or a local runtime it finds by words alone
//! and says so. Prints session text, and only through the read path.

use anyhow::{Result, bail};
use distill_core::read::{self, PAGE_MAX, Page, SEARCH_DEFAULT, Searched};
use distill_core::runtime::LocalRuntime;
use distill_core::trust::Reader;

use super::{Ctx, FilterArgs};
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// What to find: a session that has every word (ignoring case), or one close in meaning
    #[arg(required = true)]
    words: Vec<String>,
    /// The provider the reading model runs at (default: $DISTILL_PROVIDER)
    #[arg(long)]
    provider: Option<String>,
    /// The reading model, when known (default: $DISTILL_MODEL)
    #[arg(long)]
    model: Option<String>,
    #[command(flatten)]
    filter: FilterArgs,
    /// How many matching sessions to skip
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// How many sessions at most, up to 50
    #[arg(long, default_value_t = SEARCH_DEFAULT)]
    limit: usize,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let reader = Reader::from_flags_or_env(a.provider.as_deref(), a.model.as_deref());
    let filter = a.filter.into_filter()?;
    if a.limit == 0 || a.limit > PAGE_MAX {
        bail!("--limit is from 1 to {PAGE_MAX}");
    }
    let page = Page {
        offset: a.offset,
        limit: a.limit,
    };
    let query = a.words.join(" ");
    match read::search(
        &ctx.home,
        &LocalRuntime::ollama(),
        &reader,
        &query,
        &filter,
        page,
    )? {
        Searched::Text(text) => out(ctx.json, &serde_json::json!({ "text": text }), || {
            print!("{text}")
        }),
        Searched::Refused(why) => bail!("refused: {why}"),
        // Only the owner and DISTILL itself get hits, and neither searches from here.
        Searched::Hits(_) => bail!("this reader is not served on the command line"),
    }
}
