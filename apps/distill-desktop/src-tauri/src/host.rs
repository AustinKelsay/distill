//! Testable Tauri host command runner over the public Library seam.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use distill_library::{
    CurationMutationResult, ExportDataset, ExportPreview, ExportProgress, ExportProgressControl,
    ExportResult, FixtureJourneyPhase, FixtureJourneyResult, HealthReport, Library, RepairOptions,
    RepairReport, SessionCurationRequest, SessionDetail, SessionDetailRequest, SessionListPage,
    SessionListRequest, SourcePreference, SyncProgress, SyncRequest, SyncRunResult, SyncRunSummary,
};

use crate::error::HostError;

/// Validated Fixture journey request from the renderer or tests.
#[derive(Clone, Debug)]
pub struct FixtureJourneyRequest {
    /// Distill home directory.
    pub home: PathBuf,
    /// Fixture root containing `distill.fixture.json`.
    pub fixture_root: PathBuf,
}

/// Validated Distill-home-only request for health and repair.
#[derive(Clone, Debug)]
pub struct HomeRequest {
    /// Distill home directory.
    pub home: PathBuf,
}

/// Validated Sync start request.
#[derive(Clone, Debug)]
pub struct SyncStartRequest {
    /// Distill home directory.
    pub home: PathBuf,
    /// Optional Source kind filter.
    pub source_kinds: Vec<String>,
}

/// Validated Sync cancel/status request.
#[derive(Clone, Debug)]
pub struct SyncIdRequest {
    /// Distill home directory.
    pub home: PathBuf,
    /// Sync Run id.
    pub sync_run_id: i64,
}

/// Validated Source preference update.
#[derive(Clone, Debug)]
pub struct SourcePreferenceRequest {
    /// Distill home directory.
    pub home: PathBuf,
    /// Source kind.
    pub kind: String,
    /// Enabled flag.
    pub enabled: bool,
    /// Optional configured root.
    pub configured_root: Option<PathBuf>,
}

/// Validated export preview/publish request.
#[derive(Clone, Debug)]
pub struct ExportRequest {
    /// Distill home directory.
    pub home: PathBuf,
    /// Approved `train` or `holdout` dataset target.
    pub dataset: ExportDataset,
}

struct ExportCancellationState {
    token: std::sync::atomic::AtomicBool,
    started: std::sync::atomic::AtomicBool,
}

type ExportCancellation = Arc<ExportCancellationState>;

static EXPORT_CANCELLATIONS: OnceLock<Mutex<HashMap<String, ExportCancellation>>> = OnceLock::new();

fn export_cancellation_key(request: &ExportRequest) -> String {
    format!("{}\0{}", request.home.display(), request.dataset.as_str())
}

fn export_cancellation_registry() -> &'static Mutex<HashMap<String, ExportCancellation>> {
    EXPORT_CANCELLATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct ExportCancellationGuard {
    key: String,
    token: ExportCancellation,
}

impl Drop for ExportCancellationGuard {
    fn drop(&mut self) {
        if let Ok(mut entries) = export_cancellation_registry().lock() {
            if entries
                .get(&self.key)
                .is_some_and(|current| Arc::ptr_eq(current, &self.token))
            {
                entries.remove(&self.key);
            }
        }
    }
}

fn acquire_export_cancellation(
    request: &ExportRequest,
) -> Result<ExportCancellationGuard, HostError> {
    let key = export_cancellation_key(request);
    let mut entries = export_cancellation_registry()
        .lock()
        .map_err(|_| HostError {
            code: "runtime".to_string(),
            message: "export cancellation registry poisoned".to_string(),
        })?;
    let token = entries
        .entry(key.clone())
        .or_insert_with(|| {
            Arc::new(ExportCancellationState {
                token: std::sync::atomic::AtomicBool::new(false),
                started: std::sync::atomic::AtomicBool::new(false),
            })
        })
        .clone();
    token
        .started
        .store(true, std::sync::atomic::Ordering::Release);
    Ok(ExportCancellationGuard { key, token })
}

/** Register an export cancellation intent before starting its worker. */
pub fn run_prepare_export_cancellation(request: &ExportRequest) -> Result<bool, HostError> {
    let key = export_cancellation_key(request);
    let mut entries = export_cancellation_registry()
        .lock()
        .map_err(|_| HostError {
            code: "runtime".to_string(),
            message: "export cancellation registry poisoned".to_string(),
        })?;
    if let Some(state) = entries.get(&key) {
        return Ok(!state
            .started
            .swap(true, std::sync::atomic::Ordering::AcqRel));
    }
    entries.insert(
        key,
        Arc::new(ExportCancellationState {
            token: std::sync::atomic::AtomicBool::new(false),
            started: std::sync::atomic::AtomicBool::new(true),
        }),
    );
    Ok(true)
}

/**
 * List/search current Session Projections through the public Library seam.
 */
pub fn run_list_sessions(
    request: &HomeRequest,
    page: SessionListRequest,
) -> Result<SessionListPage, HostError> {
    let library = Library::open(&request.home).map_err(HostError::from_library)?;
    library.list_sessions(page).map_err(HostError::from_library)
}

/**
 * Load one bounded current-projection Session detail page.
 */
pub fn run_session_detail(
    request: &HomeRequest,
    detail: SessionDetailRequest,
) -> Result<Option<SessionDetail>, HostError> {
    let library = Library::open(&request.home).map_err(HostError::from_library)?;
    library
        .session_detail(detail)
        .map_err(HostError::from_library)
}

/**
 * Validate Distill home plus Session Identity fields for a curation mutation.
 */
pub fn validate_session_curation_request(
    home: &str,
    request: SessionCurationRequest,
) -> Result<(HomeRequest, SessionCurationRequest), HostError> {
    let home_request = validate_home_request(home)?;
    let source_kind = request.source_kind.trim();
    let external_session_id = request.external_session_id.trim();
    if source_kind.is_empty() {
        return Err(HostError::validation("source kind must not be empty"));
    }
    if external_session_id.is_empty() {
        return Err(HostError::validation(
            "external session id must not be empty",
        ));
    }
    Ok((
        home_request,
        SessionCurationRequest {
            source_kind: source_kind.to_string(),
            external_session_id: external_session_id.to_string(),
            name: request.name,
        },
    ))
}

/**
 * Add a manual tag through the public Library seam.
 */
pub fn run_add_session_tag(
    request: &HomeRequest,
    curation: SessionCurationRequest,
) -> Result<CurationMutationResult, HostError> {
    let mut library = Library::open(&request.home).map_err(HostError::from_library)?;
    library
        .add_session_tag(curation)
        .map_err(HostError::from_library)
}

/**
 * Remove a manual tag through the public Library seam.
 */
pub fn run_remove_session_tag(
    request: &HomeRequest,
    curation: SessionCurationRequest,
) -> Result<CurationMutationResult, HostError> {
    let mut library = Library::open(&request.home).map_err(HostError::from_library)?;
    library
        .remove_session_tag(curation)
        .map_err(HostError::from_library)
}

/**
 * Toggle a catalog label through the public Library seam.
 */
pub fn run_toggle_session_label(
    request: &HomeRequest,
    curation: SessionCurationRequest,
) -> Result<CurationMutationResult, HostError> {
    let mut library = Library::open(&request.home).map_err(HostError::from_library)?;
    library
        .toggle_session_label(curation)
        .map_err(HostError::from_library)
}

/**
 * Validate renderer-supplied home and Fixture paths.
 */
pub fn validate_fixture_journey_request(
    home: &str,
    fixture_root: &str,
) -> Result<FixtureJourneyRequest, HostError> {
    let home = home.trim();
    let fixture_root = fixture_root.trim();
    if home.is_empty() {
        return Err(HostError::validation("home path must not be empty"));
    }
    if fixture_root.is_empty() {
        return Err(HostError::validation("fixture root must not be empty"));
    }
    let fixture_path = PathBuf::from(fixture_root);
    if !fixture_path.is_dir() {
        return Err(HostError::validation(format!(
            "fixture root is not a directory: {fixture_root}"
        )));
    }
    Ok(FixtureJourneyRequest {
        home: PathBuf::from(home),
        fixture_root: fixture_path,
    })
}

/**
 * Validate a Distill home path for health or repair commands.
 */
pub fn validate_home_request(home: &str) -> Result<HomeRequest, HostError> {
    let home = home.trim();
    if home.is_empty() {
        return Err(HostError::validation("home path must not be empty"));
    }
    Ok(HomeRequest {
        home: PathBuf::from(home),
    })
}

/**
 * Validate a Sync start request.
 */
pub fn validate_sync_start_request(
    home: &str,
    source_kinds: Vec<String>,
) -> Result<SyncStartRequest, HostError> {
    let request = validate_home_request(home)?;
    Ok(SyncStartRequest {
        home: request.home,
        source_kinds,
    })
}

/**
 * Validate a Sync id request.
 */
pub fn validate_sync_id_request(home: &str, sync_run_id: i64) -> Result<SyncIdRequest, HostError> {
    let request = validate_home_request(home)?;
    if sync_run_id <= 0 {
        return Err(HostError::validation("sync run id must be positive"));
    }
    Ok(SyncIdRequest {
        home: request.home,
        sync_run_id,
    })
}

/**
 * Validate a Source preference update.
 */
pub fn validate_source_preference_request(
    home: &str,
    kind: &str,
    enabled: bool,
    configured_root: Option<&str>,
) -> Result<SourcePreferenceRequest, HostError> {
    let request = validate_home_request(home)?;
    let kind = kind.trim();
    if kind.is_empty() {
        return Err(HostError::validation("source kind must not be empty"));
    }
    let configured_root = match configured_root {
        Some(root) if root.trim().is_empty() => {
            return Err(HostError::validation(
                "configured root must not be empty when provided",
            ));
        }
        Some(root) => Some(PathBuf::from(root.trim())),
        None => None,
    };
    Ok(SourcePreferenceRequest {
        home: request.home,
        kind: kind.to_string(),
        enabled,
        configured_root,
    })
}

/**
 * Run the Library Fixture journey and report typed progress phases.
 */
pub fn run_fixture_journey<F>(
    request: &FixtureJourneyRequest,
    mut on_progress: F,
) -> Result<FixtureJourneyResult, HostError>
where
    F: FnMut(FixtureJourneyPhase),
{
    let mut library = Library::open(&request.home).map_err(HostError::from_library)?;
    library
        .run_fixture_journey(Path::new(&request.fixture_root), |phase| {
            on_progress(phase);
        })
        .map_err(HostError::from_library)
}

/**
 * Open a Distill home and return typed health.
 */
pub fn run_health(request: &HomeRequest) -> Result<HealthReport, HostError> {
    let library = Library::open(&request.home).map_err(HostError::from_library)?;
    library.health().map_err(HostError::from_library)
}

/**
 * Explicit Library repair after the renderer supplies confirmation.
 */
pub fn run_repair(request: &HomeRequest, confirm: bool) -> Result<RepairReport, HostError> {
    if !confirm {
        return Err(HostError::validation(
            "repair requires explicit confirmation because it performs destructive cleanup",
        ));
    }
    let mut library = Library::open(&request.home).map_err(HostError::from_library)?;
    library
        .repair(RepairOptions::all_documented())
        .map_err(HostError::from_library)
}

/**
 * List Source preferences.
 */
pub fn run_list_sources(request: &HomeRequest) -> Result<Vec<SourcePreference>, HostError> {
    let library = Library::open(&request.home).map_err(HostError::from_library)?;
    library.list_sources().map_err(HostError::from_library)
}

/**
 * Upsert one Source preference.
 */
pub fn run_set_source_preference(
    request: &SourcePreferenceRequest,
) -> Result<SourcePreference, HostError> {
    let mut library = Library::open(&request.home).map_err(HostError::from_library)?;
    library
        .set_source_preference(
            &request.kind,
            request.enabled,
            request.configured_root.as_deref(),
        )
        .map_err(HostError::from_library)
}

/**
 * Start a Sync Run off the UI thread with typed progress.
 */
pub fn run_sync_start<F>(
    request: &SyncStartRequest,
    on_progress: F,
) -> Result<SyncRunResult, HostError>
where
    F: FnMut(SyncProgress),
{
    let mut library = Library::open(&request.home).map_err(HostError::from_library)?;
    library
        .start_sync(
            SyncRequest {
                source_kinds: request.source_kinds.clone(),
            },
            on_progress,
        )
        .map_err(HostError::from_library)
}

/**
 * Load Sync Run status.
 */
pub fn run_sync_status(
    request: &HomeRequest,
    sync_run_id: Option<i64>,
) -> Result<SyncRunSummary, HostError> {
    let library = Library::open(&request.home).map_err(HostError::from_library)?;
    library
        .sync_status(sync_run_id)
        .map_err(HostError::from_library)
}

/**
 * Request Sync Run cancellation.
 */
pub fn run_sync_cancel(request: &SyncIdRequest) -> Result<SyncRunSummary, HostError> {
    let mut library = Library::open(&request.home).map_err(HostError::from_library)?;
    library
        .request_sync_cancel(request.sync_run_id)
        .map_err(HostError::from_library)?;
    library
        .sync_status(Some(request.sync_run_id))
        .map_err(HostError::from_library)
}

/**
 * Validate Distill home plus closed export dataset target.
 *
 * Parameters:
 * - `home`: Distill home path from the renderer
 * - `dataset`: caller-supplied dataset string (`train` or `holdout`)
 */
pub fn validate_export_request(home: &str, dataset: &str) -> Result<ExportRequest, HostError> {
    let request = validate_home_request(home)?;
    let dataset = ExportDataset::parse(dataset).map_err(HostError::validation)?;
    Ok(ExportRequest {
        home: request.home,
        dataset,
    })
}

/**
 * Preview export eligibility through the public Library seam.
 *
 * Parameters:
 * - `request`: validated home and dataset
 */
pub fn run_preview_export(request: &ExportRequest) -> Result<ExportPreview, HostError> {
    let library = Library::open(&request.home).map_err(HostError::from_library)?;
    library
        .preview_export(request.dataset)
        .map_err(HostError::from_library)
}

/**
 * Publish a recoverable export and report typed progress.
 *
 * The compatibility runner continues publication; callers that need
 * cancellation can use `run_publish_export_cancellable` or the typed control
 * seam below.
 *
 * Parameters:
 * - `request`: validated home and dataset
 * - `on_progress`: typed export progress observer
 */
pub fn run_publish_export<F>(
    request: &ExportRequest,
    mut on_progress: F,
) -> Result<ExportResult, HostError>
where
    F: FnMut(ExportProgress),
{
    run_publish_export_with_control(request, |progress| {
        on_progress(progress);
        ExportProgressControl::Continue
    })
}

/**
 * Publish an export while allowing the caller to request cancellation at a
 * typed Library checkpoint.
 *
 * Parameters:
 * - `request`: validated home and dataset
 * - `on_progress`: typed observer/control callback
 */
pub fn run_publish_export_with_control<F>(
    request: &ExportRequest,
    on_progress: F,
) -> Result<ExportResult, HostError>
where
    F: FnMut(ExportProgress) -> ExportProgressControl,
{
    let mut library = Library::open(&request.home).map_err(HostError::from_library)?;
    library
        .publish_export(request.dataset, on_progress)
        .map_err(HostError::from_library)
}

/**
 * Publish an export with a desktop cancellation token registered for the
 * duration of the blocking operation.
 */
pub fn run_publish_export_cancellable<F>(
    request: &ExportRequest,
    mut on_progress: F,
) -> Result<ExportResult, HostError>
where
    F: FnMut(ExportProgress),
{
    let guard = acquire_export_cancellation(request)?;
    let state = Arc::clone(&guard.token);

    run_publish_export_with_control(request, |progress| {
        on_progress(progress);
        if state.token.load(std::sync::atomic::Ordering::Acquire) {
            ExportProgressControl::Cancel
        } else {
            ExportProgressControl::Continue
        }
    })
}

/**
 * Request cancellation for an active desktop export.
 *
 * Returns `true` when an active publication was found and signalled.
 */
pub fn run_export_cancel(request: &ExportRequest) -> Result<bool, HostError> {
    let mut entries = export_cancellation_registry()
        .lock()
        .map_err(|_| HostError {
            code: "runtime".to_string(),
            message: "export cancellation registry poisoned".to_string(),
        })?;
    let state = entries
        .entry(export_cancellation_key(request))
        .or_insert_with(|| {
            Arc::new(ExportCancellationState {
                token: std::sync::atomic::AtomicBool::new(true),
                started: std::sync::atomic::AtomicBool::new(false),
            })
        });
    state
        .token
        .store(true, std::sync::atomic::Ordering::Release);
    Ok(true)
}
