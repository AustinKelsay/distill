//! Test-only ingest fault injection.
//!
//! Enabled only with the `test-faults` Cargo feature. Production builds never arm
//! faults and never expose this module.

use std::cell::Cell;

use crate::error::{LibraryError, LibraryResult};

/// Named ingest boundaries that can be interrupted under `test-faults`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultPoint {
    /// After staging a blob partial, before atomic CAS rename.
    AfterStageWriteBeforeRename,
    /// After CAS rename, before Capture database acceptance.
    AfterBlobRenameBeforeCaptureAccept,
    /// After Capture row insert, before `capture_recorded` Activity in the same tx.
    AfterCaptureInsertBeforeActivity,
    /// After Capture + `capture_recorded` commit, before Attempt insert.
    AfterCaptureRecordedBeforeAttempt,
    /// After pending Attempt insert, before projection publication.
    AfterPendingAttemptBeforePublish,
    /// Inside projection tx after Facts/projection rows, before FTS inserts.
    DuringPublishAfterFactsBeforeFts,
    /// Inside projection tx after FTS, before Attempt success / Activity.
    DuringPublishAfterFtsBeforeAttemptSuccess,
    /// Inside projection tx after `projection_replaced`, before commit.
    DuringPublishAfterActivityBeforeCommit,
    /// After export temporary JSONL write/flush, before moving the row to `committed`.
    AfterExportTempWrite,
    /// After export row reaches `committed`, before same-volume final rename.
    AfterExportCommittedBeforeRename,
    /// After export final rename, before `published` + `export_written` finalization.
    AfterExportRenameBeforeFinalization,
    /// Inside final export bookkeeping, before the publish transaction commits.
    DuringExportFinalizationBeforeCommit,
}

thread_local! {
    static ARMED: Cell<Option<FaultPoint>> = const { Cell::new(None) };
}

/**
 * Arm a one-shot fault at `point` for the current thread.
 *
 * The next matching [`check`] consumes the arm and returns an error.
 */
pub fn arm(point: FaultPoint) {
    ARMED.with(|cell| cell.set(Some(point)));
}

/// Clear any armed fault for the current thread.
pub fn clear() {
    ARMED.with(|cell| cell.set(None));
}

/**
 * Return an error when `point` is armed, otherwise continue.
 *
 * Parameters:
 * - `point`: boundary being crossed in production ingest code.
 */
pub fn check(point: FaultPoint) -> LibraryResult<()> {
    ARMED.with(|cell| {
        if cell.get() == Some(point) {
            cell.set(None);
            Err(LibraryError::InjectedTestFault { point })
        } else {
            Ok(())
        }
    })
}
