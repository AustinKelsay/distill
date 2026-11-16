//! `distill scan`: scan every session whose findings are missing or out of date, as the tick
//! does. Prints counts, never what was found. With no config, or one that can't be read, the
//! owner has no `personal_terms`, as on the read path.

use anyhow::Result;
use distill_core::findings;
use distill_core::runlog::{self, RunEntry};
use distill_core::tick::scan_outcome;

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {}

pub fn run(ctx: &Ctx, _: Args) -> Result<()> {
    let config = ctx.home.load_config().unwrap_or_default();
    let r = findings::scan_all(&ctx.home, &config)?;
    runlog::record(&ctx.home, &RunEntry::of("scan", vec![scan_outcome(&r)?]))?;
    out(ctx.json, &r, || {
        println!("{}", r.summary());
        for e in &r.errors {
            println!("  {e}");
        }
    })
}
