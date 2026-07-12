//! Thin Distill CLI over the public Library Fixture journey, health, repair,
//! Source preferences, and Sync Runs.
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
    RepairReport, SessionDetailRequest, SessionListRequest, SourcePreference, SyncProgress,
    SyncRequest, SyncRunResult, SyncRunSummary, WorkflowLane,
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
#[derive(Debug, Parser)]
#[command(
    name = "distill",
    about = "Thin Distill CLI for Library Fixture journey, health, repair, sources, and sync",
    disable_help_subcommand = true,
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    /// Owning commands. Absent means Fixture journey mode.
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
    /// Source preference commands.
    Sources {
        /// Nested source list/set action.
        #[command(subcommand)]
        action: SourcesCommand,
    },
    /// Sync Run start/status/cancel commands.
    Sync {
        /// Nested sync start/status/cancel action.
        #[command(subcommand)]
        action: SyncCommand,
    },
    /// Current-projection session list/search/detail commands.
    Sessions {
        /// Nested session list/detail action.
        #[command(subcommand)]
        action: SessionCommand,
    },
}

/// Session query subcommands.
#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// List or search current sessions with a workflow lane and cursor.
    List {
        /// Distill home directory.
        #[arg(long, value_name = "PATH")]
        home: PathBuf,
        /// Optional Unicode-safe current-projection search query.
        #[arg(long)]
        query: Option<String>,
        /// Workflow lane (`all`, `needs-review`, `train-ready`, `holdout-ready`, `favorites`).
        #[arg(long, default_value = "all")]
        lane: String,
        /// Opaque continuation cursor.
        #[arg(long)]
        cursor: Option<String>,
        /// Page size.
        #[arg(long, default_value_t = 50)]
        limit: u32,
        /// Result output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
    /// Load bounded current-projection session detail.
    Detail {
        /// Distill home directory.
        #[arg(long, value_name = "PATH")]
        home: PathBuf,
        /// Source kind such as `fixture`.
        #[arg(long)]
        source_kind: String,
        /// Stable external Session Identity.
        #[arg(long)]
        external_session_id: String,
        /// Message page size.
        #[arg(long, default_value_t = 50)]
        message_limit: u32,
        /// Artifact page size.
        #[arg(long, default_value_t = 50)]
        artifact_limit: u32,
        /// Opaque message continuation cursor.
        #[arg(long)]
        message_cursor: Option<String>,
        /// Opaque artifact continuation cursor.
        #[arg(long)]
        artifact_cursor: Option<String>,
        /// Result output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
}

/// Source preference subcommands.
#[derive(Debug, Subcommand)]
pub enum SourcesCommand {
    /// List per-Source preferences.
    List {
        /// Distill home directory.
        #[arg(long, value_name = "PATH")]
        home: PathBuf,
        /// Result output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
    /// Set enabled/disabled and optional configured-root for one Source.
    Set {
        /// Distill home directory.
        #[arg(long, value_name = "PATH")]
        home: PathBuf,
        /// Closed Source kind string.
        #[arg(long)]
        kind: String,
        /// Enable the Source for Sync Runs.
        #[arg(long, group = "enabled_flag")]
        enable: bool,
        /// Disable the Source for Sync Runs.
        #[arg(long, group = "enabled_flag")]
        disable: bool,
        /// Optional configured-root override.
        #[arg(long, value_name = "PATH")]
        root: Option<PathBuf>,
        /// Clear any configured-root override.
        #[arg(long)]
        clear_root: bool,
        /// Result output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
}

/// Sync Run subcommands.
#[derive(Debug, Subcommand)]
pub enum SyncCommand {
    /// Start a Sync Run over enabled Sources.
    Start {
        /// Distill home directory.
        #[arg(long, value_name = "PATH")]
        home: PathBuf,
        /// Optional Source kind filter (repeatable).
        #[arg(long = "kind", value_name = "KIND")]
        kinds: Vec<String>,
        /// Result output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
    /// Show Sync Run status.
    Status {
        /// Distill home directory.
        #[arg(long, value_name = "PATH")]
        home: PathBuf,
        /// Optional Sync Run id; defaults to latest.
        #[arg(long, value_name = "ID")]
        id: Option<i64>,
        /// Result output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
    /// Request Sync Run cancellation at the next safe checkpoint.
    Cancel {
        /// Distill home directory.
        #[arg(long, value_name = "PATH")]
        home: PathBuf,
        /// Sync Run id to cancel.
        #[arg(long, value_name = "ID")]
        id: i64,
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

fn execute(cli: Cli) -> Result<String, CliFailure> {
    match cli.command {
        Some(Command::Health { home, format }) => execute_health(home, format),
        Some(Command::Repair {
            home,
            confirm,
            format,
        }) => execute_repair(home, confirm, format),
        Some(Command::Sources { action }) => execute_sources(action),
        Some(Command::Sync { action }) => execute_sync(action),
        Some(Command::Sessions { action }) => execute_sessions(action),
        None => execute_journey(cli.home, cli.fixture, cli.format),
    }
}

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

fn execute_sources(action: SourcesCommand) -> Result<String, CliFailure> {
    match action {
        SourcesCommand::List { home, format } => {
            let library = Library::open(&home).map_err(|err| CliFailure::runtime(format, err))?;
            let sources = library
                .list_sources()
                .map_err(|err| CliFailure::runtime(format, err))?;
            Ok(render_sources(format, &sources))
        }
        SourcesCommand::Set {
            home,
            kind,
            enable,
            disable,
            root,
            clear_root,
            format,
        } => {
            if !enable && !disable {
                return Err(CliFailure::usage(
                    format,
                    "sources set requires --enable or --disable",
                ));
            }
            if enable && disable {
                return Err(CliFailure::usage(
                    format,
                    "sources set cannot use both --enable and --disable",
                ));
            }
            if root.is_some() && clear_root {
                return Err(CliFailure::usage(
                    format,
                    "sources set cannot use both --root and --clear-root",
                ));
            }
            let configured_root = if clear_root { None } else { root.as_deref() };
            // When neither root nor clear_root is supplied, preserve existing root by
            // reading current preference first.
            let mut library =
                Library::open(&home).map_err(|err| CliFailure::runtime(format, err))?;
            let existing_root = if root.is_none() && !clear_root {
                library
                    .list_sources()
                    .map_err(|err| CliFailure::runtime(format, err))?
                    .into_iter()
                    .find(|pref| pref.kind == kind)
                    .and_then(|pref| pref.configured_root)
            } else {
                None
            };
            let root_path = configured_root
                .map(std::path::Path::new)
                .or(existing_root.as_ref().map(std::path::Path::new));
            let pref = library
                .set_source_preference(&kind, enable, root_path)
                .map_err(|err| CliFailure::runtime(format, err))?;
            Ok(render_source(format, &pref))
        }
    }
}

fn execute_sync(action: SyncCommand) -> Result<String, CliFailure> {
    match action {
        SyncCommand::Start {
            home,
            kinds,
            format,
        } => {
            let mut library =
                Library::open(&home).map_err(|err| CliFailure::runtime(format, err))?;
            let mut progress = Vec::new();
            let result = library
                .start_sync(
                    SyncRequest {
                        source_kinds: kinds,
                    },
                    |event| progress.push(event),
                )
                .map_err(|err| CliFailure::runtime(format, err))?;
            Ok(render_sync_result(format, &result, &progress))
        }
        SyncCommand::Status { home, id, format } => {
            let library = Library::open(&home).map_err(|err| CliFailure::runtime(format, err))?;
            let summary = library
                .sync_status(id)
                .map_err(|err| CliFailure::runtime(format, err))?;
            Ok(render_sync_status(format, &summary))
        }
        SyncCommand::Cancel { home, id, format } => {
            let mut library =
                Library::open(&home).map_err(|err| CliFailure::runtime(format, err))?;
            library
                .request_sync_cancel(id)
                .map_err(|err| CliFailure::runtime(format, err))?;
            let summary = library
                .sync_status(Some(id))
                .map_err(|err| CliFailure::runtime(format, err))?;
            Ok(render_sync_status(format, &summary))
        }
    }
}

fn parse_workflow_lane(format: OutputFormat, value: &str) -> Result<WorkflowLane, CliFailure> {
    match value.trim().to_ascii_lowercase().as_str() {
        "all" => Ok(WorkflowLane::All),
        "needs-review" | "needs_review" => Ok(WorkflowLane::NeedsReview),
        "train-ready" | "train_ready" => Ok(WorkflowLane::TrainReady),
        "holdout-ready" | "holdout_ready" => Ok(WorkflowLane::HoldoutReady),
        "favorites" | "favorite" => Ok(WorkflowLane::Favorites),
        _ => Err(CliFailure::usage(
            format,
            "lane must be all, needs-review, train-ready, holdout-ready, or favorites",
        )),
    }
}

fn execute_sessions(action: SessionCommand) -> Result<String, CliFailure> {
    match action {
        SessionCommand::List {
            home,
            query,
            lane,
            cursor,
            limit,
            format,
        } => {
            let lane = parse_workflow_lane(format, &lane)?;
            let library = Library::open(&home).map_err(|err| CliFailure::runtime(format, err))?;
            let page = library
                .list_sessions(SessionListRequest {
                    query,
                    lane,
                    limit,
                    cursor,
                })
                .map_err(|err| CliFailure::runtime(format, err))?;
            Ok(render_session_page(format, &page))
        }
        SessionCommand::Detail {
            home,
            source_kind,
            external_session_id,
            message_limit,
            artifact_limit,
            message_cursor,
            artifact_cursor,
            format,
        } => {
            let library = Library::open(&home).map_err(|err| CliFailure::runtime(format, err))?;
            let detail = library
                .session_detail(SessionDetailRequest {
                    source_kind,
                    external_session_id,
                    message_limit,
                    artifact_limit,
                    message_cursor,
                    artifact_cursor,
                })
                .map_err(|err| CliFailure::runtime(format, err))?
                .ok_or_else(|| {
                    CliFailure::runtime(format, LibraryError::NotFound("session".into()))
                })?;
            Ok(render_session_detail(format, &detail))
        }
    }
}

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

fn render_sources(format: OutputFormat, sources: &[SourcePreference]) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "sources": sources,
        }))
        .expect("json"),
        OutputFormat::Human => sources
            .iter()
            .map(|pref| {
                format!(
                    "source.{} enabled={} root={}",
                    pref.kind,
                    pref.enabled,
                    pref.configured_root.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn render_source(format: OutputFormat, pref: &SourcePreference) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "source": pref,
        }))
        .expect("json"),
        OutputFormat::Human => format!(
            "source.{} enabled={} root={}",
            pref.kind,
            pref.enabled,
            pref.configured_root.as_deref().unwrap_or("")
        ),
    }
}

fn render_sync_result(
    format: OutputFormat,
    result: &SyncRunResult,
    progress: &[SyncProgress],
) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "progress": progress,
            "run": result.run,
            "session_identities": result.session_identities,
        }))
        .expect("json"),
        OutputFormat::Human => {
            let mut lines = vec![
                format!("ok: true"),
                format!("sync.id: {}", result.run.id),
                format!("sync.status: {}", result.run.status),
                format!("sync.accepted_captures: {}", result.run.accepted_captures),
                format!(
                    "sync.successful_attempts: {}",
                    result.run.successful_attempts
                ),
                format!("sync.failed_attempts: {}", result.run.failed_attempts),
            ];
            for source in &result.run.sources {
                lines.push(format!(
                    "sync.source.{} status={}",
                    source.source_kind, source.status
                ));
            }
            for detail in &result.run.warning_details {
                lines.push(format!("sync.warning: {detail}"));
            }
            lines.join("\n")
        }
    }
}

fn render_sync_status(format: OutputFormat, summary: &SyncRunSummary) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "run": summary,
        }))
        .expect("json"),
        OutputFormat::Human => {
            let mut lines = vec![
                format!("sync.id: {}", summary.id),
                format!("sync.status: {}", summary.status),
                format!("sync.cancel_requested: {}", summary.cancel_requested),
                format!("sync.accepted_captures: {}", summary.accepted_captures),
            ];
            for detail in &summary.warning_details {
                lines.push(format!("sync.warning: {detail}"));
            }
            lines.join("\n")
        }
    }
}

fn render_session_page(format: OutputFormat, page: &distill_library::SessionListPage) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "items": page.items,
            "next_cursor": page.next_cursor,
        }))
        .expect("json"),
        OutputFormat::Human => {
            let mut lines = vec![format!("sessions.count: {}", page.items.len())];
            for item in &page.items {
                lines.push(format!(
                    "session: {}:{} title={} workflow={:?} messages={}",
                    item.source_kind,
                    item.external_session_id,
                    item.title,
                    item.workflow_state,
                    item.message_count
                ));
            }
            if let Some(cursor) = &page.next_cursor {
                lines.push(format!("sessions.next_cursor: {cursor}"));
            }
            lines.join("\n")
        }
    }
}

fn render_session_detail(format: OutputFormat, detail: &distill_library::SessionDetail) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "session": detail,
        }))
        .expect("json"),
        OutputFormat::Human => {
            let mut lines = vec![
                format!(
                    "session.identity: {}:{}",
                    detail.summary.source_kind, detail.summary.external_session_id
                ),
                format!(
                    "session.title: {}",
                    detail.summary.title.as_deref().unwrap_or("")
                ),
                format!(
                    "session.project_path: {}",
                    detail.project_path.as_deref().unwrap_or("")
                ),
                format!(
                    "session.source_url: {}",
                    detail.source_url.as_deref().unwrap_or("")
                ),
                format!("session.raw_capture_count: {}", detail.raw_capture_count),
                format!("session.messages: {}", detail.messages.len()),
                format!("session.artifacts: {}", detail.artifacts.len()),
                format!("session.workflow_state: {:?}", detail.workflow_state),
            ];
            if let Some(cursor) = &detail.next_message_cursor {
                lines.push(format!("session.next_message_cursor: {cursor}"));
            }
            if let Some(cursor) = &detail.next_artifact_cursor {
                lines.push(format!("session.next_artifact_cursor: {cursor}"));
            }
            lines.join("\n")
        }
    }
}
