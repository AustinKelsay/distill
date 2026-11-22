//! `distill restore`: bring a backup back into an empty home (ADR 0009, issue #149).

use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use distill_core::backup::{self, BackupKey, store::Folder};

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// The folder target to restore from: the `path` of its `[[backup]]` entry
    #[arg(long)]
    folder: PathBuf,
    /// A file holding the recovery key (default: read it from standard input, piped)
    #[arg(long)]
    key_file: Option<PathBuf>,
    /// Restore even when the target lacks blobs the manifest names, and count them
    #[arg(long)]
    allow_missing: bool,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let text = match &a.key_file {
        Some(p) => std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?,
        // Typed at a terminal, the key would show on screen and stay in its scrollback.
        None if std::io::stdin().is_terminal() => bail!(
            "give the recovery key with --key-file, or pipe it in (for example \
             `pbpaste | distill restore --folder ...`)"
        ),
        None => {
            let mut all = String::new();
            std::io::stdin().read_to_string(&mut all)?;
            all
        }
    };
    let key = BackupKey::from_recovery(&text)?;
    // A restore reads a target; it never makes its folder.
    let store = Folder::open(&a.folder, false)?;
    let report = backup::restore(&ctx.home, &store, &key, a.allow_missing)?;
    out(ctx.json, &report, || {
        println!("{}", report.summary());
        println!("Restored into {}.", ctx.home.root.display());
    })
}
