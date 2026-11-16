use anyhow::{Result, bail};
use distill_core::exposure::{self, Outcome};

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// First take the group's and others' permissions off the home and everything in it
    #[arg(long)]
    fix: bool,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let fixed = if a.fix {
        Some(exposure::fix(&ctx.home)?)
    } else {
        None
    };
    let e = exposure::check(&ctx.home);
    out(ctx.json, &e, || {
        if let Some(n) = fixed {
            match n {
                1 => println!("Tightened 1 file or folder."),
                n => println!("Tightened {n} files and folders."),
            }
        }
        for r in &e.rules {
            let word = match r.outcome {
                Outcome::Pass => "pass",
                Outcome::Fail => "fail",
                Outcome::Unchecked => "unchecked",
            };
            println!("{word:<9} {}: {}", r.rule.label(), r.message);
            for p in &r.paths {
                println!("            {p}");
            }
        }
        println!("{}", e.disk());
        println!("{}", e.backups);
        if !e.previous_build.is_empty() {
            println!("Left by the previous build, holding session-derived data:");
            for f in &e.previous_build {
                let state = if f.owner_only {
                    "owner-only"
                } else {
                    "open to others"
                };
                println!("  {} ({}, {state}): {}", f.path, f.mode, f.holds);
            }
        }
    })?;
    if !e.passes() {
        bail!("the exposure check fails");
    }
    Ok(())
}
