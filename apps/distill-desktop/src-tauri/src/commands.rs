//! Tauri IPC command adapters for Distill desktop.

use distill_library::{FixtureJourneyResult, HealthReport, RepairReport};
use tauri::{AppHandle, Emitter};

use crate::error::HostError;
use crate::host::{
    run_fixture_journey, run_health, run_repair, validate_fixture_journey_request,
    validate_home_request,
};
use crate::FIXTURE_JOURNEY_PROGRESS_EVENT;

/**
 * Tauri command: validate inputs, run the Fixture journey off the UI thread,
 * emit typed progress, and return source/sync/session/health results.
 *
 * Parameters:
 * - `app`: Tauri app handle used to emit progress events.
 * - `home`: Distill home path.
 * - `fixture_root`: Fixture root path.
 */
#[tauri::command]
pub async fn run_fixture_journey_command(
    app: AppHandle,
    home: String,
    fixture_root: String,
) -> Result<FixtureJourneyResult, HostError> {
    let request = validate_fixture_journey_request(&home, &fixture_root)?;
    tauri::async_runtime::spawn_blocking(move || {
        run_fixture_journey(&request, |phase| {
            let _ = app.emit(FIXTURE_JOURNEY_PROGRESS_EVENT, phase);
        })
    })
    .await
    .map_err(|err| HostError {
        code: "join".to_string(),
        message: err.to_string(),
    })?
}

/**
 * Tauri command: open a Distill home and return typed health.
 *
 * Parameters:
 * - `home`: Distill home path.
 */
#[tauri::command]
pub async fn health_command(home: String) -> Result<HealthReport, HostError> {
    let request = validate_home_request(&home)?;
    tauri::async_runtime::spawn_blocking(move || run_health(&request))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: explicit Library repair after renderer confirmation.
 *
 * Parameters:
 * - `home`: Distill home path.
 * - `confirm`: must be true to authorize destructive repair.
 */
#[tauri::command]
pub async fn repair_command(home: String, confirm: bool) -> Result<RepairReport, HostError> {
    let request = validate_home_request(&home)?;
    tauri::async_runtime::spawn_blocking(move || run_repair(&request, confirm))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}
