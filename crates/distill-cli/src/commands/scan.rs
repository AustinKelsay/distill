//! `distill scan`: scan every session whose findings are missing or out of date, as the tick
//! does. Prints counts, never what was found.

use anyhow::Result;
use distill_core::findings;
use distill_core::runlog::{self, RunEntry};
use distill_core::tick::scan_outcome;

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {}

pub fn run(ctx: &Ctx, _: Args) -> Result<()> {
    let r = findings::scan_all(&ctx.home)?;
    runlog::record(&ctx.home, &RunEntry::of("scan", vec![scan_outcome(&r)?]))?;
    out(ctx.json, &r, || {
        println!("{}", r.summary());
        for e in &r.errors {
            println!("  {e}");
        }
    })
}
