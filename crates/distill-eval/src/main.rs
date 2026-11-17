//! `distill-eval`: run the steward's model steps (cards, the model detection pass, vectors) and
//! search by meaning with a trusted remote provider's models, through its OpenAI-compatible server
//! on this machine, against a copy of a home. It prints counts, kinds of failure and ranks, never
//! session text, card text or a model's answer, so its output can be read by anyone; judging what
//! the models wrote is done inside the fence, from the copy itself.
//!
//! Never point it at a live home: it writes cards, findings, vectors and the config there.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use distill_core::home::Home;
use distill_core::read::{self, Filter, Page, Searched};
use distill_core::runtime::LocalRuntime;
use distill_core::tick::Deadline;
use distill_core::trust::{self, Reader};

#[derive(Parser)]
struct Cli {
    /// The copy of a home to work in.
    #[arg(long)]
    home: PathBuf,
    /// The remote provider the server belongs to, as the home's config lists it.
    #[arg(long)]
    provider: String,
    /// The provider's OpenAI-compatible server, on this machine.
    #[arg(long, default_value = "127.0.0.1:8080")]
    addr: SocketAddr,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// The provider's models.
    Models,
    /// Trust the provider in the home's config and set the models the steps use.
    Setup {
        #[arg(long)]
        card_model: String,
        #[arg(long)]
        embedding_model: String,
        /// Defaults to the card model.
        #[arg(long)]
        detail_model: Option<String>,
        /// How many sessions each step takes, newest first.
        #[arg(long, default_value_t = 25)]
        sessions: usize,
    },
    /// Bring the word index up to date.
    Index,
    /// Run one model step: `cards`, `details` or `vectors`.
    Run { step: String },
    /// Search for each line of a JSONL file of `{"session": <key>, "query": <words>}`, as the
    /// owner, and print where the session ranks.
    Rank { queries: PathBuf },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let home = Home::new(cli.home.clone());
    if distill_core::home::user_home().is_ok_and(|u| home.root == u.join("distill")) {
        bail!("{} is the live home; work in a copy", home.root.display());
    }
    let runtime = LocalRuntime::openai_compatible(&cli.provider, cli.addr)?;
    match cli.command {
        Command::Models => match runtime.state() {
            distill_core::runtime::RuntimeState::Running(models) => {
                for m in models {
                    println!("{}", m.name);
                }
            }
            other => bail!("the provider's server: {other:?}"),
        },
        Command::Setup {
            card_model,
            embedding_model,
            detail_model,
            sessions,
        } => {
            let mut config = home.load_config()?;
            if let Err(e) = trust::add(&mut config, &cli.provider, true) {
                trust::set_trusted(&mut config, &cli.provider, true).context(e)?;
            }
            config.steward.card_model = Some(card_model);
            config.steward.embedding_model = Some(embedding_model);
            config.steward.detail_model = detail_model;
            config.steward.sessions_per_run = sessions;
            config.save(&home.config_path())?;
            println!(
                "{} trusted; models set; {sessions} sessions a step",
                cli.provider
            );
        }
        Command::Index => {
            let r = distill_core::index::build(&home)?;
            println!("{}", r.summary());
        }
        Command::Run { step } => {
            let config = home.load_config()?;
            let (summary, report, errors) = match step.as_str() {
                "cards" => {
                    let r = distill_core::cards::write(&home, &config, &runtime, Deadline::NONE)?;
                    (r.summary(), serde_json::to_value(&r)?, r.errors)
                }
                "details" => {
                    let r = distill_core::details::find(&home, &config, &runtime, Deadline::NONE)?;
                    (r.summary(), serde_json::to_value(&r)?, r.errors)
                }
                "vectors" => {
                    let r = distill_core::index::vectors::build(
                        &home,
                        &config,
                        &runtime,
                        Deadline::NONE,
                    )?;
                    (r.summary(), serde_json::to_value(&r)?, r.errors)
                }
                other => bail!("no step \"{other}\": cards, details or vectors"),
            };
            println!("{summary}");
            println!("{}", counts(&report));
            for (kind, n) in failure_kinds(&errors) {
                println!("  {n} × {kind}");
            }
        }
        Command::Rank { queries } => rank(&home, &runtime, &queries)?,
    }
    Ok(())
}

/// The report's numbers only.
fn counts(report: &serde_json::Value) -> String {
    report
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, v)| v.is_number() || v.is_boolean())
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Each failure's kind: what went wrong, cut before anything that could quote what a model
/// answered (a parser's own words in parentheses or quotes) and without the session.
fn failure_kinds(errors: &[String]) -> BTreeMap<String, usize> {
    let mut kinds = BTreeMap::new();
    for e in errors {
        let what = e.split_once(": ").map_or(e.as_str(), |(_, w)| w);
        let cut = what.find(['(', '"', '`', '\'']).unwrap_or(what.len());
        let kind: String = what[..cut].trim().chars().take(100).collect();
        *kinds.entry(kind).or_insert(0) += 1;
    }
    kinds
}

fn rank(home: &Home, runtime: &LocalRuntime, queries: &std::path::Path) -> Result<()> {
    let file = std::fs::File::open(queries)?;
    let (mut ranks, mut meaning_off) = (Vec::new(), 0);
    for line in std::io::BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let q: serde_json::Value = serde_json::from_str(&line)?;
        let (Some(session), Some(query)) = (q["session"].as_str(), q["query"].as_str()) else {
            bail!("each line needs a session and a query");
        };
        let page = Page {
            offset: 0,
            limit: 50,
        };
        let found = match read::search(
            home,
            runtime,
            &Reader::Owner,
            query,
            &Filter::default(),
            page,
        )? {
            Searched::Hits(h) => {
                meaning_off += usize::from(!matches!(h.meaning, read::Meaning::Used { .. }));
                h.hits.iter().position(|hit| hit.facts.key() == session)
            }
            _ => None,
        };
        let rank = found.map(|i| i + 1);
        println!(
            "{session}\t{}",
            rank.map_or("-".to_string(), |r| r.to_string())
        );
        ranks.push(rank);
    }
    let within = |n: usize| ranks.iter().filter(|r| r.is_some_and(|r| r <= n)).count();
    println!(
        "{} queries: top 1 {}, top 5 {}, top 10 {}, top 50 {}, not in the top 50 {}; by words only {}",
        ranks.len(),
        within(1),
        within(5),
        within(10),
        within(50),
        ranks.iter().filter(|r| r.is_none()).count(),
        meaning_off
    );
    Ok(())
}
