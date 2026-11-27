//! `distill read`: a session's events as text or structured JSONL, for a model at a named
//! provider. Through the read path, like every command that prints session text.

use std::io::{BufRead, BufWriter, Write};
use std::ops::ControlFlow;

use anyhow::{Result, bail};
use distill_core::read::{self, Range, Read, StreamOptions, Streamed};
use distill_core::runtime::LocalRuntime;
use distill_core::trust::Reader;

use super::Ctx;
use crate::output::out;

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Text,
    Jsonl,
}

#[derive(clap::Args)]
pub struct Args {
    /// One or more sessions: `harness/id`, or an id alone when only one harness has it
    sessions: Vec<String>,
    /// The provider the reading model runs at (default: $DISTILL_PROVIDER)
    #[arg(long)]
    provider: Option<String>,
    /// The reading model, when known (default: $DISTILL_MODEL)
    #[arg(long)]
    model: Option<String>,
    /// Output format: `text` (default, notice-wrapped) or `jsonl` (`schema_version: 1` records)
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Read one session key per line from standard input (requires `--format jsonl`)
    #[arg(long)]
    stdin: bool,
    /// The first event to read, by number (single-session reads only)
    #[arg(long)]
    from: Option<u64>,
    /// The last event to read, by number (single-session reads only)
    #[arg(long)]
    to: Option<u64>,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let reader = Reader::from_flags_or_env(a.provider.as_deref(), a.model.as_deref());
    let range = Range {
        from: a.from,
        to: a.to,
    };
    let has_range = range.from.is_some() || range.to.is_some();

    match a.format {
        Format::Text => {
            if a.stdin {
                bail!("--stdin requires --format jsonl");
            }
            if a.sessions.len() != 1 {
                bail!("provide one session key (or use --format jsonl for multiple sessions)");
            }
            match read::read(
                &ctx.home,
                &LocalRuntime::ollama(),
                &reader,
                &a.sessions[0],
                range,
            )? {
                Read::Text(text) => out(ctx.json, &serde_json::json!({ "text": text }), || {
                    print!("{text}")
                }),
                Read::Refused(why) => bail!("refused: {why}"),
                // Only the owner and DISTILL itself get events, and neither reads from here.
                Read::Events(_) => bail!("this reader is not served on the command line"),
            }
        }
        Format::Jsonl => {
            let mut keys = a.sessions;
            if a.stdin {
                let stdin = std::io::stdin();
                for line in stdin.lock().lines() {
                    let line = line?;
                    let key = line.trim();
                    if !key.is_empty() {
                        keys.push(key.to_string());
                    }
                }
            } else if keys.is_empty() {
                bail!("provide at least one session key or pass --stdin");
            }
            if has_range && keys.len() != 1 {
                bail!("--from and --to apply to a single session only");
            }
            let stdout = std::io::stdout();
            let mut writer = BufWriter::new(stdout.lock());
            let streamed = read::stream(
                &ctx.home,
                &LocalRuntime::ollama(),
                &reader,
                &keys,
                &StreamOptions { range },
                |rec| {
                    serde_json::to_writer(&mut writer, rec)?;
                    writer.write_all(b"\n")?;
                    writer.flush()?;
                    Ok(ControlFlow::Continue(()))
                },
            )?;
            match streamed {
                Streamed::Done(report) => {
                    for (key, why) in report.skipped {
                        eprintln!("warning: skipped {key}: {why}");
                    }
                    Ok(())
                }
                Streamed::Refused(why) => bail!("refused: {why}"),
            }
        }
    }
}
