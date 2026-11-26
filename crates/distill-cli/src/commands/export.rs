//! `distill export`: on a spoke, its captures after a byte of its manifest, for the hub's pull
//! (ADR 0010). The hub runs it over SSH; it writes the export to stdout and logs it.

use anyhow::{Result, bail};
use distill_core::machines;

use super::Ctx;

#[derive(clap::Args)]
pub struct Args {
    /// The byte of this spoke's manifest the hub's copy ends at
    #[arg(long, conflicts_with = "ssh")]
    since: Option<u64>,
    /// Take `--since` from what the hub asked SSH to run (SSH_ORIGINAL_COMMAND), for a forced
    /// command in `authorized_keys`: `command="distill export --ssh"`
    #[arg(long)]
    ssh: bool,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let since = match (a.since, a.ssh) {
        (Some(n), _) => n,
        (None, true) => {
            machines::forced_since(std::env::var("SSH_ORIGINAL_COMMAND").ok().as_deref())?
        }
        (None, false) => bail!("name where the hub's copy ends with --since <bytes>"),
    };
    let stdout = std::io::stdout();
    machines::export(&ctx.home, since, &mut stdout.lock())?;
    Ok(())
}
