//! `distill list`: sessions matching filters, newest first, for a model at a named provider.
//! Through the read path, like every command that prints session facts or text.

use anyhow::{Result, bail};
use distill_core::read::{self, Listed, ListedKeys, PAGE_MAX, Page};
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
    /// How many matching sessions to skip
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// How many sessions at most, up to 50 (default: 50; cannot combine with --all)
    #[arg(long)]
    limit: Option<usize>,
    /// Return every matching session, without the 50-row page limit
    #[arg(long)]
    all: bool,
    /// Print one session key per line for piping into `distill read --format jsonl --stdin`
    #[arg(long)]
    keys: bool,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    if a.all && a.limit.is_some() {
        bail!("--all and --limit cannot be combined");
    }
    let page = if a.all {
        None
    } else {
        let limit = a.limit.unwrap_or(PAGE_MAX);
        if limit == 0 || limit > PAGE_MAX {
            bail!("--limit is from 1 to {PAGE_MAX}");
        }
        Some(Page {
            offset: a.offset,
            limit,
        })
    };
    let reader = Reader::from_flags_or_env(a.provider.as_deref(), a.model.as_deref());
    let filter = a.filter.into_filter()?;
    if a.keys {
        return match read::list_keys(
            &ctx.home,
            &LocalRuntime::ollama(),
            &reader,
            &filter,
            a.offset,
            page.map(|p| p.limit),
        )? {
            ListedKeys::Keys { keys, total, next } => out(
                ctx.json,
                &serde_json::json!({ "keys": keys, "total": total, "next": next }),
                || {
                    for k in &keys {
                        println!("{k}");
                    }
                },
            ),
            ListedKeys::Refused(why) => bail!("refused: {why}"),
        };
    }
    let listed = match page {
        Some(p) => read::list(&ctx.home, &LocalRuntime::ollama(), &reader, &filter, p)?,
        None => read::list_all(
            &ctx.home,
            &LocalRuntime::ollama(),
            &reader,
            &filter,
            a.offset,
        )?,
    };
    match listed {
        Listed::Text(text) => out(ctx.json, &serde_json::json!({ "text": text }), || {
            print!("{text}")
        }),
        Listed::Refused(why) => bail!("refused: {why}"),
        // Only the owner and DISTILL itself get rows, and neither lists from here.
        Listed::Rows(_) => bail!("this reader is not served on the command line"),
    }
}
