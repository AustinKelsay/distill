//! `distill read`: a session's events as text, for a model at a named provider. The one command
//! that prints session text, and only through the read path.

use anyhow::{Result, bail};
use distill_core::read::{self, Range, Read};
use distill_core::runtime::LocalRuntime;
use distill_core::trust::Reader;

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// The session: `harness/id`, or its id alone when only one harness has it
    session: String,
    /// The provider the reading model runs at (default: $DISTILL_PROVIDER)
    #[arg(long)]
    provider: Option<String>,
    /// The reading model, when known (default: $DISTILL_MODEL)
    #[arg(long)]
    model: Option<String>,
    /// The first event to read, by number
    #[arg(long)]
    from: Option<u64>,
    /// The last event to read, by number
    #[arg(long)]
    to: Option<u64>,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let reader = Reader::from_flags_or_env(a.provider.as_deref(), a.model.as_deref());
    let range = Range {
        from: a.from,
        to: a.to,
    };
    match read::read(
        &ctx.home,
        &LocalRuntime::ollama(),
        &reader,
        &a.session,
        range,
    )? {
        Read::Text(text) => out(ctx.json, &serde_json::json!({ "text": text }), || {
            print!("{text}")
        }),
        Read::Refused(why) => bail!("refused: {why}"),
        // Only the owner and Distill itself get events, and neither reads from here.
        Read::Events(_) => bail!("this reader is not served on the command line"),
    }
}
