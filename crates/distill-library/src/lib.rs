//! Distill Library public crate.
//!
//! Callers open a Distill home through [`Library`] and drive ingest, query, replay,
//! health, repair, Source preferences, and Sync Runs through typed product methods.
//! SQLite, content-addressed files, and SourceAdapter internals stay private.

#![deny(missing_docs)]

mod adapter;
mod curation;
mod error;
mod export;
#[cfg(feature = "test-faults")]
pub mod faults;
mod health;
mod ingest;
mod library;
mod ops;
mod query;
mod storage;
mod types;

pub use error::{LibraryError, LibraryResult};
pub use library::Library;
#[cfg(feature = "test-leases")]
pub use ops::test_leases;
pub use ops::SYNC_LEASE_STALE_AFTER;

/// Test-only access to the provider-process policy contracts.
#[cfg(feature = "test-leases")]
pub mod test_support {
    pub use crate::ops::{
        enforce_output_bounds_for_test, run_bounded_command, BoundedProcessOutput,
        ProviderProcessLimits,
    };
}
pub use types::{
    derive_workflow_state, matches_workflow_lane, ActivityEventSummary, AttemptSummary,
    CurationMutationResult, ExportDataset, ExportOmission, ExportOmissionReason, ExportPreview,
    ExportProgress, ExportProgressControl, ExportResult, ExportStatus, FixtureJourneyPhase,
    FixtureJourneyResult, HealthIssue, HealthReport, IngestReport, OpenReconciliation,
    ProjectedArtifact, ProjectedMessage, RenormalizeReport, RepairAction, RepairOptions,
    RepairReport, SearchHit, SessionCurationRequest, SessionDetail, SessionDetailRequest,
    SessionIdentity, SessionLabel, SessionListItem, SessionListPage, SessionListRequest,
    SessionSummary, SessionTag, SourceDetectRequest, SourceDetectResult, SourcePreference,
    SourceSummary, SyncProgress, SyncRequest, SyncRunResult, SyncRunSummary, SyncSourceOutcome,
    WorkflowLane, WorkflowState, EXPORT_FORMAT_ID, INLINE_CONTENT_THRESHOLD_BYTES, MAX_PAGE_SIZE,
};
