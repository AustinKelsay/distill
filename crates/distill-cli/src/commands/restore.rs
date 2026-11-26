//! `distill restore`: bring a backup back into an empty home (ADR 0009, issue #149).

use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use distill_core::backup::{self, BackupKey, store};
use distill_core::config::BackupTarget;

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// The folder target to restore from: the `path` of its `[[backup]]` entry
    #[arg(long, required_unless_present = "kind", conflicts_with = "kind")]
    folder: Option<PathBuf>,
    /// Or the kind of an adapter target to restore from: runs `distill-backup-<kind>`
    #[arg(long)]
    kind: Option<String>,
    /// An adapter target's setting, as its `[[backup]]` table has it (repeat for each)
    #[arg(long = "set", value_name = "KEY=VALUE", requires = "kind")]
    settings: Vec<String>,
    /// A file holding the backup key (default: read it from standard input, piped)
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
            "give the backup key with --key-file, or pipe it in (for example \
             `pbpaste | distill restore --folder ...`)"
        ),
        None => {
            let mut all = String::new();
            std::io::stdin().read_to_string(&mut all)?;
            all
        }
    };
    let key = BackupKey::from_recovery(&text)?;
    // A restore reads a target; it never makes its folder or sets up its place.
    let target = BackupTarget {
        name: "restore".into(),
        kind: a.kind.clone().unwrap_or_else(|| "folder".into()),
        path: a.folder.clone(),
        cards: false,
        vectors: false,
        settings: a
            .settings
            .iter()
            .map(String::as_str)
            .map(setting)
            .collect::<Result<_>>()?,
    };
    let store = store::open(&target, false)?;
    let report = backup::restore(&ctx.home, &*store, &key, a.allow_missing)?;
    out(ctx.json, &report, || {
        println!("{}", report.summary());
        println!("Restored into {}.", ctx.home.root.display());
    })
}

/// `key=value` as a `[[backup]]` table would have it: the value read as TOML (`22`, `true`,
/// `"text"`), or else as text.
fn setting(s: &str) -> Result<(String, toml::Value)> {
    let Some((key, value)) = s.split_once('=') else {
        bail!("give a setting as key=value, not {s:?}");
    };
    let value = toml::from_str::<toml::Table>(&format!("v = {value}"))
        .ok()
        .and_then(|mut t| t.remove("v"))
        .unwrap_or_else(|| toml::Value::String(value.to_string()));
    Ok((key.trim().to_string(), value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_is_read_as_its_table_would_have_it() {
        let v = |s| setting(s).unwrap();
        assert_eq!(v("port=22"), ("port".into(), toml::Value::Integer(22)));
        assert_eq!(v("tls=true"), ("tls".into(), toml::Value::Boolean(true)));
        assert_eq!(
            v("dir=/mnt/a b"),
            ("dir".into(), toml::Value::String("/mnt/a b".into()))
        );
        assert_eq!(
            v("bucket=\"22\""),
            ("bucket".into(), toml::Value::String("22".into()))
        );
        assert!(setting("dir").is_err());
    }
}
