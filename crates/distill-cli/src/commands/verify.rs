use anyhow::{Result, bail};
use distill_core::archive::Archive;
use distill_core::runlog::{self, RunEntry, StepOutcome};

use super::Ctx;
use crate::output::{human_bytes, out};

#[derive(clap::Args)]
pub struct Args {}

pub fn run(ctx: &Ctx, _: Args) -> Result<()> {
    let r = Archive::open(&ctx.home)?.verify()?;
    let summary = format!(
        "Verified {} captures: {} missing, {} corrupt",
        r.checked,
        r.missing.len(),
        r.corrupt.len()
    );
    let step = StepOutcome {
        name: "verify".into(),
        ok: r.ok(),
        summary: summary.clone(),
        report: serde_json::to_value(&r)?,
        errors: Vec::new(),
    };
    runlog::record(&ctx.home, &RunEntry::of("verify", vec![step]))?;
    out(ctx.json, &r, || {
        println!(
            "Checked {} captures ({}): {} missing, {} corrupt",
            r.checked,
            human_bytes(r.bytes),
            r.missing.len(),
            r.corrupt.len()
        )
    })?;
    if !r.ok() {
        bail!("archive verification failed");
    }
    Ok(())
}
