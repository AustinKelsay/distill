//! Public and shared Library value types.

use serde::{Deserialize, Serialize};

/// Captures at or below this size may be stored inline in SQLite.
/// Larger captures are written to the content-addressed blob store.
pub const INLINE_CONTENT_THRESHOLD_BYTES: u64 = 64 * 1024;

/// Default maximum accepted capture size (64 MiB).
pub const DEFAULT_MAX_CAPTURE_BYTES: u64 = 64 * 1024 * 1024;

/// Maximum number of rows returned by any first-tracer query slice.
pub const MAX_PAGE_SIZE: u32 = 200;

/// Stable Session Identity for thin callers (CLI, host, renderer).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionIdentity {
    /// Source kind string such as `fixture`.
    pub source_kind: String,
    /// Source-provided or deterministic synthetic Session identifier.
    pub external_session_id: String,
}

/// Caller-facing Source observation after Fixture detection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceSummary {
    /// Source kind string such as `fixture`.
    pub kind: String,
    /// Human-readable Source label.
    pub display_name: String,
    /// Absolute data root path as a UTF-8 string.
    pub data_root: String,
    /// Parser identity used for Normalization Attempts.
    pub parser_id: String,
    /// Parser contract version.
    pub parser_version: String,
}

/// Progress phases for the first-run Fixture journey.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureJourneyPhase {
    /// Detecting the Fixture Source.
    DetectingSource,
    /// Ingesting Capture Candidates through the production seam.
    SyncingCaptures,
    /// Loading the first projected Session Identity.
    LoadingSession,
    /// Running Library health.
    CheckingHealth,
}

/// Combined first-run Fixture journey result for thin callers.
///
/// `sync` is the Fixture ingest report for this journey, not a generic Sync Run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FixtureJourneyResult {
    /// Detected Fixture Source summary.
    pub source: SourceSummary,
    /// Fixture ingest counters and Session Identities touched by this run.
    pub sync: IngestReport,
    /// First Session Projection loaded after ingest, when present.
    pub session: Option<SessionDetail>,
    /// Library health after the journey.
    pub health: HealthReport,
}

/// Summary returned after Fixture ingest through the production seam.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IngestReport {
    /// Newly accepted Capture count.
    pub accepted_captures: u64,
    /// Exact-duplicate Captures skipped.
    pub skipped_duplicates: u64,
    /// Successful Normalization Attempts.
    pub successful_attempts: u64,
    /// Failed Normalization Attempts.
    pub failed_attempts: u64,
    /// Accepted Capture row ids in discovery order.
    pub capture_ids: Vec<i64>,
    /// Distinct Session Identities successfully projected during this ingest.
    pub session_identities: Vec<SessionIdentity>,
}

/// Compact Session list row.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSummary {
    /// Database session id.
    pub id: i64,
    /// Source kind string (e.g. `fixture`).
    pub source_kind: String,
    /// Stable Session Identity external id.
    pub external_session_id: String,
    /// Current projection title when present.
    pub title: Option<String>,
    /// Accepted Capture count for this Session Identity.
    pub accepted_capture_count: i64,
    /// Normalization Attempt count for this Session Identity.
    pub normalization_attempt_count: i64,
    /// Latest successful projection generation.
    pub successful_projection_generation: i64,
}

/// Full Session Projection detail for query callers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionDetail {
    /// Compact Session identity and counters.
    pub summary: SessionSummary,
    /// Ordered projected Transcript Messages.
    pub messages: Vec<ProjectedMessage>,
    /// Projected Artifacts for the current generation.
    pub artifacts: Vec<ProjectedArtifact>,
    /// Current Session metadata JSON object as text.
    pub metadata_json: String,
}

/// One projected Transcript Message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectedMessage {
    /// Message row id.
    pub id: i64,
    /// Projection ordinal.
    pub ordinal: i64,
    /// Message role.
    pub role: String,
    /// `text` or `meta`.
    pub message_kind: String,
    /// Visible text body.
    pub text: String,
}

/// One projected Artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectedArtifact {
    /// Artifact row id.
    pub id: i64,
    /// Artifact type label.
    pub artifact_type: String,
    /// Optional linked projected message.
    pub message_id: Option<i64>,
    /// Optional linked Capture Fact.
    pub capture_fact_id: Option<i64>,
    /// Optional text preview.
    pub text_preview: Option<String>,
}

/// FTS hit against the current projection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchHit {
    /// Session row id.
    pub session_id: i64,
    /// Matching message row id.
    pub message_id: i64,
    /// Source kind.
    pub source_kind: String,
    /// External Session Identity.
    pub external_session_id: String,
    /// Message role.
    pub role: String,
    /// Matching text snippet.
    pub text: String,
}

/// Append-only Activity Event summary for assertions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivityEventSummary {
    /// Event type string.
    pub event_type: String,
    /// Optional Capture id.
    pub capture_id: Option<i64>,
    /// Optional Session id.
    pub session_id: Option<i64>,
}

/// Typed health issue with stable codes and redacted summaries.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HealthIssue {
    /// Stable machine-readable issue code.
    pub code: String,
    /// Issue severity: `blocking`, `repairable`, or `info`.
    pub severity: String,
    /// Issue category: `schema`, `content`, `fts`, `staging`, `orphan`, or `incomplete`.
    pub category: String,
    /// Redacted human summary without raw paths or payloads.
    pub summary: String,
}

/// Safe actions performed while opening a Distill home.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct OpenReconciliation {
    /// Disposable staging partial files removed on open.
    pub removed_staging_partials: u64,
}

/// Caller options for explicit destructive Library repair.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RepairOptions {
    /// Delete CAS blobs not referenced by any Capture.
    pub remove_orphan_blobs: bool,
    /// Resolve incomplete Captures and pending Attempts with safe recovery bookkeeping.
    pub resolve_incomplete_state: bool,
    /// Rebuild FTS rows from current projection messages when identity/content disagree.
    pub rebuild_fts: bool,
}

impl RepairOptions {
    /// Enable every documented destructive repair action.
    pub fn all_documented() -> Self {
        Self {
            remove_orphan_blobs: true,
            resolve_incomplete_state: true,
            rebuild_fts: true,
        }
    }
}

/// One named repair action with an affected-row/file count.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RepairAction {
    /// Stable snake_case action name.
    pub name: String,
    /// Number of entities affected by this action.
    pub count: u64,
}

/// Result of an explicit Library repair call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepairReport {
    /// Named actions performed (zero counts when already clean / idempotent).
    pub actions: Vec<RepairAction>,
    /// Health after repair.
    pub health_after: HealthReport,
}

/// Library health report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthReport {
    /// True when no integrity or incomplete-state issues were found.
    pub ok: bool,
    /// Schema and migration status text (`ok` or `failed`).
    pub schema_status: String,
    /// Referenced content presence/checksum status (`ok` or `failed`).
    pub content_status: String,
    /// Projection/FTS identity+content agreement (`ok` or `failed`).
    pub fts_status: String,
    /// Disposable staging partial status (`ok` or `failed`).
    pub staging_status: String,
    /// Unreferenced CAS blob status (`ok` or `failed`).
    pub orphan_status: String,
    /// Incomplete Capture/Attempt/projection status (`ok` or `failed`).
    pub incomplete_status: String,
    /// Sync/operations status: `ok`, `active`, or `failed` when Sync Runs exist.
    pub operations_status: String,
    /// Typed integrity and recovery issues.
    pub issues: Vec<HealthIssue>,
    /// Safe reconciliation performed on the most recent open.
    pub open_reconciliation: OpenReconciliation,
}

/// Persisted per-Source preference surfaced to callers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourcePreference {
    /// Source kind string such as `fixture`.
    pub kind: String,
    /// Whether the Source is eligible for Sync Runs.
    pub enabled: bool,
    /// Optional canonical configured-root override.
    pub configured_root: Option<String>,
    /// Last observed display name when known.
    pub display_name: Option<String>,
    /// Last observed data root when known.
    pub data_root: Option<String>,
}

/// One independent Source detection request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceDetectRequest {
    /// Source kind to detect.
    pub kind: String,
    /// Optional configured-root override for this detection only.
    pub configured_root: Option<String>,
}

/// Independent typed detection outcome for one requested Source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceDetectResult {
    /// Requested Source kind.
    pub kind: String,
    /// Typed health/status: `ok`, `disabled`, `unavailable`, `missing`, or `unhealthy`.
    pub status: String,
    /// Executable path or name when applicable; `None` for Fixture.
    pub executable: Option<String>,
    /// Effective data root when detection succeeded.
    pub effective_data_root: Option<String>,
    /// Human-readable label when known.
    pub display_name: Option<String>,
    /// Typed error class when not ok.
    pub error_class: Option<String>,
    /// Redacted safe diagnostic message.
    pub error_message: Option<String>,
}

/// Request to start a Sync Run over enabled Sources.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyncRequest {
    /// Optional Source kind filter. Empty means every enabled Source.
    pub source_kinds: Vec<String>,
}

/// Typed Sync Run progress events for CLI/host/renderer surfaces.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncProgress {
    /// Sync Run row queued with matching Activity.
    RunQueued {
        /// Durable Sync Run id.
        sync_run_id: i64,
    },
    /// Sync Run transitioned to running.
    RunStarted {
        /// Durable Sync Run id.
        sync_run_id: i64,
    },
    /// About to process one Source.
    SourceStarted {
        /// Durable Sync Run id.
        sync_run_id: i64,
        /// Source kind string.
        source_kind: String,
    },
    /// Finished processing one Source.
    SourceFinished {
        /// Durable Sync Run id.
        sync_run_id: i64,
        /// Source kind string.
        source_kind: String,
        /// Per-source outcome status.
        status: String,
    },
    /// About to process one Capture Candidate (logical identity only).
    CandidateStarted {
        /// Durable Sync Run id.
        sync_run_id: i64,
        /// Source kind string.
        source_kind: String,
        /// Logical source path or virtual identity (never absolute FS diagnostics).
        candidate_id: String,
    },
    /// Finished one Capture Candidate.
    CandidateFinished {
        /// Durable Sync Run id.
        sync_run_id: i64,
        /// Source kind string.
        source_kind: String,
        /// Logical candidate identity.
        candidate_id: String,
        /// Candidate outcome: `accepted`, `skipped_duplicate`, `failed`, or `cancelled`.
        outcome: String,
    },
}

/// Compact Sync Run summary for status queries.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyncRunSummary {
    /// Durable Sync Run id.
    pub id: i64,
    /// Explicit status: queued/running/completed/warning/failed/cancelled.
    pub status: String,
    /// Whether a cancel request is pending.
    pub cancel_requested: bool,
    /// Aggregate accepted Capture count.
    pub accepted_captures: u64,
    /// Aggregate skipped duplicates.
    pub skipped_duplicates: u64,
    /// Aggregate successful Attempts.
    pub successful_attempts: u64,
    /// Aggregate failed Attempts.
    pub failed_attempts: u64,
    /// Typed terminal error class when failed/cancelled.
    pub error_class: Option<String>,
    /// Redacted terminal message when present.
    pub error_message: Option<String>,
    /// Redacted details for non-fatal warning outcomes.
    pub warning_details: Vec<String>,
    /// Per-source outcome summaries.
    pub sources: Vec<SyncSourceOutcome>,
}

/// Per-Source outcome within one Sync Run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyncSourceOutcome {
    /// Source kind string.
    pub source_kind: String,
    /// Outcome status for this Source within the run.
    pub status: String,
    /// Accepted Captures for this Source.
    pub accepted_captures: u64,
    /// Skipped duplicates for this Source.
    pub skipped_duplicates: u64,
    /// Successful Attempts for this Source.
    pub successful_attempts: u64,
    /// Failed Attempts for this Source.
    pub failed_attempts: u64,
    /// Typed error class when failed.
    pub error_class: Option<String>,
    /// Redacted error message when failed.
    pub error_message: Option<String>,
}

/// Terminal Sync Run result returned by [`crate::Library::start_sync`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncRunResult {
    /// Final Sync Run summary.
    pub run: SyncRunSummary,
    /// Distinct Session Identities projected during this run.
    pub session_identities: Vec<SessionIdentity>,
}

/// Caller-safe Normalization Attempt summary with immutable Fact counts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttemptSummary {
    /// Attempt row id.
    pub id: i64,
    /// Accepted Capture this Attempt belongs to.
    pub capture_id: i64,
    /// Registered parser identity.
    pub parser_id: String,
    /// Registered parser version used for this Attempt.
    pub parser_version: String,
    /// `pending`, `succeeded`, or `failed`.
    pub outcome: String,
    /// Typed failure class when failed (`parse_failed` or `projection_failed`).
    pub error_class: Option<String>,
    /// Safe diagnostic message when failed.
    pub error_message: Option<String>,
    /// Successful projection generation when this Attempt published one.
    pub projection_generation: Option<i64>,
    /// Immutable Capture Fact count owned by this Attempt.
    pub fact_count: i64,
}

/// Result of re-normalizing an accepted Capture with the registered parser.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenormalizeReport {
    /// Capture that was re-normalized without creating a new Capture.
    pub capture_id: i64,
    /// Newly recorded Attempt id.
    pub attempt_id: i64,
    /// Final Attempt outcome (`succeeded` or `failed`).
    pub outcome: String,
    /// Registered parser identity used for the Attempt.
    pub parser_id: String,
    /// Registered parser version used for the Attempt.
    pub parser_version: String,
}
