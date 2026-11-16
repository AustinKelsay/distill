use anyhow::Result;
use distill_core::normalize;
use distill_core::runlog::{self, RunEntry};
use distill_core::tick::parse_outcome;

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// Parse every capture again, even unchanged ones
    #[arg(long)]
    force: bool,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let r = normalize::normalize(&ctx.home, a.force)?;
    runlog::record(&ctx.home, &RunEntry::of("parse", vec![parse_outcome(&r)?]))?;
    out(ctx.json, &r, || println!("{}", r.summary()))
}
