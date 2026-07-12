//! Typed Library errors surfaced at the public seam.

use std::path::PathBuf;

use thiserror::Error;

use crate::adapter::SourceStageError;

/// Result alias for Library operations.
pub type LibraryResult<T> = Result<T, LibraryError>;

/// Errors returned by the public Library interface.
#[derive(Debug, Error)]
pub enum LibraryError {
    /// A candidate path escaped the configured Fixture or Source root.
    #[error("path outside configured root: {path}")]
    PathOutsideConfiguredRoot {
        /// Escaping path.
        path: PathBuf,
        /// Configured root that should contain the path.
        root: PathBuf,
    },

    /// A capture exceeded the configured size limit before acceptance.
    #[error("capture exceeds size limit: {byte_size} > {limit}")]
    CaptureTooLarge {
        /// Observed byte size.
        byte_size: u64,
        /// Configured maximum.
        limit: u64,
    },

    /// Schema migration checksum failed verification.
    #[error("migration checksum mismatch for version {version}")]
    MigrationChecksumMismatch {
        /// Migration version that failed verification.
        version: i64,
    },

    /// SourceAdapter stage failure bubbled through ingest.
    #[error("source adapter error: {0}")]
    SourceAdapter(#[from] SourceStageError),

    /// Filesystem IO failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// SQLite failure.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// JSON serialization or parse failure.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// Capture content is missing or corrupt during replay or health.
    #[error("content integrity failure for capture {capture_id}: {detail}")]
    ContentIntegrity {
        /// Capture row id.
        capture_id: i64,
        /// Human-readable detail.
        detail: String,
    },

    /// Content failed verification before any Capture was accepted.
    #[error("staged content integrity failure: {detail}")]
    StagedContentIntegrity {
        /// Human-readable integrity detail.
        detail: String,
    },

    /// Requested entity was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// Generic invalid argument.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// Test-only injected fault at a named ingest boundary.
    #[cfg(feature = "test-faults")]
    #[error("injected test fault: {point:?}")]
    InjectedTestFault {
        /// Boundary that was armed when the fault fired.
        point: crate::faults::FaultPoint,
    },
}

impl LibraryError {
    /// Stable machine-readable error code shared by thin callers.
    pub fn code(&self) -> &'static str {
        match self {
            Self::PathOutsideConfiguredRoot { .. } => "path_outside_configured_root",
            Self::CaptureTooLarge { .. } => "capture_too_large",
            Self::MigrationChecksumMismatch { .. } => "migration_checksum_mismatch",
            Self::SourceAdapter(_) => "source_adapter",
            Self::Io(_) => "io",
            Self::Sqlite(_) => "sqlite",
            Self::Json(_) => "json",
            Self::ContentIntegrity { .. } => "content_integrity",
            Self::StagedContentIntegrity { .. } => "staged_content_integrity",
            Self::NotFound(_) => "not_found",
            Self::InvalidArgument(_) => "invalid_argument",
            #[cfg(feature = "test-faults")]
            Self::InjectedTestFault { .. } => "injected_test_fault",
        }
    }
}
