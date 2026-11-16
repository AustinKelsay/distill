//! One module per command. Each has an `Args` and a `run`.

pub mod exposure;
pub mod findings;
pub mod gui;
pub mod harnesses;
pub mod init;
pub mod parse;
pub mod providers;
pub mod read;
pub mod reads;
pub mod runs;
pub mod scan;
pub mod schedule;
pub mod status;
pub mod sweep;
pub mod tick;
pub mod verify;

use anyhow::{Context, Result};
use distill_core::{Harness, Home};

/// What every command gets.
pub struct Ctx {
    pub home: Home,
    pub json: bool,
}

/// A harness named on the command line.
pub fn parse_harness(s: &str) -> Result<Harness> {
    Harness::parse(s).with_context(|| {
        format!(
            "unknown harness {s:?}; one of {}",
            Harness::ALL.map(|k| k.id()).join(", ")
        )
    })
}
