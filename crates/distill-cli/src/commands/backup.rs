//! `distill backup`: make the backup key, back up to the `[[backup]]` targets, and check that
//! they hold their newest run (ADR 0009).

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use distill_core::backup::{self, BackupKey};
use distill_core::config::BackupTarget;

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Make this home's backup key and print its recovery key, once
    Init,
    /// Back up to every `[[backup]]` target in config.toml, or only the one named
    Run {
        #[arg(long)]
        target: Option<String>,
    },
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
        Cmd::Run { target } => run_targets(ctx, target.as_deref()),
        Cmd::Verify { target, full } => verify(ctx, target.as_deref(), full),
    }
}

fn init(ctx: &Ctx) -> Result<()> {
    let key = BackupKey::create(&ctx.home)?;
    let recovery = key.recovery();
    let recovery = distill_core::backup::expose(&recovery);
    out(
        ctx.json,
        &serde_json::json!({ "recovery_key": recovery }),
        || {
            println!("Made this home's backup key. Its recovery key:\n\n  {recovery}\n");
            println!(
                "Keep it somewhere other than this machine (a password manager, or on paper). \
             Every backup is encrypted with it: without it, a lost disk means lost backups. \
             It is not shown again; it stays in keys/backup.key."
            );
        },
    )
}

/// The `[[backup]]` targets in config.toml, or only the one named.
fn targets(ctx: &Ctx, only: Option<&str>) -> Result<Vec<BackupTarget>> {
    let config = ctx.home.load_config()?;
    if let Some(t) = config
        .backups
        .iter()
        .enumerate()
        .find(|(i, t)| config.backups[..*i].iter().any(|u| u.name == t.name))
        .map(|(_, t)| t)
    {
        bail!(
            "config.toml names two [[backup]] targets {:?}; give each its own name",
            t.name
        );
    }
    let targets: Vec<_> = config
        .backups
        .into_iter()
        .filter(|t| only.is_none_or(|n| t.name == n))
        .collect();
    if targets.is_empty() {
        match only {
            Some(n) => bail!("no [[backup]] target named {n:?} in config.toml"),
            None => bail!("no [[backup]] targets in config.toml"),
        }
    }
    Ok(targets)
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
