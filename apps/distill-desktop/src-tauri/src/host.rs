//! Testable Tauri host command runner over the public Library Fixture journey.

use std::path::{Path, PathBuf};

use distill_library::{
    FixtureJourneyPhase, FixtureJourneyResult, HealthReport, Library, RepairOptions, RepairReport,
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

/**
 * Validate renderer-supplied home and Fixture paths.
 *
 * Parameters:
 * - `home`: Distill home path string.
 * - `fixture_root`: Fixture root path string.
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
 *
 * Parameters:
 * - `home`: Distill home path string.
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
 * Run the Library Fixture journey and report typed progress phases.
 *
 * This function is intentionally synchronous so Tauri can schedule it off the
 * UI thread via `spawn_blocking`. It never exposes SQLite or filesystem handles
 * to callers beyond path strings already supplied by the host.
 *
 * Parameters:
 * - `request`: validated home and Fixture root.
 * - `on_progress`: phase observer used for typed host events.
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
 *
 * Parameters:
 * - `request`: validated Distill home.
 */
pub fn run_health(request: &HomeRequest) -> Result<HealthReport, HostError> {
    let library = Library::open(&request.home).map_err(HostError::from_library)?;
    library.health().map_err(HostError::from_library)
}

/**
 * Explicit Library repair after the renderer supplies confirmation.
 *
 * Parameters:
 * - `request`: validated Distill home.
 * - `confirm`: must be true; otherwise returns a validation error.
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
