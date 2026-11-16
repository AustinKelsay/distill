use anyhow::Result;
use distill_core::runlog;

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let runs = runlog::recent(&ctx.home, a.limit)?;
    out(ctx.json, &runs, || {
        for r in &runs {
            println!(
                "{} {:<9} {} {}",
                r.at,
                r.step,
                if r.ok { "ok  " } else { "FAIL" },
                r.summary
            );
        }
    })
}
