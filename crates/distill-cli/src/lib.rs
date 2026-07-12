//! Thin Distill CLI over the public Library Fixture journey, health, and repair.
//!
//! Exit codes:
//! - `0` — command succeeded
//! - `1` — Library or runtime failure
//! - `2` — usage / invalid arguments

#![deny(missing_docs)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use distill_library::{
    FixtureJourneyPhase, FixtureJourneyResult, HealthReport, Library, LibraryError, RepairOptions,
    RepairReport,
};
use serde::Serialize;

/// Documented CLI exit code for a successful command.
pub const EXIT_SUCCESS: u8 = 0;
/// Documented CLI exit code for Library or runtime failures.
pub const EXIT_RUNTIME: u8 = 1;
/// Documented CLI exit code for usage or invalid arguments.
pub const EXIT_USAGE: u8 = 2;

/// Output format for CLI results.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    /// Stable human-readable multi-line summary.
    #[default]
    Human,
    /// Stable JSON document for machine callers.
    Json,
}

/// Distill CLI arguments.
///
/// When no subcommand is provided, `--home` and `--fixture` run the Fixture journey
/// (preserved thin-caller contract). Owning `health` and `repair` subcommands cover
/// issue #21 recovery surfaces.
#[derive(Debug, Parser)]
#[command(
    name = "distill",
    about = "Thin Distill CLI for Library Fixture journey, health, and repair",
    disable_help_subcommand = true,
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    /// Owning health/repair commands. Absent means Fixture journey mode.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Distill home directory to open or create (journey mode).
    #[arg(long, value_name = "PATH")]
    pub home: Option<PathBuf>,

    /// Fixture root containing `distill.fixture.json` (journey mode).
    #[arg(long, value_name = "PATH")]
    pub fixture: Option<PathBuf>,

    /// Result output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
}

/// Owning CLI commands beyond the default Fixture journey.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Report Library health for a Distill home.
    Health {
        /// Distill home directory to open.
        #[arg(long, value_name = "PATH")]
        home: PathBuf,

        /// Result output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
    /// Explicitly repair documented repairable Library states.
    Repair {
        /// Distill home directory to open.
        #[arg(long, value_name = "PATH")]
        home: PathBuf,

        /// Required confirmation flag for destructive repair actions.
        #[arg(long)]
        confirm: bool,

        /// Result output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
}

impl Cli {
    /// Parse CLI arguments from process environment.
    pub fn parse_from_env() -> Self {
        Self::parse()
    }
}

/// Typed CLI error payload for JSON failures.
#[derive(Debug, Serialize)]
pub struct CliErrorBody {
    /// Stable error class string.
    pub error: String,
    /// Human-readable detail.
    pub message: String,
}

/**
 * Execute the selected CLI command and write stable output to stdout/stderr.
 *
 * Parameters:
 * - `cli`: parsed CLI arguments.
 */
pub fn run(cli: Cli) -> ExitCode {
    match execute(cli) {
        Ok(output) => {
            println!("{output}");
            ExitCode::from(EXIT_SUCCESS)
        }
        Err(failure) => {
            eprintln!("{}", failure.render());
            ExitCode::from(failure.code)
        }
    }
}

/// Internal CLI failure with documented exit code.
struct CliFailure {
    code: u8,
    format: OutputFormat,
    body: CliErrorBody,
}

impl CliFailure {
    fn usage(format: OutputFormat, message: impl Into<String>) -> Self {
        Self {
            code: EXIT_USAGE,
            format,
            body: CliErrorBody {
                error: "usage".to_string(),
                message: message.into(),
            },
        }
    }

    fn runtime(format: OutputFormat, err: LibraryError) -> Self {
        Self {
            code: EXIT_RUNTIME,
            format,
            body: CliErrorBody {
                error: err.code().to_string(),
                message: err.to_string(),
            },
        }
    }

    fn render(&self) -> String {
        match self.format {
            OutputFormat::Human => format!("{}: {}", self.body.error, self.body.message),
            OutputFormat::Json => serde_json::to_string(&self.body)
                .unwrap_or_else(|_| "{\"error\":\"json\",\"message\":\"serialize failed\"}".into()),
        }
    }
}

/**
 * Validate inputs and dispatch journey, health, or repair.
 */
fn execute(cli: Cli) -> Result<String, CliFailure> {
    match cli.command {
        Some(Command::Health { home, format }) => execute_health(home, format),
        Some(Command::Repair {
            home,
            confirm,
            format,
        }) => execute_repair(home, confirm, format),
        None => execute_journey(cli.home, cli.fixture, cli.format),
    }
}

/**
 * Run the Fixture journey using the legacy flat `--home` / `--fixture` flags.
 */
fn execute_journey(
    home: Option<PathBuf>,
    fixture: Option<PathBuf>,
    format: OutputFormat,
) -> Result<String, CliFailure> {
    let home = home.ok_or_else(|| CliFailure::usage(format, "home path is required"))?;
    let fixture = fixture.ok_or_else(|| CliFailure::usage(format, "fixture path is required"))?;
    if home.as_os_str().is_empty() {
        return Err(CliFailure::usage(format, "home path must not be empty"));
    }
    if fixture.as_os_str().is_empty() {
        return Err(CliFailure::usage(format, "fixture path must not be empty"));
    }
    if !fixture.is_dir() {
        return Err(CliFailure::usage(
            format,
            format!("fixture root is not a directory: {}", fixture.display()),
        ));
    }

    let mut library = Library::open(&home).map_err(|err| CliFailure::runtime(format, err))?;
    let mut phases = Vec::new();
    let result = library
        .run_fixture_journey(&fixture, |phase| phases.push(phase))
        .map_err(|err| CliFailure::runtime(format, err))?;

    Ok(render_journey_success(format, &result, &phases))
}

/**
 * Open a Distill home and print typed health.
 */
fn execute_health(home: PathBuf, format: OutputFormat) -> Result<String, CliFailure> {
    if home.as_os_str().is_empty() {
        return Err(CliFailure::usage(format, "home path must not be empty"));
    }
    let library = Library::open(&home).map_err(|err| CliFailure::runtime(format, err))?;
    let health = library
        .health()
        .map_err(|err| CliFailure::runtime(format, err))?;
    Ok(render_health(format, &health))
}

/**
 * Open a Distill home and run explicit documented repair after `--confirm`.
 */
fn execute_repair(
    home: PathBuf,
    confirm: bool,
    format: OutputFormat,
) -> Result<String, CliFailure> {
    if home.as_os_str().is_empty() {
        return Err(CliFailure::usage(format, "home path must not be empty"));
    }
    if !confirm {
        return Err(CliFailure::usage(
            format,
            "repair requires --confirm because it performs destructive cleanup",
        ));
    }
    let mut library = Library::open(&home).map_err(|err| CliFailure::runtime(format, err))?;
    let report = library
        .repair(RepairOptions::all_documented())
        .map_err(|err| CliFailure::runtime(format, err))?;
    Ok(render_repair(format, &report))
}

/**
 * Render a successful journey in the requested format.
 */
fn render_journey_success(
    format: OutputFormat,
    result: &FixtureJourneyResult,
    phases: &[FixtureJourneyPhase],
) -> String {
    match format {
        OutputFormat::Json => {
            let payload = serde_json::json!({
                "ok": true,
                "phases": phases,
                "source": result.source,
                "sync": result.sync,
                "session": result.session,
                "health": result.health,
            });
            serde_json::to_string_pretty(&payload).expect("json serialize")
        }
        OutputFormat::Human => {
            let mut lines = Vec::new();
            lines.push("ok: true".to_string());
            lines.push(format!("source.kind: {}", result.source.kind));
            lines.push(format!(
                "source.display_name: {}",
                result.source.display_name
            ));
            lines.push(format!("source.data_root: {}", result.source.data_root));
            lines.push(format!(
                "sync.accepted_captures: {}",
                result.sync.accepted_captures
            ));
            lines.push(format!(
                "sync.successful_attempts: {}",
                result.sync.successful_attempts
            ));
            lines.push(format!(
                "sync.failed_attempts: {}",
                result.sync.failed_attempts
            ));
            if let Some(session) = &result.session {
                lines.push(format!(
                    "session.identity: {}:{}",
                    session.summary.source_kind, session.summary.external_session_id
                ));
                lines.push(format!(
                    "session.title: {}",
                    session.summary.title.as_deref().unwrap_or("")
                ));
                lines.push(format!(
                    "session.accepted_capture_count: {}",
                    session.summary.accepted_capture_count
                ));
                lines.push(format!(
                    "session.normalization_attempt_count: {}",
                    session.summary.normalization_attempt_count
                ));
                lines.push(format!(
                    "session.successful_projection_generation: {}",
                    session.summary.successful_projection_generation
                ));
                lines.push(format!("session.messages: {}", session.messages.len()));
            } else {
                lines.push("session: none".to_string());
            }
            lines.push(format!("health.ok: {}", result.health.ok));
            lines.push(format!(
                "health.schema_status: {}",
                result.health.schema_status
            ));
            lines.join("\n")
        }
    }
}

/**
 * Render a health report for CLI callers.
 */
fn render_health(format: OutputFormat, health: &HealthReport) -> String {
    match format {
        OutputFormat::Json => {
            let payload = serde_json::json!({
                "ok": health.ok,
                "health": health,
            });
            serde_json::to_string_pretty(&payload).expect("json serialize")
        }
        OutputFormat::Human => {
            let mut lines = vec![
                format!("ok: {}", health.ok),
                format!("health.schema_status: {}", health.schema_status),
                format!("health.content_status: {}", health.content_status),
                format!("health.fts_status: {}", health.fts_status),
                format!("health.staging_status: {}", health.staging_status),
                format!("health.orphan_status: {}", health.orphan_status),
                format!("health.incomplete_status: {}", health.incomplete_status),
                format!("health.operations_status: {}", health.operations_status),
                format!(
                    "health.open_reconciliation.removed_staging_partials: {}",
                    health.open_reconciliation.removed_staging_partials
                ),
                format!("health.issues: {}", health.issues.len()),
            ];
            for issue in &health.issues {
                lines.push(format!(
                    "health.issue: {} {} {} {}",
                    issue.code, issue.severity, issue.category, issue.summary
                ));
            }
            lines.join("\n")
        }
    }
}

/**
 * Render a repair report for CLI callers.
 */
fn render_repair(format: OutputFormat, report: &RepairReport) -> String {
    match format {
        OutputFormat::Json => {
            let payload = serde_json::json!({
                "ok": report.health_after.ok,
                "repair": report,
            });
            serde_json::to_string_pretty(&payload).expect("json serialize")
        }
        OutputFormat::Human => {
            let mut lines = vec![
                format!("ok: {}", report.health_after.ok),
                format!("repair.actions: {}", report.actions.len()),
            ];
            for action in &report.actions {
                lines.push(format!("repair.action: {} {}", action.name, action.count));
            }
            lines.push(format!("health.ok: {}", report.health_after.ok));
            lines.push(format!(
                "health.incomplete_status: {}",
                report.health_after.incomplete_status
            ));
            lines.join("\n")
        }
    }
}
