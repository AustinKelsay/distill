//! Typed host errors returned across the Tauri IPC boundary.

use distill_library::{safe_caller_message, LibraryError};
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
     * Translate a Library error into a typed host error with redacted detail.
     *
     * Parameters:
     * - `err`: Library failure from a host command.
     */
    pub fn from_library(err: LibraryError) -> Self {
        Self {
            code: err.code().to_string(),
            message: safe_caller_message(&err),
        }
    }
}
