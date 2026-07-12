//! Tauri IPC command adapters for Distill desktop.

use distill_library::{
    ActivityListPage, ActivityListRequest, CurationMutationResult, ExportPreview, ExportProgress,
    ExportResult, FixtureJourneyResult, HealthReport, LegacyImportReport, OperationsPage,
    OperationsRequest, RepairReport, SessionCurationRequest, SessionDetail, SessionDetailRequest,
    SessionListPage, SessionListRequest, SourcePreference, SyncProgress, SyncRunResult,
    SyncRunSummary,
};
use tauri::{AppHandle, Emitter};

use crate::error::HostError;
use crate::host::{
    run_add_session_tag, run_export_cancel, run_fixture_journey, run_health, run_import_legacy,
    run_list_activity, run_list_operations, run_list_sessions, run_list_sources,
    run_prepare_export_cancellation, run_preview_export, run_publish_export_cancellable,
    run_remove_session_tag, run_repair, run_session_detail, run_set_source_preference,
    run_sync_cancel, run_sync_start, run_sync_status, run_toggle_session_label,
    validate_export_request, validate_fixture_journey_request, validate_home_request,
    validate_legacy_import_request, validate_session_curation_request,
    validate_source_preference_request, validate_sync_id_request, validate_sync_start_request,
};
use crate::{EXPORT_PROGRESS_EVENT, FIXTURE_JOURNEY_PROGRESS_EVENT, SYNC_PROGRESS_EVENT};

/**
 * Tauri command: validate inputs, run the Fixture journey off the UI thread,
 * emit typed progress, and return source/sync/session/health results.
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
 * Tauri command: import a legacy Electron home into a native Distill home.
 */
#[tauri::command]
pub async fn import_legacy_command(
    home: String,
    source_home: String,
) -> Result<LegacyImportReport, HostError> {
    let request = validate_legacy_import_request(&home, &source_home)?;
    tauri::async_runtime::spawn_blocking(move || run_import_legacy(&request))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: explicit Library repair after renderer confirmation.
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

/**
 * Tauri command: list Source preferences.
 */
#[tauri::command]
pub async fn list_sources_command(home: String) -> Result<Vec<SourcePreference>, HostError> {
    let request = validate_home_request(&home)?;
    tauri::async_runtime::spawn_blocking(move || run_list_sources(&request))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: upsert Source preference.
 */
#[tauri::command]
pub async fn set_source_preference_command(
    home: String,
    kind: String,
    enabled: bool,
    configured_root: Option<String>,
) -> Result<SourcePreference, HostError> {
    let request =
        validate_source_preference_request(&home, &kind, enabled, configured_root.as_deref())?;
    tauri::async_runtime::spawn_blocking(move || run_set_source_preference(&request))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: start Sync Run off the UI thread with typed progress events.
 */
#[tauri::command]
pub async fn sync_start_command(
    app: AppHandle,
    home: String,
    source_kinds: Option<Vec<String>>,
) -> Result<SyncRunResult, HostError> {
    let request = validate_sync_start_request(&home, source_kinds.unwrap_or_default())?;
    tauri::async_runtime::spawn_blocking(move || {
        run_sync_start(&request, |progress: SyncProgress| {
            let _ = app.emit(SYNC_PROGRESS_EVENT, progress);
        })
    })
    .await
    .map_err(|err| HostError {
        code: "join".to_string(),
        message: err.to_string(),
    })?
}

/**
 * Tauri command: Sync Run status.
 */
#[tauri::command]
pub async fn sync_status_command(
    home: String,
    sync_run_id: Option<i64>,
) -> Result<SyncRunSummary, HostError> {
    let request = validate_home_request(&home)?;
    tauri::async_runtime::spawn_blocking(move || run_sync_status(&request, sync_run_id))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: request Sync Run cancellation.
 */
#[tauri::command]
pub async fn sync_cancel_command(
    home: String,
    sync_run_id: i64,
) -> Result<SyncRunSummary, HostError> {
    let request = validate_sync_id_request(&home, sync_run_id)?;
    tauri::async_runtime::spawn_blocking(move || run_sync_cancel(&request))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: list/search current Session Projections off the UI thread.
 */
#[tauri::command]
pub async fn sessions_list_command(
    home: String,
    request: SessionListRequest,
) -> Result<SessionListPage, HostError> {
    let home_request = validate_home_request(&home)?;
    tauri::async_runtime::spawn_blocking(move || run_list_sessions(&home_request, request))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: load bounded Session detail off the UI thread.
 */
#[tauri::command]
pub async fn session_detail_command(
    home: String,
    request: SessionDetailRequest,
) -> Result<Option<SessionDetail>, HostError> {
    let home_request = validate_home_request(&home)?;
    tauri::async_runtime::spawn_blocking(move || run_session_detail(&home_request, request))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: add a manual session tag off the UI thread.
 */
#[tauri::command]
pub async fn add_session_tag_command(
    home: String,
    request: SessionCurationRequest,
) -> Result<CurationMutationResult, HostError> {
    let (home_request, curation) = validate_session_curation_request(&home, request)?;
    tauri::async_runtime::spawn_blocking(move || run_add_session_tag(&home_request, curation))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: remove a manual session tag off the UI thread.
 */
#[tauri::command]
pub async fn remove_session_tag_command(
    home: String,
    request: SessionCurationRequest,
) -> Result<CurationMutationResult, HostError> {
    let (home_request, curation) = validate_session_curation_request(&home, request)?;
    tauri::async_runtime::spawn_blocking(move || run_remove_session_tag(&home_request, curation))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: toggle a catalog session label off the UI thread.
 */
#[tauri::command]
pub async fn toggle_session_label_command(
    home: String,
    request: SessionCurationRequest,
) -> Result<CurationMutationResult, HostError> {
    let (home_request, curation) = validate_session_curation_request(&home, request)?;
    tauri::async_runtime::spawn_blocking(move || run_toggle_session_label(&home_request, curation))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: preview export eligibility off the UI thread.
 */
#[tauri::command]
pub async fn export_preview_command(
    home: String,
    dataset: String,
) -> Result<ExportPreview, HostError> {
    let request = validate_export_request(&home, &dataset)?;
    tauri::async_runtime::spawn_blocking(move || run_preview_export(&request))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: publish a recoverable export off the UI thread with typed progress.
 */
#[tauri::command]
pub async fn export_publish_command(
    app: AppHandle,
    home: String,
    dataset: String,
) -> Result<ExportResult, HostError> {
    let request = validate_export_request(&home, &dataset)?;
    if !run_prepare_export_cancellation(&request)? {
        return Err(HostError {
            code: "export_already_running".to_string(),
            message: "an export for this home and dataset is already running".to_string(),
        });
    }
    tauri::async_runtime::spawn_blocking(move || {
        run_publish_export_cancellable(&request, |progress: ExportProgress| {
            let _ = app.emit(EXPORT_PROGRESS_EVENT, progress);
        })
    })
    .await
    .map_err(|err| HostError {
        code: "join".to_string(),
        message: err.to_string(),
    })?
}

/**
 * Tauri command: request cancellation of an active export publication.
 */
#[tauri::command]
pub async fn export_cancel_command(home: String, dataset: String) -> Result<bool, HostError> {
    let request = validate_export_request(&home, &dataset)?;
    tauri::async_runtime::spawn_blocking(move || run_export_cancel(&request))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: list append-only Activity Events off the UI thread.
 */
#[tauri::command]
pub async fn activity_list_command(
    home: String,
    request: ActivityListRequest,
) -> Result<ActivityListPage, HostError> {
    let home_request = validate_home_request(&home)?;
    tauri::async_runtime::spawn_blocking(move || run_list_activity(&home_request, request))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}

/**
 * Tauri command: list operational Sync/export diagnostics off the UI thread.
 */
#[tauri::command]
pub async fn operations_list_command(
    home: String,
    request: OperationsRequest,
) -> Result<OperationsPage, HostError> {
    let home_request = validate_home_request(&home)?;
    tauri::async_runtime::spawn_blocking(move || run_list_operations(&home_request, request))
        .await
        .map_err(|err| HostError {
            code: "join".to_string(),
            message: err.to_string(),
        })?
}
