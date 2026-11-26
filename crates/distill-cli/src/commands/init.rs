use anyhow::Result;
use distill_core::config::Spoke;
use distill_core::{home, setup};

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// Overwrite an existing config
    #[arg(long)]
    force: bool,
    /// Set this machine up as a spoke named MACHINE: it only captures, for a hub to pull from
    #[arg(long, value_name = "MACHINE")]
    spoke: Option<String>,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let mut c = setup::detected_config();
    if let Some(machine) = a.spoke {
        home::machine_name(&machine)?;
        c.spoke = Some(Spoke { machine });
    }
    setup::initialize(&ctx.home, &c, a.force)?;
    out(ctx.json, &c, || {
        println!("Wrote {}", ctx.home.config_path().display());
        if let Some(s) = &c.spoke {
            println!(
                "  a spoke named \"{}\": its tick only captures, and nothing reads here",
                s.machine
            );
        }
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
