//! `distill reveal`: a model at a trusted provider asks for the real value of specific secrets,
//! and collects them once the owner has approved. Approving and denying happen only in the GUI
//! (ADR 0004), so this command has no way to answer a request.

use anyhow::{Result, bail};
use clap::Subcommand;
use distill_core::reveal::{self, Collected, Filed};
use distill_core::runtime::LocalRuntime;
use distill_core::trust::Reader;
use serde_json::json;

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Who is asking: the reading model.
#[derive(clap::Args)]
struct Who {
    /// The provider the asking model runs at (default: $DISTILL_PROVIDER)
    #[arg(long)]
    provider: Option<String>,
    /// The asking model, when known (default: $DISTILL_MODEL)
    #[arg(long)]
    model: Option<String>,
}

impl Who {
    fn reader(&self) -> Reader {
        Reader::from_flags_or_env(self.provider.as_deref(), self.model.as_deref())
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Ask the owner for the values of secrets in a session; prints a request ID, never a value
    Request {
        /// The session: `harness/id`, or its id alone when only one harness has it
        session: String,
        /// Each secret, by the fingerprint its placeholder shows (`#a1b2c3d4`)
        #[arg(required = true)]
        fingerprints: Vec<String>,
        /// Why the values are needed; the owner reads it before answering
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        who: Who,
    },
    /// Collect the values of a request the owner approved: once, by the model that asked
    Collect {
        /// The request ID `distill reveal request` printed
        request: String,
        #[command(flatten)]
        who: Who,
    },
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let runtime = LocalRuntime::ollama();
    match a.cmd {
        Cmd::Request {
            session,
            fingerprints,
            reason,
            who,
        } => match reveal::request(
            &ctx.home,
            &runtime,
            &who.reader(),
            &session,
            &fingerprints,
            &reason,
        )? {
            Filed::Filed(r) => {
                let receipt = r.receipt();
                let secrets: Vec<String> = r.secrets.iter().map(|s| s.placeholder()).collect();
                let value = json!({
                    "request": r.id,
                    "session": r.session,
                    "secrets": secrets,
                    "expires_at": r.expires_at,
                    "text": receipt,
                });
                out(ctx.json, &value, || println!("{receipt}"))
            }
            Filed::Refused(why) => bail!("refused: {why}"),
        },
        Cmd::Collect { request, who } => {
            match reveal::collect(&ctx.home, &runtime, &who.reader(), &request)? {
                Collected::Released(r) => {
                    let text = r.text();
                    let values: Vec<_> = r
                        .values
                        .iter()
                        .map(|v| json!({"secret": v.secret.placeholder(), "value": v.value}))
                        .collect();
                    let missing: Vec<String> = r.missing.iter().map(|s| s.placeholder()).collect();
                    let value = json!({
                        "request": r.request,
                        "session": r.session,
                        "values": values,
                        "missing": missing,
                    });
                    out(ctx.json, &value, || print!("{text}"))
                }
                Collected::Refused(why) => bail!("refused: {why}"),
            }
        }
    }
}
