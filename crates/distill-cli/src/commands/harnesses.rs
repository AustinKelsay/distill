use anyhow::Result;
use distill_core::{Harness, harness};

use super::Ctx;
use crate::output::{human_bytes, out};

#[derive(clap::Args)]
pub struct Args {}

pub fn run(ctx: &Ctx, _: Args) -> Result<()> {
    let c = ctx.home.load_config().unwrap_or_default();
    let found: Vec<_> = Harness::ALL
        .iter()
        .map(|&k| harness::detect(k, &c.roots_for(k)))
        .collect();
    out(ctx.json, &found, || {
        for d in &found {
            println!(
                "{:<12} {:<9} {:>10} {:>7} files{}",
                d.kind.id(),
                if !d.found {
                    "missing"
                } else if c.enabled(d.kind) {
                    "enabled"
                } else {
                    "disabled"
                },
                human_bytes(d.bytes),
                d.files,
                if d.has_parser {
                    ""
                } else {
                    "  (archived only, no parser)"
                }
            );
        }
    })
}
