//! Thin Distill CLI over the public Library Fixture journey.
//!
//! Exit codes:
//! - `0` — journey succeeded
//! - `1` — Library or runtime failure
//! - `2` — usage / invalid arguments

#![deny(missing_docs)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use distill_library::{FixtureJourneyPhase, FixtureJourneyResult, Library, LibraryError};
use serde::Serialize;

/// Documented CLI exit code for a successful Fixture journey.
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
#[derive(Debug, Parser)]
#[command(
    name = "distill",
    about = "Thin Distill CLI for the Fixture Library journey",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Distill home directory to open or create.
    #[arg(long, value_name = "PATH")]
    pub home: PathBuf,

    /// Fixture root containing `distill.fixture.json`.
    #[arg(long, value_name = "PATH")]
    pub fixture: PathBuf,

    /// Result output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
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
 * Execute the CLI Fixture journey and write stable output to stdout/stderr.
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
 * Validate inputs, run the Library Fixture journey, and format the result.
 */
fn execute(cli: Cli) -> Result<String, CliFailure> {
    let format = cli.format;
    if cli.home.as_os_str().is_empty() {
        return Err(CliFailure::usage(format, "home path must not be empty"));
    }
    if cli.fixture.as_os_str().is_empty() {
        return Err(CliFailure::usage(format, "fixture path must not be empty"));
    }
    if !cli.fixture.is_dir() {
        return Err(CliFailure::usage(
            format,
            format!("fixture root is not a directory: {}", cli.fixture.display()),
        ));
    }

    let mut library = Library::open(&cli.home).map_err(|err| CliFailure::runtime(format, err))?;
    let mut phases = Vec::new();
    let result = library
        .run_fixture_journey(&cli.fixture, |phase| phases.push(phase))
        .map_err(|err| CliFailure::runtime(format, err))?;

    Ok(render_success(format, &result, &phases))
}

/**
 * Render a successful journey in the requested format.
 */
fn render_success(
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
