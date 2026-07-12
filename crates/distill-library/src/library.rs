//! Public `Library` product seam.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use semver::Version;

use crate::adapter::{FixtureAdapter, ParserIdentity, SourceAdapter, FIXTURE_PARSER_ID};
use crate::curation;
use crate::error::{LibraryError, LibraryResult};
use crate::export;
use crate::health::{self as health_ops};
use crate::ingest;
use crate::ops::{self, new_owner_id};
use crate::query;
use crate::storage::{ensure_home_layout, migrate_to_latest, open_connection, DistillPaths};
use crate::types::{
    ActivityEventSummary, AttemptSummary, CurationMutationResult, ExportDataset, ExportPreview,
    ExportProgress, ExportProgressControl, ExportResult, FixtureJourneyPhase, FixtureJourneyResult,
    HealthReport, IngestReport, OpenReconciliation, RenormalizeReport, RepairOptions, RepairReport,
    SearchHit, SessionCurationRequest, SessionDetail, SessionDetailRequest, SessionListPage,
    SessionListRequest, SourceDetectRequest, SourceDetectResult, SourcePreference, SourceSummary,
    SyncProgress, SyncRequest, SyncRunResult, SyncRunSummary, DEFAULT_MAX_CAPTURE_BYTES,
    MAX_PAGE_SIZE,
};

/// Deep Distill Library over one Distill home.
pub struct Library {
    paths: DistillPaths,
    conn: Connection,
    max_capture_bytes: u64,
    /// Registered Fixture parser identity used for ingest and renormalize Attempts.
    fixture_parser: ParserIdentity,
    /// Safe reconciliation performed during the most recent open.
    open_reconciliation: OpenReconciliation,
    /// Owner id for Sync Runs started by this Library instance.
    owner_id: String,
}

impl Library {
    /**
     * Open or create a Distill home, apply checksummed migrations, and return a Library.
     *
     * Safe open reconciliation removes disposable staging partials only, then
     * idempotently fails stale Sync Run leases. Destructive orphan or incomplete-state
     * repair requires [`Self::repair`].
     *
     * Parameters:
     * - `home`: Distill home directory path. Created with mode `0o700` when missing.
     */
    pub fn open(home: impl AsRef<Path>) -> LibraryResult<Self> {
        Self::open_with_limits(home, DEFAULT_MAX_CAPTURE_BYTES)
    }

    /**
     * Open a Distill home with an explicit maximum Capture size.
     *
     * Parameters:
     * - `home`: Distill home directory.
     * - `max_capture_bytes`: rejection threshold for Capture acceptance.
     */
    pub fn open_with_limits(home: impl AsRef<Path>, max_capture_bytes: u64) -> LibraryResult<Self> {
        let paths = ensure_home_layout(home.as_ref())?;
        let mut conn = open_connection(&paths)?;
        migrate_to_latest(&mut conn)?;
        let mut open_reconciliation = health_ops::reconcile_on_open(&paths)?;
        let (classified_exports, removed_export_temps) =
            export::reconcile_incomplete_exports(&mut conn, &paths)?;
        open_reconciliation.classified_incomplete_exports = classified_exports;
        open_reconciliation.removed_export_temp_files = removed_export_temps;
        ops::fail_stale_active_runs(&mut conn)?;
        Ok(Self {
            paths,
            conn,
            max_capture_bytes,
            fixture_parser: ParserIdentity {
                id: FIXTURE_PARSER_ID.to_string(),
                version: crate::adapter::FIXTURE_PARSER_VERSION.to_string(),
            },
            open_reconciliation,
            owner_id: new_owner_id(),
        })
    }

    /// Absolute Distill home path.
    pub fn home(&self) -> &Path {
        &self.paths.home
    }

    /// Safe reconciliation report from the most recent open.
    pub fn open_reconciliation(&self) -> &OpenReconciliation {
        &self.open_reconciliation
    }

    /**
     * Update the registered Fixture parser version used for ingest and renormalize.
     *
     * The parser id remains the Library-owned `fixture` identity. Callers cannot
     * supply arbitrary parser ids.
     *
     * Parameters:
     * - `version`: Semantic version newer than the currently registered parser.
     */
    pub fn set_registered_fixture_parser_version(
        &mut self,
        version: impl Into<String>,
    ) -> LibraryResult<()> {
        let version = version.into();
        let requested = Version::parse(&version).map_err(|_| {
            crate::error::LibraryError::InvalidArgument(
                "fixture parser version must be a semantic version".into(),
            )
        })?;
        let current = Version::parse(&self.fixture_parser.version).map_err(|_| {
            crate::error::LibraryError::InvalidArgument(
                "registered fixture parser version is invalid".into(),
            )
        })?;
        if requested <= current {
            return Err(crate::error::LibraryError::InvalidArgument(
                "fixture parser version must advance beyond the registered version".into(),
            ));
        }
        self.fixture_parser.version = version;
        Ok(())
    }

    /**
     * List per-Source preferences, including closed kinds not yet upserted.
     */
    pub fn list_sources(&self) -> LibraryResult<Vec<SourcePreference>> {
        ops::list_source_preferences(&self.conn)
    }

    /**
     * Upsert enabled/disabled and optional configured-root preference for one Source.
     *
     * Parameters:
     * - `kind`: closed Source kind string.
     * - `enabled`: whether Sync may include this Source.
     * - `configured_root`: optional override directory; `None` clears the override.
     */
    pub fn set_source_preference(
        &mut self,
        kind: &str,
        enabled: bool,
        configured_root: Option<&Path>,
    ) -> LibraryResult<SourcePreference> {
        ops::upsert_source_preference(&self.conn, kind, enabled, configured_root)
    }

    /**
     * Detect each requested Source independently through typed results.
     *
     * Parameters:
     * - `requests`: one request per Source; failures never abort siblings.
     */
    pub fn detect_sources(
        &self,
        requests: &[SourceDetectRequest],
    ) -> LibraryResult<Vec<SourceDetectResult>> {
        ops::detect_sources(&self.conn, requests, &self.fixture_parser.version)
    }

    /**
     * Detect a Fixture root through the production SourceAdapter seam.
     *
     * Parameters:
     * - `fixture_root`: directory containing `distill.fixture.json`.
     */
    pub fn detect_fixture(&self, fixture_root: impl AsRef<Path>) -> LibraryResult<SourceSummary> {
        let adapter = if self.fixture_parser.version == crate::adapter::FIXTURE_PARSER_VERSION
            && self.fixture_parser.id == FIXTURE_PARSER_ID
        {
            FixtureAdapter::new(fixture_root.as_ref().to_path_buf())
        } else {
            FixtureAdapter::with_parser(
                fixture_root.as_ref().to_path_buf(),
                self.fixture_parser.clone(),
            )
        };
        let discovered = adapter.detect()?;
        Ok(SourceSummary {
            kind: discovered.kind.as_str().to_string(),
            display_name: discovered.display_name,
            data_root: discovered.data_root.display().to_string(),
            parser_id: discovered.parser.id,
            parser_version: discovered.parser.version,
        })
    }

    /**
     * Ingest a Fixture root through the production SourceAdapter seam.
     *
     * Parameters:
     * - `fixture_root`: directory containing `distill.fixture.json`.
     */
    pub fn ingest_fixture(
        &mut self,
        fixture_root: impl AsRef<Path>,
    ) -> LibraryResult<IngestReport> {
        let adapter = if self.fixture_parser.version == crate::adapter::FIXTURE_PARSER_VERSION
            && self.fixture_parser.id == FIXTURE_PARSER_ID
        {
            FixtureAdapter::new(fixture_root.as_ref().to_path_buf())
        } else {
            FixtureAdapter::with_parser(
                fixture_root.as_ref().to_path_buf(),
                self.fixture_parser.clone(),
            )
        };
        ingest::ingest_adapter(
            &mut self.conn,
            &self.paths,
            &adapter,
            self.max_capture_bytes,
        )
    }

    /**
     * Start a durable Sync Run over enabled Sources.
     *
     * A second overlapping start returns [`crate::LibraryError::SyncAlreadyRunning`]
     * with no Sync Run or Activity side effects. Selection with unknown kinds or zero
     * enabled Sources fails before any durable side effects.
     *
     * Parameters:
     * - `request`: optional Source kind filter.
     * - `on_progress`: typed progress observer for run/source/candidate events.
     */
    pub fn start_sync<F>(
        &mut self,
        request: SyncRequest,
        on_progress: F,
    ) -> LibraryResult<SyncRunResult>
    where
        F: FnMut(SyncProgress),
    {
        ops::start_sync(
            &mut self.conn,
            &self.paths,
            &self.owner_id,
            &self.fixture_parser,
            self.max_capture_bytes,
            &request,
            on_progress,
        )
    }

    /**
     * Request cancellation of an active Sync Run at the next safe checkpoint.
     *
     * Safe for a separate Library instance against the same Distill home.
     * Terminal runs are idempotent no-ops.
     *
     * Parameters:
     * - `sync_run_id`: durable Sync Run id.
     */
    pub fn request_sync_cancel(&mut self, sync_run_id: i64) -> LibraryResult<()> {
        ops::request_cancel(&self.conn, sync_run_id)
    }

    /**
     * Load Sync Run status. When `sync_run_id` is `None`, returns the latest run.
     *
     * Parameters:
     * - `sync_run_id`: optional Sync Run id.
     */
    pub fn sync_status(&self, sync_run_id: Option<i64>) -> LibraryResult<SyncRunSummary> {
        let id = match sync_run_id {
            Some(id) => id,
            None => self
                .conn
                .query_row(
                    "SELECT id FROM sync_runs ORDER BY id DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| LibraryError::NotFound("no sync runs".into()))?,
        };
        ops::load_sync_run(&self.conn, id)
    }

    /**
     * Re-normalize an accepted Capture from Distill-owned bytes.
     *
     * Uses the Library-registered Fixture parser. Does not create a new Capture and
     * does not accept caller-supplied parser ids.
     *
     * Parameters:
     * - `capture_id`: Accepted Capture row id.
     */
    pub fn renormalize_capture(&mut self, capture_id: i64) -> LibraryResult<RenormalizeReport> {
        ingest::renormalize_capture(
            &mut self.conn,
            &self.paths,
            capture_id,
            &self.fixture_parser,
        )
    }

    /**
     * List immutable Attempt summaries for one Capture, oldest first.
     *
     * Parameters:
     * - `capture_id`: Accepted Capture row id.
     */
    pub fn capture_attempts(&self, capture_id: i64) -> LibraryResult<Vec<AttemptSummary>> {
        query::list_capture_attempts(&self.conn, capture_id)
    }

    /**
     * Run the first-run Fixture journey for thin CLI and desktop callers.
     *
     * Detects the Fixture Source, ingests through the production seam, loads the
     * first projected Session Identity when present, and returns Library health.
     * Progress callbacks observe phase transitions only; they do not receive
     * storage handles or SQL.
     *
     * Parameters:
     * - `fixture_root`: directory containing `distill.fixture.json`.
     * - `on_progress`: optional phase observer for host/CLI progress surfaces.
     */
    pub fn run_fixture_journey<F>(
        &mut self,
        fixture_root: impl AsRef<Path>,
        mut on_progress: F,
    ) -> LibraryResult<FixtureJourneyResult>
    where
        F: FnMut(FixtureJourneyPhase),
    {
        let fixture_root = fixture_root.as_ref();
        on_progress(FixtureJourneyPhase::DetectingSource);
        let source = self.detect_fixture(fixture_root)?;

        on_progress(FixtureJourneyPhase::SyncingCaptures);
        let sync = self.ingest_fixture(fixture_root)?;

        on_progress(FixtureJourneyPhase::LoadingSession);
        let session = if let Some(identity) = sync.session_identities.first() {
            self.session_slice(&identity.source_kind, &identity.external_session_id, 20, 20)?
        } else {
            None
        };

        on_progress(FixtureJourneyPhase::CheckingHealth);
        let health = self.health()?;

        Ok(FixtureJourneyResult {
            source,
            sync,
            session,
            health,
        })
    }

    /**
     * Load a bounded first slice of one Session Projection by identity.
     *
     * Parameters:
     * - `source_kind`: Source kind string such as `fixture`.
     * - `external_session_id`: Stable Session Identity.
     */
    pub fn session_slice(
        &self,
        source_kind: &str,
        external_session_id: &str,
        message_limit: u32,
        artifact_limit: u32,
    ) -> LibraryResult<Option<SessionDetail>> {
        validate_page_size(message_limit)?;
        validate_page_size(artifact_limit)?;
        query::get_session(
            &self.conn,
            source_kind,
            external_session_id,
            message_limit,
            artifact_limit,
        )
    }

    /**
     * Load a bounded Session Projection detail page with optional continuation cursors.
     *
     * Parameters:
     * - `request`: identity plus message/artifact page bounds and opaque cursors.
     */
    pub fn session_detail(
        &self,
        request: SessionDetailRequest,
    ) -> LibraryResult<Option<SessionDetail>> {
        validate_page_size(request.message_limit)?;
        validate_page_size(request.artifact_limit)?;
        query::session_detail(&self.conn, &request)
    }

    /**
     * List or search current Session Projections with lane filter and keyset cursor paging.
     *
     * Parameters:
     * - `request`: optional query text, workflow lane, limit, and opaque cursor.
     */
    pub fn list_sessions(&self, request: SessionListRequest) -> LibraryResult<SessionListPage> {
        validate_page_size(request.limit)?;
        query::list_sessions(&self.conn, &request)
    }

    /**
     * Search the current Session Projection via FTS5.
     *
     * Parameters:
     * - `query`: free-text query. Punctuation-only input returns no hits.
     */
    pub fn search(&self, query_text: &str, limit: u32) -> LibraryResult<Vec<SearchHit>> {
        validate_page_size(limit)?;
        query::search(&self.conn, query_text, limit)
    }

    /**
     * Replay Distill-owned Capture bytes by Capture id.
     *
     * Parameters:
     * - `capture_id`: accepted Capture row id.
     */
    pub fn replay_capture(&self, capture_id: i64) -> LibraryResult<Vec<u8>> {
        query::replay_capture(&self.conn, &self.paths.home, capture_id)
    }

    /**
     * Report schema, content, FTS, staging, orphan, incomplete-state, and Sync health.
     */
    pub fn health(&self) -> LibraryResult<HealthReport> {
        health_ops::health(&self.conn, &self.paths, &self.open_reconciliation)
    }

    /**
     * Explicit idempotent repair for documented repairable states.
     *
     * Destructive actions require caller-selected [`RepairOptions`]. Safe staging
     * cleanup is always included. Referenced content and immutable Captures/Facts
     * are never deleted.
     *
     * Parameters:
     * - `options`: which documented destructive repairs to perform.
     */
    pub fn repair(&mut self, options: RepairOptions) -> LibraryResult<RepairReport> {
        health_ops::repair(
            &mut self.conn,
            &self.paths,
            &options,
            &self.open_reconciliation,
        )
    }

    /**
     * List Activity Events for contract assertions.
     */
    pub fn recent_activity(&self, limit: u32) -> LibraryResult<Vec<ActivityEventSummary>> {
        validate_page_size(limit)?;
        query::list_activity(&self.conn, limit)
    }

    /**
     * Add a manual tag to a session by Session Identity.
     *
     * Blank names, missing sessions, and duplicate assignments are typed no-ops
     * (`changed: false`) with no Activity side effects.
     *
     * Parameters:
     * - `request`: `(source_kind, external_session_id)` plus tag name.
     */
    pub fn add_session_tag(
        &mut self,
        request: SessionCurationRequest,
    ) -> LibraryResult<CurationMutationResult> {
        curation::add_session_tag(&mut self.conn, request)
    }

    /**
     * Remove a manual tag from a session by Session Identity.
     *
     * Blank names, missing sessions, unknown tags, and missing assignments are
     * typed no-ops (`changed: false`) with no Activity side effects.
     *
     * Parameters:
     * - `request`: `(source_kind, external_session_id)` plus tag name.
     */
    pub fn remove_session_tag(
        &mut self,
        request: SessionCurationRequest,
    ) -> LibraryResult<CurationMutationResult> {
        curation::remove_session_tag(&mut self.conn, request)
    }

    /**
     * Toggle a seeded catalog label on a session by Session Identity.
     *
     * Enabling a dataset label removes conflicting dataset labels in the same
     * transaction. Blank names, unknown labels, and missing sessions are typed
     * no-ops (`changed: false`) with no Activity side effects.
     *
     * Parameters:
     * - `request`: `(source_kind, external_session_id)` plus label name.
     */
    pub fn toggle_session_label(
        &mut self,
        request: SessionCurationRequest,
    ) -> LibraryResult<CurationMutationResult> {
        curation::toggle_session_label(&mut self.conn, request)
    }

    /**
     * Preview a `distill-session-jsonl-v1` dataset export without side effects.
     *
     * Shares publish eligibility policy over the current projection and manual
     * curation. Performs no filesystem, export-row, or Activity mutations.
     *
     * Parameters:
     * - `dataset`: approved `train` or `holdout` target.
     */
    pub fn preview_export(&self, dataset: ExportDataset) -> LibraryResult<ExportPreview> {
        export::preview_export(&self.conn, dataset)
    }

    /**
     * Publish a recoverable Library-owned `distill-session-jsonl-v1` export.
     *
     * Uses the same eligibility snapshot as [`Self::preview_export`], writes under
     * `<distill-home>/exports`, and reaches `published` only after same-volume
     * rename plus matching SQLite bookkeeping and `export_written` Activity.
     *
     * Parameters:
     * - `dataset`: approved `train` or `holdout` target.
     * - `on_progress`: typed progress observer that may request cancellation.
     */
    pub fn publish_export<F>(
        &mut self,
        dataset: ExportDataset,
        on_progress: F,
    ) -> LibraryResult<ExportResult>
    where
        F: FnMut(ExportProgress) -> ExportProgressControl,
    {
        export::publish_export(&mut self.conn, &self.paths, dataset, on_progress)
    }
}

fn validate_page_size(limit: u32) -> LibraryResult<()> {
    if limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(crate::error::LibraryError::InvalidArgument(format!(
            "page size must be between 1 and {MAX_PAGE_SIZE}"
        )));
    }
    Ok(())
}
