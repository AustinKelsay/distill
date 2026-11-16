//! `distill`: the command line. Each command is its own module under `commands/` and holds no
//! logic of its own; the work is in distill-core. No command prints session text.

mod commands;
mod output;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use distill_core::Home;

use crate::commands::*;

#[derive(Parser)]
#[command(
    name = "distill",
    version,
    about = "Keep every agent session in one private archive"
)]
struct Cli {
    /// Distill home (default: $DISTILL_HOME or ~/distill)
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    /// Print machine-readable JSON
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create the home and a config with every harness found on this machine enabled
    Init(init::Args),
    /// Show each harness and what of it is on this machine
    Harnesses(harnesses::Args),
    /// Copy new and changed harness files into the raw archive
    Sweep(sweep::Args),
    /// Check every capture in the raw archive against its hash
    Verify(verify::Args),
    /// Turn captures into events and session facts
    Parse(parse::Args),
    /// What the timer runs: every step of a tick, in order
    Tick(tick::Args),
    /// Sessions, capture health and format drift per harness, and the last sweep
    Status(status::Args),
    /// The run log
    Runs(runs::Args),
    /// The timer that runs `distill tick`
    Schedule(schedule::Args),
    /// Open the Distill window
    Gui(gui::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let ctx = Ctx {
        home: Home::resolve(cli.home.as_deref())?,
        json: cli.json,
    };
    match cli.cmd {
        Cmd::Init(a) => init::run(&ctx, a),
        Cmd::Harnesses(a) => harnesses::run(&ctx, a),
        Cmd::Sweep(a) => sweep::run(&ctx, a),
        Cmd::Verify(a) => verify::run(&ctx, a),
        Cmd::Parse(a) => parse::run(&ctx, a),
        Cmd::Tick(a) => tick::run(&ctx, a),
        Cmd::Status(a) => status::run(&ctx, a),
        Cmd::Runs(a) => runs::run(&ctx, a),
        Cmd::Schedule(a) => schedule::run(&ctx, a),
        Cmd::Gui(a) => gui::run(&ctx, a),
    }
}
