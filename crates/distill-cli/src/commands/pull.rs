//! `distill pull`: on a hub, take each spoke's new captures over SSH, as the tick does
//! (ADR 0010).

use anyhow::{Result, bail};
use distill_core::machines;
use distill_core::runlog::{self, RunEntry};
use distill_core::tick::{Deadline, pull_outcome};

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// Pull only this machine
    machine: Option<String>,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let config = ctx.home.load_config()?;
    let r = machines::pull(&ctx.home, &config, a.machine.as_deref(), Deadline::NONE)?;
    runlog::record(&ctx.home, &RunEntry::of("pull", vec![pull_outcome(&r)?]))?;
    out(ctx.json, &r, || println!("{}", r.summary()))?;
    if !r.ok() {
        bail!("a pull failed");
    }
    Ok(())
}
