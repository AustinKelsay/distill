//! `distill backup`: make the backup key, and back up to the `[[backup]]` targets (ADR 0009).

use anyhow::{Result, bail};
use clap::Subcommand;
use distill_core::backup::{self, BackupKey};

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
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    match a.cmd {
        Cmd::Init => init(ctx),
        Cmd::Run { target } => run_targets(ctx, target.as_deref()),
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

fn run_targets(ctx: &Ctx, only: Option<&str>) -> Result<()> {
    let config = ctx.home.load_config()?;
    let targets: Vec<_> = config
        .backups
        .iter()
        .filter(|t| only.is_none_or(|n| t.name == n))
        .collect();
    if targets.is_empty() {
        match only {
            Some(n) => bail!("no [[backup]] target named {n:?} in config.toml"),
            None => bail!("no [[backup]] targets in config.toml"),
        }
    }
    let mut reports = Vec::new();
    let mut failed = false;
    for t in targets {
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
