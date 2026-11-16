use anyhow::Result;
use clap::Subcommand;
use distill_core::Config;
use distill_core::config::ProviderKind;
use distill_core::runtime::{LocalRuntime, RuntimeState, Stored};
use distill_core::trust::{self, ListedProvider, OLLAMA_CLOUD, Trust};

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// List each provider, its kind and whether it is trusted (the default)
    List,
    /// Add a remote provider, untrusted unless --trusted
    Add {
        name: String,
        #[arg(long)]
        trusted: bool,
    },
    /// Mark a remote provider trusted: its models may read session text
    Trust { name: String },
    /// Unmark a remote provider: its models are refused from now on
    Untrust { name: String },
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    match a.cmd {
        None | Some(Cmd::List) => list(ctx),
        Some(Cmd::Add { name, trusted }) => edit(ctx, &name, |c| trust::add(c, &name, trusted)),
        Some(Cmd::Trust { name }) => edit(ctx, &name, |c| trust::set_trusted(c, &name, true)),
        Some(Cmd::Untrust { name }) => edit(ctx, &name, |c| trust::set_trusted(c, &name, false)),
    }
}

/// Change the config's `[[providers]]`, save it and show where `name` now stands.
fn edit(ctx: &Ctx, name: &str, change: impl FnOnce(&mut Config) -> Result<()>) -> Result<()> {
    let mut c = ctx.home.load_config()?;
    change(&mut c)?;
    c.save(&ctx.home.config_path())?;
    let p = Trust::new(&c).provider(name).cloned();
    out(ctx.json, &p, || {
        if let Some(p) = &p {
            let state = if p.trusted { "trusted" } else { "not trusted" };
            println!("{} is {state}", p.name);
        }
    })
}

fn list(ctx: &Ctx) -> Result<()> {
    let c = ctx.home.load_config().unwrap_or_default();
    let l = Trust::new(&c).listing(&LocalRuntime::ollama());
    for w in &l.warnings {
        eprintln!("warning: {w}");
    }
    out(ctx.json, &l, || {
        for p in &l.providers {
            let ListedProvider { provider, runtime } = p;
            println!(
                "{:<16} {:<7} {}",
                provider.name,
                match provider.kind {
                    ProviderKind::Local => "local",
                    ProviderKind::Remote => "remote",
                },
                if provider.trusted {
                    "trusted"
                } else {
                    "not trusted"
                }
            );
            match runtime {
                None => {}
                Some(RuntimeState::NotInstalled) => {
                    println!("  Ollama is not installed on this machine")
                }
                Some(RuntimeState::NotRunning) => println!(
                    "  Ollama is installed but not running (start the Ollama app or `ollama serve`)"
                ),
                Some(RuntimeState::Running(models)) if models.is_empty() => {
                    println!("  running, with no models installed")
                }
                Some(RuntimeState::Running(models)) => {
                    for m in models {
                        println!(
                            "  {:<30} {}",
                            m.name,
                            match m.stored {
                                Stored::Here => "stored on this machine".to_string(),
                                Stored::Cloud =>
                                    format!("served from a cloud (reads as {OLLAMA_CLOUD})"),
                            }
                        );
                    }
                }
            }
        }
    })
}
