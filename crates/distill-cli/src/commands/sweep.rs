use anyhow::{Result, bail};
use distill_core::runlog::{self, RunEntry, StepOutcome};
use distill_core::sweep;

use super::{Ctx, parse_harness};
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// Sweep only this harness
    #[arg(long)]
    harness: Option<String>,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let only = a.harness.as_deref().map(parse_harness).transpose()?;
    let r = sweep::sweep(&ctx.home, &ctx.home.load_config()?, only)?;
    let step = StepOutcome {
        name: "sweep".into(),
        ok: r.errors.is_empty(),
        summary: r.summary(),
        report: serde_json::to_value(&r)?,
        errors: r.errors.clone(),
    };
    runlog::record(&ctx.home, &RunEntry::of("sweep", vec![step]))?;
    out(ctx.json, &r, || {
        println!("{}", r.summary());
        for e in r.errors.iter().take(10) {
            eprintln!("  {e}");
        }
    })?;
    if !r.errors.is_empty() {
        bail!("{} files failed to capture", r.errors.len());
    }
    Ok(())
}
