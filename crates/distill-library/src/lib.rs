//! Distill Library public crate.
//!
//! Callers open a Distill home through [`Library`] and drive ingest, query, replay,
//! health, and repair through typed product methods. SQLite, content-addressed files,
//! and SourceAdapter internals stay private to this crate.

#![deny(missing_docs)]

mod adapter;
mod error;
#[cfg(feature = "test-faults")]
pub mod faults;
mod health;
mod ingest;
mod library;
mod query;
mod storage;
mod types;

pub use error::{LibraryError, LibraryResult};
pub use library::Library;
pub use types::{
    ActivityEventSummary, AttemptSummary, FixtureJourneyPhase, FixtureJourneyResult, HealthIssue,
    HealthReport, IngestReport, OpenReconciliation, ProjectedArtifact, ProjectedMessage,
    RenormalizeReport, RepairAction, RepairOptions, RepairReport, SearchHit, SessionDetail,
    SessionIdentity, SessionSummary, SourceSummary, INLINE_CONTENT_THRESHOLD_BYTES, MAX_PAGE_SIZE,
};
