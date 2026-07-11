//! Distill desktop host library: Tauri commands over the public Library seam.

#![deny(missing_docs)]

mod commands;
mod error;
mod host;

pub use error::HostError;
pub use host::{
    run_fixture_journey as execute_fixture_journey, validate_fixture_journey_request,
    FixtureJourneyRequest,
};

use distill_library::FixtureJourneyPhase;

/// Event name for typed Fixture journey progress.
pub const FIXTURE_JOURNEY_PROGRESS_EVENT: &str = "fixture-journey-progress";

/**
 * Build and run the Distill Tauri application.
 */
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::run_fixture_journey_command
        ])
        .run(tauri::generate_context!())
        .expect("error while running Distill desktop");
}

/// Re-export progress phase for host tests and documentation.
pub type ProgressPhase = FixtureJourneyPhase;
