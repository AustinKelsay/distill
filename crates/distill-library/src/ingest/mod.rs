//! Ingest pipeline: SourceAdapter -> verified Capture -> Attempt -> Projection.

mod attempt;
mod capture;
mod retry;

pub use retry::renormalize_capture;

use rusqlite::Connection;
use serde_json::json;

use crate::adapter::SourceAdapter;
use crate::error::{LibraryError, LibraryResult};
use crate::privacy::reject_oversized_candidate_file;
use crate::storage::{store_capture_bytes, DistillPaths};
use crate::types::{IngestReport, SessionIdentity};

use attempt::{
    fail_attempt, insert_attempt, publish_projection,
    refresh_session_counters_after_failed_attempt, PARSE_FAILURE_MESSAGE,
    PROJECTION_FAILURE_MESSAGE,
};
use capture::{
    emit_activity, enforce_configured_root, find_duplicate, insert_capture, upsert_source,
    verify_snapshot_metadata,
};

/// Optional Sync Run checkpoint hooks for cancellation and progress.
///
/// Safe cancellation is observed only before each Capture Candidate. A request
/// mid-candidate finishes the current snapshot/accept/Attempt/projection work,
/// then stops before the next candidate. After `on_candidate_started` returns,
/// `assert_owner_before_work` must succeed before candidate work begins; that
/// assertion does not honor a newly requested cancellation so a cancel at
/// CandidateStarted still finishes that candidate.
pub struct IngestCheckpoints<'a> {
    /// Return true to cancel before the next candidate.
    pub should_cancel: &'a mut dyn FnMut() -> LibraryResult<bool>,
    /// Observe logical candidate identity before work begins.
    pub on_candidate_started: &'a mut dyn FnMut(&str),
    /// Assert Sync Run lease ownership after progress callback, before work.
    pub assert_owner_before_work: &'a mut dyn FnMut() -> LibraryResult<()>,
    /// Observe logical candidate identity and outcome after work finishes.
    pub on_candidate_finished: &'a mut dyn FnMut(&str, &str),
    /// When true, ordinary per-candidate snapshot/policy failures append
    /// `capture_failed` and continue later candidates (Sync). When false,
    /// those errors abort ingest (direct `ingest_fixture` / journey).
    pub continue_on_candidate_error: bool,
}

/**
 * Run a SourceAdapter through the production ingest seam into the Library store.
 */
pub fn ingest_adapter(
    conn: &mut Connection,
    paths: &DistillPaths,
    adapter: &dyn SourceAdapter,
    max_capture_bytes: u64,
) -> LibraryResult<IngestReport> {
    let mut should_cancel = || Ok(false);
    let mut on_started = |_id: &str| {};
    let mut assert_owner = || Ok(());
    let mut on_finished = |_id: &str, _outcome: &str| {};
    ingest_adapter_with_checkpoints(
        conn,
        paths,
        adapter,
        max_capture_bytes,
        IngestCheckpoints {
            should_cancel: &mut should_cancel,
            on_candidate_started: &mut on_started,
            assert_owner_before_work: &mut assert_owner,
            on_candidate_finished: &mut on_finished,
            continue_on_candidate_error: false,
        },
    )
}

/**
 * Ingest through the same production policy with Sync Run checkpoint hooks.
 *
 * Parameters:
 * - `conn`: Library SQLite connection.
 * - `paths`: Distill home paths.
 * - `adapter`: SourceAdapter implementation.
 * - `max_capture_bytes`: Capture acceptance limit.
 * - `checkpoints`: cancel/progress hooks observed between candidates only.
 */
pub fn ingest_adapter_with_checkpoints(
    conn: &mut Connection,
    paths: &DistillPaths,
    adapter: &dyn SourceAdapter,
    max_capture_bytes: u64,
    checkpoints: IngestCheckpoints<'_>,
) -> LibraryResult<IngestReport> {
    let source = adapter.detect()?;
    let source_id = upsert_source(conn, &source.kind, &source.display_name, &source.data_root)?;
    let candidates = adapter.discover(&source)?;
    let mut report = IngestReport::default();

    for candidate in candidates {
        if (checkpoints.should_cancel)()? {
            break;
        }
        let candidate_id = candidate.source_path.as_str();
        (checkpoints.on_candidate_started)(candidate_id);
        (checkpoints.assert_owner_before_work)()?;

        let outcome = match ingest_one_candidate(
            conn,
            paths,
            adapter,
            source_id,
            &source,
            &candidate,
            max_capture_bytes,
            &mut report,
        ) {
            Ok(outcome) => outcome,
            Err(err)
                if checkpoints.continue_on_candidate_error
                    && is_continuable_candidate_error(&err) =>
            {
                emit_snapshot_failure_activity(conn, &source.kind, candidate_id, &err)?;
                report.failed_attempts += 1;
                "failed"
            }
            Err(err) => return Err(err),
        };
        (checkpoints.on_candidate_finished)(candidate_id, outcome);
    }

    Ok(report)
}

/**
 * Snapshot/accept/Attempt/projection for one candidate. Never observes cancel mid-transaction.
 */
#[allow(clippy::too_many_arguments)]
fn ingest_one_candidate(
    conn: &mut Connection,
    paths: &DistillPaths,
    adapter: &dyn SourceAdapter,
    source_id: i64,
    source: &crate::adapter::DiscoveredSource,
    candidate: &crate::adapter::CaptureCandidate,
    max_capture_bytes: u64,
    report: &mut IngestReport,
) -> LibraryResult<&'static str> {
    enforce_configured_root(&source.data_root, candidate)?;
    reject_oversized_candidate_file(candidate, max_capture_bytes)?;
    let snapshot = adapter.snapshot(candidate)?;
    verify_snapshot_metadata(&snapshot)?;
    if find_duplicate(
        conn,
        candidate.source_kind.as_str(),
        &candidate.source_path,
        &snapshot.sha256,
    )?
    .is_some()
    {
        report.skipped_duplicates += 1;
        return Ok("skipped_duplicate");
    }

    let content = store_capture_bytes(
        &paths.home,
        &paths.staging,
        &paths.blobs,
        &snapshot.bytes,
        &snapshot.media_type,
        max_capture_bytes,
    )?;

    let capture_id = {
        let tx = conn.transaction()?;
        let capture_id = insert_capture(
            &tx,
            source_id,
            candidate,
            &content,
            snapshot.source_modified_at.as_deref(),
        )?;
        #[cfg(feature = "test-faults")]
        crate::faults::check(crate::faults::FaultPoint::AfterCaptureInsertBeforeActivity)?;
        emit_activity(
            &tx,
            "capture_recorded",
            Some(candidate.source_kind.as_str()),
            None,
            Some(capture_id),
            None,
            json!({ "source_path": candidate.source_path, "sha256": snapshot.sha256 }),
        )?;
        tx.commit()?;
        capture_id
    };
    #[cfg(feature = "test-faults")]
    crate::faults::check(crate::faults::FaultPoint::AfterCaptureRecordedBeforeAttempt)?;
    report.accepted_captures += 1;
    report.capture_ids.push(capture_id);

    match adapter.parse(candidate, &snapshot) {
        Ok(parsed) => {
            let attempt_id = insert_attempt(
                conn,
                capture_id,
                &source.parser.id,
                &source.parser.version,
                "pending",
                None,
                None,
            )?;
            #[cfg(feature = "test-faults")]
            crate::faults::check(crate::faults::FaultPoint::AfterPendingAttemptBeforePublish)?;
            match publish_projection(conn, capture_id, attempt_id, candidate, &parsed) {
                Ok(()) => {
                    report.successful_attempts += 1;
                    push_session_identity(
                        report,
                        candidate.source_kind.as_str(),
                        &parsed.external_session_id,
                    );
                    Ok("accepted")
                }
                Err(err) => {
                    #[cfg(feature = "test-faults")]
                    if matches!(&err, crate::error::LibraryError::InjectedTestFault { .. }) {
                        return Err(err);
                    }
                    #[cfg(not(feature = "test-faults"))]
                    let _ = err;
                    fail_attempt(
                        conn,
                        attempt_id,
                        "projection_failed",
                        PROJECTION_FAILURE_MESSAGE,
                    )?;
                    refresh_session_counters_after_failed_attempt(
                        conn,
                        candidate.source_kind.as_str(),
                        candidate
                            .external_session_id
                            .as_deref()
                            .or(Some(parsed.external_session_id.as_str())),
                    )?;
                    report.failed_attempts += 1;
                    Ok("failed")
                }
            }
        }
        Err(_err) => {
            let _attempt_id = insert_attempt(
                conn,
                capture_id,
                &source.parser.id,
                &source.parser.version,
                "failed",
                Some("parse_failed"),
                Some(PARSE_FAILURE_MESSAGE),
            )?;
            refresh_session_counters_after_failed_attempt(
                conn,
                candidate.source_kind.as_str(),
                candidate.external_session_id.as_deref(),
            )?;
            report.failed_attempts += 1;
            Ok("failed")
        }
    }
}

/**
 * Record a distinct Session Identity on the ingest report.
 */
fn push_session_identity(report: &mut IngestReport, source_kind: &str, external_session_id: &str) {
    let identity = SessionIdentity {
        source_kind: source_kind.to_string(),
        external_session_id: external_session_id.to_string(),
    };
    if !report.session_identities.contains(&identity) {
        report.session_identities.push(identity);
    }
}

/**
 * Ordinary per-candidate failures that must not abort later candidates.
 */
fn is_continuable_candidate_error(err: &LibraryError) -> bool {
    matches!(
        err,
        LibraryError::SourceAdapter(_)
            | LibraryError::PathOutsideConfiguredRoot { .. }
            | LibraryError::CaptureTooLarge { .. }
            | LibraryError::StagedContentIntegrity { .. }
            | LibraryError::InvalidArgument(_)
    )
}

/**
 * Append canonical `capture_failed` for a snapshot/pre-accept candidate failure.
 *
 * No Capture row is created. Existing projections remain unchanged.
 */
fn emit_snapshot_failure_activity(
    conn: &Connection,
    source_kind: &crate::adapter::SourceKind,
    candidate_id: &str,
    err: &LibraryError,
) -> LibraryResult<()> {
    emit_activity(
        conn,
        "capture_failed",
        Some(source_kind.as_str()),
        None,
        None,
        None,
        json!({
            "reason": "snapshot_failed",
            "candidate_id": candidate_id,
            "error_class": err.code(),
        }),
    )
}
