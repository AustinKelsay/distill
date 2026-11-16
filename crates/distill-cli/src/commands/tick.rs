use anyhow::Result;
use distill_core::tick;

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {}

pub fn run(ctx: &Ctx, _: Args) -> Result<()> {
    let r = tick::tick(&ctx.home, &ctx.home.load_config()?)?;
    out(ctx.json, &r, || {
        for s in &r.steps {
            println!("{:<6} {}", s.name, s.summary);
            for e in s.errors.iter().take(10) {
                eprintln!("  {e}");
            }
        }
    })
}
