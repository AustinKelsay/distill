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
        println!(
            "scan: {} of {} sessions current, {} waiting · {} secrets found",
            s.scan.current, s.scan.sessions, s.scan.pending, s.scan.secrets
        );
        println!(
            "word index: {} of {} sessions current, {} waiting · {}",
            s.index.current,
            s.index.sessions,
            s.index.pending,
            human_bytes(s.index.bytes)
        );
        match &s.cards.skipped {
            Some(why) => println!(
                "cards: skipped ({why}) · {} of {} sessions have one",
                s.cards.current, s.cards.sessions
            ),
            None => println!(
                "cards: {} of {} sessions current, {} waiting, {} failing · model {}",
                s.cards.current,
                s.cards.sessions,
                s.cards.pending,
                s.cards.failing,
                s.cards.model.as_deref().unwrap_or("none")
            ),
        }
        match &s.vectors.skipped {
            Some(why) => println!(
                "vectors: skipped ({why}) · {} of {} sessions have them · search by meaning is off",
                s.vectors.current, s.vectors.sessions
            ),
            None => println!(
                "vectors: {} of {} sessions current, {} waiting · {} · model {}",
                s.vectors.current,
                s.vectors.sessions,
                s.vectors.pending,
                human_bytes(s.vectors.bytes),
                s.vectors.model.as_deref().unwrap_or("none")
            ),
        }
        println!(
            "reveals: {} waiting for the owner in the GUI",
            distill_core::util::count(s.pending_reveals, "request")
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
