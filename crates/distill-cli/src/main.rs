//! `distill`: the command line. Each command is its own module under `commands/` and holds no
//! logic of its own; the work is in distill-core. Only `list`, `read`, `search`, `tags` and
//! `reveal collect` print session text or secret values, and only through the read path; and
//! `export`, on a spoke, writes its captures for the hub's pull (ADR 0010).

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
    /// DISTILL home (default: $DISTILL_HOME or ~/distill)
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
    /// On a hub: take each spoke's new captures over SSH
    Pull(pull::Args),
    /// On a spoke: write its captures after a point, for the hub's pull (run by the hub over SSH)
    Export(export::Args),
    /// Turn captures into events and session facts
    Parse(parse::Args),
    /// Scan every session whose findings are missing or out of date for secrets and personal details
    Scan(scan::Args),
    /// Findings by fingerprint: the rotation list, or where one fingerprint appears
    Findings(findings::Args),
    /// What the timer runs: every step of a tick, in order
    Tick(tick::Args),
    /// Sessions, capture health and format drift per harness, the last sweep, scan and index progress
    Status(status::Args),
    /// The run log
    Runs(runs::Args),
    /// Providers, which are local and which are trusted, and the local runtime's models
    Providers(providers::Args),
    /// Encrypted backups to the `[[backup]]` targets: make the key, back up now, or check them
    Backup(backup::Args),
    /// Bring a backup back into an empty home, with its backup key
    Restore(restore::Args),
    /// List sessions matching filters, newest first, as a model at a named provider
    List(list::Args),
    /// Read one or more sessions' events as a model at a named provider
    Read(read::Args),
    /// The read log: who read which session, and what they got
    Reads(reads::Args),
    /// Search sessions by words and by meaning as a model at a named provider
    Search(search::Args),
    /// The tags on the sessions' cards, with how many sessions carry each, as a model at a named
    /// provider
    Tags(tags::Args),
    /// Add tags to a session or hide tags on it, as its owner
    Tag(tag::Args),
    /// Ask for a secret's value as a model, and collect it once the owner approves in the GUI
    Reveal(reveal::Args),
    /// The timer that runs `distill tick`
    Schedule(schedule::Args),
    /// Whether DISTILL is no more exposed than the harness folders: file modes, the fingerprint
    /// key, synced folders and network listeners
    Exposure(exposure::Args),
    /// Open the DISTILL window
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
        Cmd::Pull(a) => pull::run(&ctx, a),
        Cmd::Export(a) => export::run(&ctx, a),
        Cmd::Parse(a) => parse::run(&ctx, a),
        Cmd::Scan(a) => scan::run(&ctx, a),
        Cmd::Findings(a) => findings::run(&ctx, a),
        Cmd::Tick(a) => tick::run(&ctx, a),
        Cmd::Status(a) => status::run(&ctx, a),
        Cmd::Runs(a) => runs::run(&ctx, a),
        Cmd::Providers(a) => providers::run(&ctx, a),
        Cmd::Backup(a) => backup::run(&ctx, a),
        Cmd::Restore(a) => restore::run(&ctx, a),
        Cmd::List(a) => list::run(&ctx, a),
        Cmd::Read(a) => read::run(&ctx, a),
        Cmd::Reads(a) => reads::run(&ctx, a),
        Cmd::Search(a) => search::run(&ctx, a),
        Cmd::Tags(a) => tags::run(&ctx, a),
        Cmd::Tag(a) => tag::run(&ctx, a),
        Cmd::Reveal(a) => reveal::run(&ctx, a),
        Cmd::Schedule(a) => schedule::run(&ctx, a),
        Cmd::Exposure(a) => exposure::run(&ctx, a),
        Cmd::Gui(a) => gui::run(&ctx, a),
    }
}
