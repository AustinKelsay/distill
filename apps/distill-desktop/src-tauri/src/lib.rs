//! Distill desktop host library: Tauri commands over the public Library seam.

#![deny(missing_docs)]

mod commands;
mod error;
mod host;

pub use error::HostError;
pub use host::{
    run_add_session_tag as execute_add_session_tag,
    run_capture_attempts as execute_capture_attempts, run_detect_sources as execute_detect_sources,
    run_export_cancel as execute_export_cancel, run_fixture_journey as execute_fixture_journey,
    run_health as execute_health, run_import_legacy as execute_import_legacy,
    run_list_activity as execute_list_activity, run_list_operations as execute_list_operations,
    run_list_sessions as execute_list_sessions, run_list_sources as execute_list_sources,
    run_prepare_export_cancellation as execute_prepare_export_cancellation,
    run_preview_export as execute_preview_export, run_publish_export as execute_publish_export,
    run_publish_export_cancellable as execute_publish_export_cancellable,
    run_publish_export_with_control as execute_publish_export_with_control,
    run_remove_session_tag as execute_remove_session_tag,
    run_renormalize_capture as execute_renormalize_capture, run_repair as execute_repair,
    run_session_detail as execute_session_detail,
    run_set_source_preference as execute_set_source_preference,
    run_sync_cancel as execute_sync_cancel, run_sync_start as execute_sync_start,
    run_sync_status as execute_sync_status,
    run_toggle_session_label as execute_toggle_session_label, validate_capture_id_request,
    validate_export_request, validate_fixture_journey_request, validate_home_request,
    validate_legacy_import_request, validate_session_curation_request,
    validate_source_detect_request, validate_source_preference_request, validate_sync_id_request,
    validate_sync_start_request, CaptureIdRequest, ExportRequest, FixtureJourneyRequest,
    HomeRequest, LegacyImportRequest, SourceDetectBatchRequest, SourcePreferenceRequest,
    SyncIdRequest, SyncStartRequest,
};

use distill_library::{ExportProgress, FixtureJourneyPhase, SyncProgress};
use tauri::Manager;

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
    let smoke_dom_activation = std::env::var_os("DISTILL_SMOKE_DOM_ACTIVATE").is_some();
    tauri::Builder::default()
        .setup(move |app| {
            if smoke_dom_activation {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    // Keep the hook alive through the bounded AT-SPI focus and
                    // keyboard probes that precede migration in the smoke.
                    for _ in 0..600 {
                        let next_handle = handle.clone();
                        let _ = handle.run_on_main_thread(move || {
                            if let Some(window) = next_handle.get_webview_window("main") {
                                let _ = window.eval(
                                    r#"(() => {
                                      const panel = document.querySelector('[data-testid="migration-panel"]');
                                      const button = document.querySelector('[data-testid="migration-run"]');
                                      const status = document.querySelector('[data-testid="migration-status"]');
                                      if (!panel || !button || !status || !button.getAttribute('aria-label')?.includes('(ready)') || button.disabled) return;
                                      if (!status.textContent?.includes('Migration status: idle')) return;
                                      if (typeof panel.requestSubmit === 'function') panel.requestSubmit(button);
                                      else button.click();
                                    })();"#,
                                );
                            }
                        });
                        std::thread::sleep(std::time::Duration::from_millis(250));
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::run_fixture_journey_command,
            commands::health_command,
            commands::import_legacy_command,
            commands::repair_command,
            commands::list_sources_command,
            commands::detect_sources_command,
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
            commands::operations_list_command,
            commands::capture_attempts_command,
            commands::renormalize_capture_command
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
