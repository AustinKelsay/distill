//! Testable Tauri host command runner over the public Library seam.

use std::path::{Path, PathBuf};

use distill_library::{
    CurationMutationResult, FixtureJourneyPhase, FixtureJourneyResult, HealthReport, Library,
    RepairOptions, RepairReport, SessionCurationRequest, SessionDetail, SessionDetailRequest,
    SessionListPage, SessionListRequest, SourcePreference, SyncProgress, SyncRequest,
    SyncRunResult, SyncRunSummary,
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
