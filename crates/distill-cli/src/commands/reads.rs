//! `distill reads`: the read log, newest first. It holds no session text.

use anyhow::Result;
use distill_core::readlog::{self, ReadResult};

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// Only this reader's reads: `owner`, `distill`, a provider, or `provider/model`
    #[arg(long)]
    reader: Option<String>,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let log = readlog::list(&ctx.home, a.reader.as_deref(), a.limit)?;
    out(ctx.json, &log, || {
        for e in &log {
            let result = match &e.result {
                ReadResult::Served { events } => {
                    format!("served {}", distill_core::util::count(*events, "event"))
                }
                ReadResult::Listed { sessions } => {
                    format!("listed {}", distill_core::util::count(*sessions, "session"))
                }
                ReadResult::Indexed { sessions, events } => format!(
                    "indexed {}, {}",
                    distill_core::util::count(*sessions, "session"),
                    distill_core::util::count(*events, "event")
                ),
                ReadResult::RevealRequested { request, secrets } => {
                    let fps: Vec<String> = secrets.iter().map(|s| format!("#{s}")).collect();
                    format!("reveal {request} requested: {}", fps.join(" "))
                }
                ReadResult::RevealApproved { request } => {
                    format!("reveal {request} approved by the owner")
                }
                ReadResult::RevealDenied { request } => {
                    format!("reveal {request} denied by the owner")
                }
                ReadResult::RevealCollected { request, secrets } => format!(
                    "reveal {request} collected: {}",
                    distill_core::util::count(*secrets, "value")
                ),
                ReadResult::RevealExpired { request } => format!("reveal {request} expired"),
                ReadResult::Findings {
                    fingerprints,
                    sessions,
                } => {
                    let mut s = format!(
                        "listed {}",
                        distill_core::util::count(*fingerprints, "fingerprint")
                    );
                    if *sessions > 0 {
                        s += &format!(" in {}", distill_core::util::count(*sessions, "session"));
                    }
                    s
                }
                ReadResult::Refused { reason } => format!("refused: {reason}"),
                ReadResult::Failed { error } => format!("failed: {error}"),
            };
            println!(
                "{}  {:<28} {:<36} {result}",
                e.at,
                e.reader.to_string(),
                e.session.as_deref().unwrap_or(match e.result {
                    ReadResult::Listed { .. } => "(a list of sessions)",
                    ReadResult::Findings { .. } => "(findings)",
                    _ => "-",
                })
            );
        }
    })
}
