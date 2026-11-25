use anyhow::{Result, bail};
use distill_core::archive::Archive;
use distill_core::readlog;
use distill_core::runlog::{self, RunEntry, StepOutcome};

use super::Ctx;
use crate::output::{human_bytes, out};

#[derive(clap::Args)]
pub struct Args {}

pub fn run(ctx: &Ctx, _: Args) -> Result<()> {
    let r = Archive::open(&ctx.home)?.verify()?;
    // And the read log's chain (#161): the first line that doesn't follow the one before it, or
    // a backup run that found the log rewritten.
    let chain = readlog::verify(&ctx.home)?;
    let edited = readlog::said(&ctx.home, &chain)?;
    let summary = format!(
        "Verified {} captures: {} missing, {} corrupt",
        r.checked,
        r.missing.len(),
        r.corrupt.len()
    );
    let mut report = serde_json::to_value(&r)?;
    report["read_log"] = serde_json::json!({
        "lines": chain.lines,
        "chained": chain.chained,
        "broken": chain.broken,
        "edited": edited,
    });
    let step = StepOutcome {
        name: "verify".into(),
        ok: r.ok() && edited.is_none(),
        summary: summary.clone(),
        report: report.clone(),
        errors: Vec::new(),
    };
    runlog::record(&ctx.home, &RunEntry::of("verify", vec![step]))?;
    out(ctx.json, &report, || {
        println!(
            "Checked {} captures ({}): {} missing, {} corrupt",
            r.checked,
            human_bytes(r.bytes),
            r.missing.len(),
            r.corrupt.len()
        );
        match &edited {
            Some(e) => println!("{e}"),
            None => println!(
                "Read log: {} lines, {} of them chained; no edit seen",
                chain.lines, chain.chained
            ),
        }
    })?;
    if !r.ok() {
        bail!("archive verification failed");
    }
    if edited.is_some() {
        bail!("the read log has been edited");
    }
    Ok(())
}
