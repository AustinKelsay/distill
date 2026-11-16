//! `distill`: the command line. Each command is its own module under `commands/` and holds no
//! logic of its own; the work is in distill-core. Only `read` prints session text, and only
//! through the read path.

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
    /// Scan every session whose findings are missing or out of date for secrets
    Scan(scan::Args),
    /// Findings by fingerprint: the rotation list, or where one fingerprint appears
    Findings(findings::Args),
    /// What the timer runs: every step of a tick, in order
    Tick(tick::Args),
    /// Sessions, capture health and format drift per harness, the last sweep and scan progress
    Status(status::Args),
    /// The run log
    Runs(runs::Args),
    /// Providers, which are local and which are trusted, and the local runtime's models
    Providers(providers::Args),
    /// Read a session's events as a model at a named provider
    Read(read::Args),
    /// The read log: who read which session, and what they got
    Reads(reads::Args),
    /// Ask for a secret's value as a model, and collect it once the owner approves in the GUI
    Reveal(reveal::Args),
    /// The timer that runs `distill tick`
    Schedule(schedule::Args),
    /// Whether Distill is no more exposed than the harness folders: file modes, the fingerprint
    /// key, synced folders and network listeners
    Exposure(exposure::Args),
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
        Cmd::Scan(a) => scan::run(&ctx, a),
        Cmd::Findings(a) => findings::run(&ctx, a),
        Cmd::Tick(a) => tick::run(&ctx, a),
        Cmd::Status(a) => status::run(&ctx, a),
        Cmd::Runs(a) => runs::run(&ctx, a),
        Cmd::Providers(a) => providers::run(&ctx, a),
        Cmd::Read(a) => read::run(&ctx, a),
        Cmd::Reads(a) => reads::run(&ctx, a),
        Cmd::Reveal(a) => reveal::run(&ctx, a),
        Cmd::Schedule(a) => schedule::run(&ctx, a),
        Cmd::Exposure(a) => exposure::run(&ctx, a),
        Cmd::Gui(a) => gui::run(&ctx, a),
    }
}
