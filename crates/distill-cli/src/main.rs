//! `distill`: the one command the timer, the steward and you all use.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use distill_core::export::{self, Recipe};
use distill_core::search::{self, ListFilter};
use distill_core::steward::{self, RunOptions};
use distill_core::{
    Config, Home, SourceKind, archive, legacy, normalize, propose, runlog, schedule, source, stats,
    sweep, tick,
};

#[derive(Parser)]
#[command(
    name = "distill",
    version,
    about = "Passively archive agent sessions and let agents organize them"
)]
struct Cli {
    /// Distill home (default: $DISTILL_HOME or ~/distill)
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    /// Print machine-readable JSON
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create the home and a config with every detected source enabled
    Init {
        /// Overwrite an existing config
        #[arg(long)]
        force: bool,
    },
    /// Show detected sources
    Sources,
    /// Copy new and changed session data into the archive
    Sweep {
        #[arg(long)]
        source: Option<String>,
    },
    /// Check every archived blob against its hash
    Verify,
    /// Recover sessions kept only by the Distill this one replaced (~/.distill)
    ImportLegacy {
        /// The old app's database (default: ~/.distill/distill.db)
        #[arg(long)]
        from: Option<PathBuf>,
        /// Report what would be imported without writing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Rebuild meta.json and events.jsonl from the archive
    Normalize {
        /// Re-run parsers even for unchanged captures
        #[arg(long)]
        force: bool,
    },
    /// What the timer runs: sweep, normalize, steward if due, rebuild datasets
    Tick {
        /// Run the steward even if it is not due
        #[arg(long)]
        steward: bool,
    },
    /// The steward agent
    Steward {
        #[command(subcommand)]
        cmd: StewardCmd,
    },
    /// Dashboard numbers
    Status,
    /// List sessions
    Sessions {
        #[arg(long)]
        source: Option<String>,
        /// Match title or project path
        #[arg(long)]
        grep: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Search event text across all sessions
    Search {
        query: String,
        #[arg(long)]
        source: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Dataset recipes
    Recipe {
        #[command(subcommand)]
        cmd: RecipeCmd,
    },
    /// Dataset builds and approvals
    Export {
        #[command(subcommand)]
        cmd: ExportCmd,
    },
    /// Recent run log
    Runs {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// The timer that runs `distill tick`
    Schedule {
        #[command(subcommand)]
        cmd: ScheduleCmd,
    },
    /// Open the setup and dashboard window
    Gui,
}

#[derive(Subcommand)]
enum StewardCmd {
    /// Write cards for sessions that lack one
    Run {
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Print the digest the steward reads for one session
    Digest { source: String, session_id: String },
    /// Write a repair brief for a source whose captures fail to normalize
    RepairBrief { source: String },
    /// Start an agent on a parser repair, on its own branch of a Distill checkout
    Repair {
        source: String,
        #[arg(long)]
        repo: PathBuf,
    },
}

#[derive(Subcommand)]
enum RecipeCmd {
    /// Recipes, and the steward's pending proposals
    List,
    /// The steward's pending proposals
    Proposals,
    /// Ask the steward to propose new recipes
    Propose,
    /// Turn a proposal into a recipe
    Accept {
        name: String,
    },
    /// Drop a proposal
    Dismiss {
        name: String,
    },
    /// Write the default recipe under a name
    New {
        name: String,
    },
    Show {
        name: String,
    },
}

#[derive(Subcommand)]
enum ExportCmd {
    /// Build a recipe (or every recipe)
    Build {
        recipe: Option<String>,
    },
    List,
    /// Approve a build so it may leave the machine
    Approve {
        recipe: String,
        build: String,
    },
    /// Copy an approved build to a directory
    Publish {
        recipe: String,
        build: String,
        #[arg(long)]
        to: PathBuf,
    },
}

#[derive(Subcommand)]
enum ScheduleCmd {
    /// Print the timer definition without installing it
    Print,
    Install,
    Uninstall,
    Status,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn parse_source(s: &str) -> Result<SourceKind> {
    SourceKind::parse(s).with_context(|| {
        format!(
            "unknown source {s:?}; one of {}",
            SourceKind::ALL.map(|k| k.id()).join(", ")
        )
    })
}

fn out<T: serde::Serialize>(json: bool, value: &T, human: impl FnOnce()) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        human();
    }
    Ok(())
}

fn human_bytes(b: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", units[i])
    }
}

fn run(cli: Cli) -> Result<()> {
    let home = Home::resolve(cli.home.as_deref())?;
    let json = cli.json;
    let config = || home.load_config();
    match cli.cmd {
        Cmd::Init { force } => {
            home.ensure()?;
            if home.config_path().exists() && !force {
                bail!(
                    "{} already exists (use --force to overwrite)",
                    home.config_path().display()
                );
            }
            let mut c = Config::default();
            for kind in SourceKind::ALL {
                let found = c.roots_for(kind).iter().any(|r| r.exists());
                c.source_mut(kind).enabled = found;
            }
            distill_core::setup::initialize(&home, &c)?;
            out(json, &c, || {
                println!("Wrote {}", home.config_path().display());
                for s in &c.sources {
                    println!(
                        "  {:<12} {}",
                        s.kind.id(),
                        if s.enabled { "enabled" } else { "not found" }
                    );
                }
                println!("Next: `distill tick`, then `distill schedule install`.");
            })
        }
        Cmd::Sources => {
            let c = config().unwrap_or_default();
            let dets: Vec<_> = SourceKind::ALL
                .iter()
                .map(|&k| source::detect(k, &c.roots_for(k)))
                .collect();
            out(json, &dets, || {
                for d in &dets {
                    let enabled = c.source(d.kind).is_some_and(|s| s.enabled);
                    println!(
                        "{:<12} {:<9} {:>10} {:>7} files{}",
                        d.kind.id(),
                        if !d.found {
                            "missing"
                        } else if enabled {
                            "enabled"
                        } else {
                            "disabled"
                        },
                        human_bytes(d.bytes),
                        d.files,
                        if d.has_parser {
                            ""
                        } else {
                            "  (archived only, no parser)"
                        }
                    );
                }
            })
        }
        Cmd::Sweep { source } => {
            let only = source.as_deref().map(parse_source).transpose()?;
            let r = sweep::sweep(&home, &config()?, only)?;
            out(json, &r, || {
                println!("{}", r.summary());
                for e in r.errors.iter().take(10) {
                    eprintln!("  {e}");
                }
            })?;
            if !r.errors.is_empty() {
                bail!("{} sources failed to capture", r.errors.len());
            }
            Ok(())
        }
        Cmd::Verify => {
            let r = archive::Archive::open(&home)?.verify()?;
            let ok = r.ok();
            runlog::record(
                &home,
                &runlog::RunEntry::new(
                    "verify",
                    ok,
                    format!(
                        "Verified {} blobs: {} missing, {} corrupt",
                        r.checked,
                        r.missing.len(),
                        r.corrupt.len()
                    ),
                ),
            )?;
            out(json, &r, || {
                println!(
                    "Checked {} blobs ({}): {} missing, {} corrupt",
                    r.checked,
                    human_bytes(r.bytes),
                    r.missing.len(),
                    r.corrupt.len()
                )
            })?;
            if !ok {
                bail!("archive verification failed");
            }
            Ok(())
        }
        Cmd::ImportLegacy { from, dry_run } => {
            let db = match from {
                Some(p) => p,
                None => legacy::default_db()?,
            };
            let r = legacy::import(&home, &config()?, &db, dry_run)?;
            out(json, &r, || {
                println!("{} ({})", r.summary(), human_bytes(r.bytes));
                for p in r.mismatched.iter().take(10) {
                    eprintln!("  hash mismatch, skipped: {p}");
                }
                if r.imported > 0 && !dry_run {
                    println!("Run `distill normalize` (or wait for the next tick) to index them.");
                }
            })?;
            if !r.mismatched.is_empty() {
                bail!(
                    "{} legacy captures failed hash verification and were not imported",
                    r.mismatched.len()
                );
            }
            Ok(())
        }
        Cmd::Normalize { force } => {
            let r = normalize::normalize(&home, force)?;
            out(json, &r, || {
                println!(
                    "Normalized {} ({} unchanged, {} empty, {} failed, {} without a parser); {} sessions total",
                    r.normalized, r.skipped, r.empty, r.failed, r.no_parser, r.sessions
                )
            })
        }
        Cmd::Tick { steward } => {
            let r = tick::tick(&home, &config()?, steward)?;
            out(json, &r, || {
                println!("{}", r.sweep.summary());
                println!(
                    "Normalized {} sessions ({} failed)",
                    r.normalize.normalized, r.normalize.failed
                );
                if let Some(s) = &r.steward {
                    println!(
                        "Steward wrote {} cards (${:.4}), {} remaining",
                        s.carded, s.cost_usd, s.remaining
                    );
                }
                for b in &r.builds {
                    println!("Dataset build {b}");
                }
                for e in &r.errors {
                    eprintln!("  {e}");
                }
            })
        }
        Cmd::Steward { cmd } => match cmd {
            StewardCmd::Run { limit, dry_run } => {
                let r = steward::run(&home, &config()?, &RunOptions { limit, dry_run })?;
                out(json, &r, || {
                    if dry_run {
                        println!("{} sessions need a card", r.remaining);
                    } else {
                        println!(
                            "Wrote {} cards, {} failed, ${:.4} spent, {} remaining{}",
                            r.carded,
                            r.failed,
                            r.cost_usd,
                            r.remaining,
                            if r.stopped_for_budget {
                                " (stopped at the daily budget)"
                            } else {
                                ""
                            }
                        );
                        for e in r.errors.iter().take(5) {
                            eprintln!("  {e}");
                        }
                    }
                })
            }
            StewardCmd::Digest { source, session_id } => {
                let kind = parse_source(&source)?;
                let meta = normalize::find(&home, kind, &session_id)?.context("no such session")?;
                let events = normalize::read_events(&home, &meta)?;
                println!("{}", steward::digest(&meta, &events));
                Ok(())
            }
            StewardCmd::RepairBrief { source } => {
                let p = steward::repair_brief(&home, parse_source(&source)?)?;
                println!("{}", p.display());
                Ok(())
            }
            StewardCmd::Repair { source, repo } => {
                let branch = steward::repair(&home, &config()?, parse_source(&source)?, &repo)?;
                println!("Repair is on branch {branch}. Review and merge it to approve.");
                Ok(())
            }
        },
        Cmd::Status => {
            let d = stats::dashboard(&home)?;
            out(json, &d, || {
                let t = &d.totals;
                println!(
                    "{} sessions · {} archived · {:.0}% enriched · {} datasets · ${:.2} spent today",
                    t.sessions,
                    human_bytes(t.archive_bytes),
                    t.enriched * 100.0,
                    t.datasets,
                    t.spent_today_usd
                );
                println!("last sweep: {}", d.last_sweep.as_deref().unwrap_or("never"));
                println!(
                    "steward: {} ({})",
                    t.steward,
                    if t.steward_local {
                        "nothing leaves this machine"
                    } else {
                        "session excerpts leave this machine"
                    }
                );
                for s in &d.sources {
                    println!(
                        "  {:<12} {:<8} {:>6} sessions {:>10}  {}",
                        s.kind.id(),
                        format!("{:?}", s.health).to_lowercase(),
                        s.sessions,
                        human_bytes(s.archive_bytes),
                        s.note.as_deref().unwrap_or("")
                    );
                }
                if !d.needs_you.is_empty() {
                    println!("needs you:");
                    for n in &d.needs_you {
                        println!("  - {}", n.title());
                    }
                }
            })
        }
        Cmd::Sessions {
            source,
            grep,
            limit,
        } => {
            let f = ListFilter {
                source: source.as_deref().map(parse_source).transpose()?,
                text: grep,
                ..Default::default()
            };
            let list = search::list(&home, &f, limit)?;
            out(json, &list, || {
                for m in &list {
                    println!(
                        "{}  {:<12} {:<38} {:>4} tools  {}",
                        m.started_at
                            .as_deref()
                            .map(|s| &s[..s.len().min(16)])
                            .unwrap_or("                "),
                        m.source.id(),
                        m.session_id,
                        m.counts.tool_calls,
                        m.title.as_deref().unwrap_or("")
                    );
                }
            })
        }
        Cmd::Search {
            query,
            source,
            limit,
        } => {
            let hits = search::search(
                &home,
                &query,
                source.as_deref().map(parse_source).transpose()?,
                limit,
            )?;
            out(json, &hits, || {
                for h in &hits {
                    println!(
                        "{}/{} #{}: {}",
                        h.source.id(),
                        h.session_id,
                        h.seq,
                        h.snippet
                    );
                }
            })
        }
        Cmd::Recipe { cmd } => match cmd {
            RecipeCmd::List => {
                let names = export::list_recipes(&home)?;
                let proposals = propose::list(&home)?;
                out(json, &names, || {
                    names.iter().for_each(|n| println!("{n}"));
                    for p in &proposals {
                        println!(
                            "{}  (proposed, {} sessions: {})",
                            p.name, p.matches, p.recipe.description
                        );
                    }
                })
            }
            RecipeCmd::Proposals => {
                let proposals = propose::list(&home)?;
                out(json, &proposals, || {
                    for p in &proposals {
                        println!(
                            "{}  {} sessions  {}",
                            p.name, p.matches, p.recipe.description
                        );
                    }
                })
            }
            RecipeCmd::Propose => {
                let r = propose::propose(&home, &config()?)?;
                out(json, &r, || {
                    println!("{}", r.summary());
                    for p in &r.proposed {
                        println!(
                            "  {}  {} sessions  {}",
                            p.name, p.matches, p.recipe.description
                        );
                    }
                    for w in &r.rejected {
                        println!("  turned down: {w}");
                    }
                    if !r.proposed.is_empty() {
                        println!("Accept one with `distill recipe accept <name>`.");
                    }
                })
            }
            RecipeCmd::Accept { name } => {
                propose::accept(&home, &name)?;
                out(json, &serde_json::json!({"accepted": name}), || {
                    println!("{name} is now a recipe; the next tick builds it.")
                })
            }
            RecipeCmd::Dismiss { name } => {
                propose::dismiss(&home, &name)?;
                out(json, &serde_json::json!({"dismissed": name}), || {
                    println!("Dropped proposal {name}.")
                })
            }
            RecipeCmd::New { name } => {
                let path = export::recipe_path(&home, &name);
                if path.exists() {
                    bail!("{} already exists", path.display());
                }
                export::save_recipe(&home, &name, &export::default_recipe())?;
                println!("Wrote {}; edit it to change the filter.", path.display());
                Ok(())
            }
            RecipeCmd::Show { name } => {
                let r: Recipe = export::load_recipe(&home, &name)?;
                out(json, &r, || {
                    print!(
                        "{}",
                        std::fs::read_to_string(export::recipe_path(&home, &name))
                            .unwrap_or_default()
                    )
                })
            }
        },
        Cmd::Export { cmd } => match cmd {
            ExportCmd::Build { recipe } => {
                let names = match recipe {
                    Some(n) => vec![n],
                    None => export::list_recipes(&home)?,
                };
                let mut builds = Vec::new();
                for n in names {
                    builds.push(export::build(&home, &n)?);
                }
                out(json, &builds, || {
                    for b in &builds {
                        println!(
                            "{}/{}: {} records, {} redactions, {}",
                            b.recipe,
                            b.build,
                            b.records,
                            b.redactions,
                            if b.approved {
                                "approved"
                            } else {
                                "waiting for approval"
                            }
                        );
                    }
                })
            }
            ExportCmd::List => {
                let builds = export::list_builds(&home)?;
                out(json, &builds, || {
                    for b in &builds {
                        println!(
                            "{}  {}/{}  {} records  {}",
                            b.built_at,
                            b.recipe,
                            b.build,
                            b.records,
                            if b.approved { "approved" } else { "pending" }
                        );
                    }
                })
            }
            ExportCmd::Approve { recipe, build } => {
                export::approve(&home, &recipe, &build, "cli")?;
                println!("Approved {recipe}/{build}");
                Ok(())
            }
            ExportCmd::Publish { recipe, build, to } => {
                let p = export::publish(&home, &recipe, &build, &to)?;
                println!("{}", p.display());
                Ok(())
            }
        },
        Cmd::Runs { limit } => {
            let runs = runlog::recent(&home, limit)?;
            out(json, &runs, || {
                for r in &runs {
                    println!(
                        "{} {:<9} {} {}",
                        r.at,
                        r.step,
                        if r.ok { "ok  " } else { "FAIL" },
                        r.summary
                    );
                }
            })
        }
        Cmd::Schedule { cmd } => {
            let exe = std::env::current_exe()?;
            match cmd {
                ScheduleCmd::Print => {
                    let c = config()?;
                    if cfg!(target_os = "macos") {
                        print!("{}", schedule::launchd_plist(&exe, &home, &c));
                    } else {
                        let (s, t) = schedule::systemd_units(&exe, &home, &c);
                        println!("# distill-tick.service\n{s}\n# distill-tick.timer\n{t}");
                    }
                    Ok(())
                }
                ScheduleCmd::Install => {
                    let p = schedule::install(&exe, &home, &config()?)?;
                    println!("Installed {}", p.display());
                    Ok(())
                }
                ScheduleCmd::Uninstall => {
                    schedule::uninstall()?;
                    println!("Removed the Distill timer");
                    Ok(())
                }
                ScheduleCmd::Status => {
                    println!(
                        "{}",
                        if schedule::is_installed() {
                            "installed"
                        } else {
                            "not installed"
                        }
                    );
                    Ok(())
                }
            }
        }
        Cmd::Gui => {
            let exe = std::env::current_exe()?;
            let gui = exe.with_file_name(if cfg!(windows) {
                "distill-gui.exe"
            } else {
                "distill-gui"
            });
            let prog = if gui.exists() {
                gui
            } else {
                PathBuf::from("distill-gui")
            };
            let status = std::process::Command::new(&prog)
                .env("DISTILL_HOME", &home.root)
                .status()
                .with_context(|| {
                    format!(
                        "start {} (build it with `cargo build -p distill-gui`)",
                        prog.display()
                    )
                })?;
            if !status.success() {
                bail!("distill-gui exited with {status}");
            }
            Ok(())
        }
    }
}
