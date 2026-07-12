//! Ingest pipeline: SourceAdapter -> verified Capture -> Attempt -> Projection.

mod attempt;
mod capture;
mod retry;

pub use retry::renormalize_capture;

use rusqlite::Connection;
use serde_json::json;

use crate::adapter::SourceAdapter;
use crate::error::LibraryResult;
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

/**
 * Run a SourceAdapter through the production ingest seam into the Library store.
 */
pub fn ingest_adapter(
    conn: &mut Connection,
    paths: &DistillPaths,
    adapter: &dyn SourceAdapter,
    max_capture_bytes: u64,
) -> LibraryResult<IngestReport> {
    let source = adapter.detect()?;
    let source_id = upsert_source(conn, &source.kind, &source.display_name, &source.data_root)?;
    let candidates = adapter.discover(&source)?;
    let mut report = IngestReport::default();

    for candidate in candidates {
        enforce_configured_root(&source.data_root, &candidate)?;
        let snapshot = adapter.snapshot(&candidate)?;
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
            // Exact duplicates are inert: no Capture, Attempt, projection, FTS, or Activity.
            continue;
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
                &candidate,
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

        match adapter.parse(&candidate, &snapshot) {
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
                match publish_projection(conn, capture_id, attempt_id, &candidate, &parsed) {
                    Ok(()) => {
                        report.successful_attempts += 1;
                        push_session_identity(
                            &mut report,
                            candidate.source_kind.as_str(),
                            &parsed.external_session_id,
                        );
                    }
                    Err(err) => {
                        // Injected faults simulate process death: leave the pending Attempt
                        // and last-good projection untouched instead of recording an ordinary
                        // projection_failed outcome. Present only when `test-faults` is enabled.
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
            }
        }
    }

    Ok(report)
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
