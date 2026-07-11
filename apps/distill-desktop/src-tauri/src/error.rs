//! Typed host errors returned across the Tauri IPC boundary.

use distill_library::LibraryError;
use serde::Serialize;
use thiserror::Error;

/// Host-boundary failure with a stable machine-readable code.
#[derive(Debug, Error, Serialize)]
#[error("{code}: {message}")]
pub struct HostError {
    /// Stable error class for the renderer.
    pub code: String,
    /// Human-readable detail safe for the UI.
    pub message: String,
}

impl HostError {
    /**
     * Build a validation failure for empty or missing caller inputs.
     *
     * Parameters:
     * - `message`: validation detail.
     */
    pub fn validation(message: impl Into<String>) -> Self {
        Self {
            code: "validation".to_string(),
            message: message.into(),
        }
    }

    /**
     * Translate a Library error into a typed host error.
     *
     * Parameters:
     * - `err`: Library failure from the Fixture journey.
     */
    pub fn from_library(err: LibraryError) -> Self {
        Self {
            code: library_error_code(&err).to_string(),
            message: err.to_string(),
        }
    }
}

/**
 * Map Library errors to stable host error codes.
 */
fn library_error_code(err: &LibraryError) -> &'static str {
    match err {
        LibraryError::PathOutsideConfiguredRoot { .. } => "path_outside_configured_root",
        LibraryError::CaptureTooLarge { .. } => "capture_too_large",
        LibraryError::MigrationChecksumMismatch { .. } => "migration_checksum_mismatch",
        LibraryError::SourceAdapter(_) => "source_adapter",
        LibraryError::Io(_) => "io",
        LibraryError::Sqlite(_) => "sqlite",
        LibraryError::Json(_) => "json",
        LibraryError::ContentIntegrity { .. } => "content_integrity",
        LibraryError::StagedContentIntegrity { .. } => "staged_content_integrity",
        LibraryError::NotFound(_) => "not_found",
        LibraryError::InvalidArgument(_) => "invalid_argument",
    }
}
