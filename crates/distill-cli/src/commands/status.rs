use anyhow::Result;
use distill_core::status;

use super::Ctx;
use crate::output::{human_bytes, out};

#[derive(clap::Args)]
pub struct Args {}

pub fn run(ctx: &Ctx, _: Args) -> Result<()> {
    let s = status::status(&ctx.home)?;
    out(ctx.json, &s, || {
        if !s.configured {
            println!("Distill is not set up here yet: run `distill init`.");
        }
        println!(
            "{} sessions · {} archived · last sweep: {}",
            s.sessions,
            human_bytes(s.archive_bytes),
            s.last_sweep.as_deref().unwrap_or("never")
        );
        for h in &s.harnesses {
            println!(
                "  {:<12} {:<8} {:>6} sessions {:>6} captures {:>10}  {}",
                h.harness.id(),
                format!("{:?}", h.health).to_lowercase(),
                h.sessions,
                h.captures,
                human_bytes(h.archive_bytes),
                h.note.as_deref().unwrap_or("")
            );
        }
        for w in &s.warnings {
            println!("warning: {}", w.message);
        }
    })
}
