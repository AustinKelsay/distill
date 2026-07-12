//! Distill desktop host library: Tauri commands over the public Library seam.

#![deny(missing_docs)]

mod commands;
mod error;
mod host;

pub use error::HostError;
pub use host::{
    run_add_session_tag as execute_add_session_tag, run_export_cancel as execute_export_cancel,
    run_fixture_journey as execute_fixture_journey, run_health as execute_health,
    run_list_activity as execute_list_activity, run_list_operations as execute_list_operations,
    run_list_sessions as execute_list_sessions, run_list_sources as execute_list_sources,
    run_prepare_export_cancellation as execute_prepare_export_cancellation,
    run_preview_export as execute_preview_export, run_publish_export as execute_publish_export,
    run_publish_export_cancellable as execute_publish_export_cancellable,
    run_publish_export_with_control as execute_publish_export_with_control,
    run_remove_session_tag as execute_remove_session_tag, run_repair as execute_repair,
    run_session_detail as execute_session_detail,
    run_set_source_preference as execute_set_source_preference,
    run_sync_cancel as execute_sync_cancel, run_sync_start as execute_sync_start,
    run_sync_status as execute_sync_status,
    run_toggle_session_label as execute_toggle_session_label, validate_export_request,
    validate_fixture_journey_request, validate_home_request, validate_session_curation_request,
    validate_source_preference_request, validate_sync_id_request, validate_sync_start_request,
    ExportRequest, FixtureJourneyRequest, HomeRequest, SourcePreferenceRequest, SyncIdRequest,
    SyncStartRequest,
};

use distill_library::{ExportProgress, FixtureJourneyPhase, SyncProgress};

/// Event name for typed Fixture journey progress.
pub const FIXTURE_JOURNEY_PROGRESS_EVENT: &str = "fixture-journey-progress";
/// Event name for typed Sync Run progress.
pub const SYNC_PROGRESS_EVENT: &str = "sync-progress";
/// Event name for typed export publication progress.
pub const EXPORT_PROGRESS_EVENT: &str = "export-progress";

/**
 * Build and run the Distill Tauri application.
 */
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::run_fixture_journey_command,
            commands::health_command,
            commands::repair_command,
            commands::list_sources_command,
            commands::set_source_preference_command,
            commands::sync_start_command,
            commands::sync_status_command,
            commands::sync_cancel_command,
            commands::sessions_list_command,
            commands::session_detail_command,
            commands::add_session_tag_command,
            commands::remove_session_tag_command,
            commands::toggle_session_label_command,
            commands::export_preview_command,
            commands::export_publish_command,
            commands::export_cancel_command,
            commands::activity_list_command,
            commands::operations_list_command
        ])
        .run(tauri::generate_context!())
        .expect("error while running Distill desktop");
}

/// Re-export progress phase for host tests and documentation.
pub type ProgressPhase = FixtureJourneyPhase;
/// Re-export Sync progress for host tests.
pub type HostSyncProgress = SyncProgress;
/// Re-export export progress for host tests.
pub type HostExportProgress = ExportProgress;
