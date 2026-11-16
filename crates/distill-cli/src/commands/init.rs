use anyhow::Result;
use distill_core::setup;

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// Overwrite an existing config
    #[arg(long)]
    force: bool,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let c = setup::detected_config();
    setup::initialize(&ctx.home, &c, a.force)?;
    out(ctx.json, &c, || {
        println!("Wrote {}", ctx.home.config_path().display());
        for h in &c.harnesses {
            println!(
                "  {:<12} {}",
                h.kind.id(),
                if h.enabled { "enabled" } else { "not found" }
            );
        }
        println!("Next: `distill tick`, then `distill schedule install`.");
    })
}
