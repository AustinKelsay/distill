//! `distill backup`: make the backup key, back up to the `[[backup]]` targets, show where each
//! stands, and check that they hold their newest run (ADR 0009).

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use distill_core::backup::{self, BackupKey};
use distill_core::config::BackupTarget;
use distill_core::status::WarningKind;

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Make this home's backup key and print it, once
    Init,
    /// Replace the backup key with a new one and print it, once; every target starts over
    NewKey,
    /// Back up to every `[[backup]]` target in config.toml, or only the one named
    Run {
        #[arg(long)]
        target: Option<String>,
    },
    /// Each target's last whole run, its last failure and the captures it is behind
    Status,
    /// Check that every target, or the one named, holds every object of its newest run
    Verify {
        #[arg(long)]
        target: Option<String>,
        /// Fetch and decrypt every object, not only look for it by name
        #[arg(long)]
        full: bool,
    },
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    match a.cmd {
        Cmd::Init => init(ctx),
        Cmd::NewKey => new_key(ctx),
        Cmd::Run { target } => run_targets(ctx, target.as_deref()),
        Cmd::Verify { target, full } => verify(ctx, target.as_deref(), full),
        Cmd::Status => status(ctx),
    }
}

fn init(ctx: &Ctx) -> Result<()> {
    let key = BackupKey::create(&ctx.home)?;
    let recovery = key.recovery();
    let recovery = distill_core::backup::expose(&recovery);
    out(
        ctx.json,
        &serde_json::json!({ "backup_key": recovery }),
        || {
            println!("Made this home's backup key:\n\n  {recovery}\n");
            println!(
                "Keep it somewhere other than this machine (a password manager, or on paper). \
             Every backup is encrypted with it: without it, a lost disk means lost backups. \
             It is not shown again; it stays in keys/backup.key."
            );
        },
    )
}

fn new_key(ctx: &Ctx) -> Result<()> {
    let key = BackupKey::replace(&ctx.home)?;
    let recovery = key.recovery();
    let recovery = distill_core::backup::expose(&recovery);
    out(
        ctx.json,
        &serde_json::json!({ "backup_key": recovery }),
        || {
            println!("Made a new backup key for this home:\n\n  {recovery}\n");
            println!(
                "Keep it somewhere other than this machine, beside the old one. Every target \
             starts over: the next backup (`distill backup run`, or the next tick) sends \
             everything again under the new key. Only the old key opens the backups made \
             before now, so keep its copy for as long as you may want them."
            );
        },
    )
}

/// The `[[backup]]` targets in config.toml, or only the one named.
fn targets(ctx: &Ctx, only: Option<&str>) -> Result<Vec<BackupTarget>> {
    backup::targets(&ctx.home.load_config()?, only)
}

/// Each target's last whole run, its last failure and the captures it is behind.
fn status(ctx: &Ctx) -> Result<()> {
    let config = ctx.home.load_config()?;
    if !config.backups.is_empty() {
        backup::targets(&config, None)?;
    }
    let s = distill_core::status::status(&ctx.home)?;
    out(ctx.json, &s.backups, || {
        if s.backups.is_empty() {
            println!("No [[backup]] targets in config.toml.");
        }
        for t in &s.backups {
            let last = t.last_ok.as_deref().unwrap_or("never");
            let behind = t
                .captures_behind
                .map_or(String::new(), |n| format!(", {n} captures behind"));
            println!("{} ({}): last whole run {last}{behind}", t.name, t.kind);
            if let (Some(at), Some(e)) = (&t.last_failed, &t.last_error) {
                println!("  failed {at}: {e}");
            }
        }
        for w in s.warnings.iter().filter(|w| w.kind == WarningKind::Backup) {
            println!("{}", w.message);
        }
    })
}

fn verify(ctx: &Ctx, only: Option<&str>, full: bool) -> Result<()> {
    let key = BackupKey::existing(&ctx.home)?
        .context("this home has no backup key yet: run `distill backup init` first")?;
    let mut checks = Vec::new();
    for t in targets(ctx, only)? {
        // Never a first use: a check makes nothing at the target.
        let check = backup::store::open(&t, false)
            .and_then(|s| backup::verify(&ctx.home, &t.name, &*s, &key, full));
        checks.push(check.map_err(|e| format!("{}: {e:#}", t.name)));
    }
    out(ctx.json, &checks, || {
        for c in &checks {
            match c {
                Ok(c) => println!("{}", c.summary()),
                Err(e) => println!("{e}"),
            }
        }
    })?;
    if !checks.iter().all(|c| c.as_ref().is_ok_and(|c| c.ok())) {
        bail!("a backup target is not whole");
    }
    Ok(())
}

fn run_targets(ctx: &Ctx, only: Option<&str>) -> Result<()> {
    let targets = targets(ctx, only)?;
    let mut reports = Vec::new();
    let mut failed = false;
    for t in &targets {
        let r = backup::run(&ctx.home, t).unwrap_or_else(|e| backup::BackupReport {
            target: t.name.clone(),
            error: Some(format!("{e:#}")),
            ..Default::default()
        });
        failed |= !r.ok;
        reports.push(r);
    }
    out(ctx.json, &reports, || {
        for r in &reports {
            println!("{}", r.summary());
        }
    })?;
    if failed {
        bail!("a backup failed");
    }
    Ok(())
}
