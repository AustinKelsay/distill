//! Per-Source Sync Run execution through the production ingest checkpoint seam.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::adapter::{
    CodexAdapter, FixtureAdapter, ParserIdentity, SourceAdapter, SourceKind, FIXTURE_PARSER_ID,
};
use crate::error::{LibraryError, LibraryResult};
use crate::ingest::{self, IngestCheckpoints};
use crate::ops::sync::cancel_requested;
use crate::ops::sync_lease::{assert_lease_owned, refresh_lease};
use crate::storage::{open_connection, DistillPaths};
use crate::types::{IngestReport, SyncProgress, SyncSourceOutcome};

/**
 * Execute one Source inside an active Sync Run.
 *
 * Ordinary detect/discover/snapshot/config failures become failed Source outcomes
 * with stable redacted diagnostics so sibling Sources can continue.
 */
#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_one_source<F>(
    conn: &mut Connection,
    paths: &DistillPaths,
    sync_run_id: i64,
    owner_id: &str,
    source_kind: SourceKind,
    fixture_parser: &ParserIdentity,
    max_capture_bytes: u64,
    on_progress: &mut F,
    aggregate: &mut IngestReport,
) -> LibraryResult<SyncSourceOutcome>
where
    F: FnMut(SyncProgress),
{
    match source_kind {
        SourceKind::Fixture => {
            let root = match load_configured_root(conn, source_kind) {
                Ok(Some(root)) => root,
                Ok(None) => {
                    return Ok(failed_outcome(
                        source_kind,
                        "configured_root_required",
                        "source sync requires a configured root",
                    ));
                }
                Err(err) => {
                    return Ok(failed_outcome(
                        source_kind,
                        err.code(),
                        "configured root is invalid",
                    ));
                }
            };
            let adapter = FixtureAdapter::with_parser(
                root,
                ParserIdentity {
                    id: FIXTURE_PARSER_ID.to_string(),
                    version: fixture_parser.version.clone(),
                },
            );
            sync_adapter_source(
                conn,
                paths,
                sync_run_id,
                owner_id,
                source_kind,
                &adapter,
                max_capture_bytes,
                on_progress,
                aggregate,
            )
        }
        SourceKind::Codex => {
            let root = match load_configured_root(conn, source_kind) {
                Ok(Some(root)) => root,
                Ok(None) => {
                    return Ok(failed_outcome(
                        source_kind,
                        "configured_root_required",
                        "source sync requires a configured root",
                    ));
                }
                Err(err) => {
                    return Ok(failed_outcome(
                        source_kind,
                        err.code(),
                        "configured root is invalid",
                    ));
                }
            };
            let adapter = CodexAdapter::new(root);
            sync_adapter_source(
                conn,
                paths,
                sync_run_id,
                owner_id,
                source_kind,
                &adapter,
                max_capture_bytes,
                on_progress,
                aggregate,
            )
        }
        SourceKind::ClaudeCode | SourceKind::OpenCode | SourceKind::Droid => {
            Ok(SyncSourceOutcome {
                source_kind: source_kind.as_str().into(),
                status: "failed".into(),
                accepted_captures: 0,
                skipped_duplicates: 0,
                successful_attempts: 0,
                failed_attempts: 0,
                error_class: Some("adapter_not_registered".into()),
                error_message: Some("source adapter is not registered in this build".into()),
            })
        }
    }
}

/**
 * Run one concrete SourceAdapter through the shared Sync checkpoint ingest path.
 */
#[allow(clippy::too_many_arguments)]
fn sync_adapter_source<F>(
    conn: &mut Connection,
    paths: &DistillPaths,
    sync_run_id: i64,
    owner_id: &str,
    source_kind: SourceKind,
    adapter: &dyn SourceAdapter,
    max_capture_bytes: u64,
    on_progress: &mut F,
    aggregate: &mut IngestReport,
) -> LibraryResult<SyncSourceOutcome>
where
    F: FnMut(SyncProgress),
{
    let check_paths = paths.clone();
    let owner = owner_id.to_string();
    let cancelled = std::cell::Cell::new(false);
    let source_report = {
        let progress = std::cell::RefCell::new(on_progress);
        let mut should_cancel = || {
            let check = open_connection(&check_paths)?;
            refresh_lease(&check, sync_run_id, &owner)?;
            if cancel_requested(&check, sync_run_id)? {
                cancelled.set(true);
                return Ok(true);
            }
            Ok(false)
        };
        let mut on_started = |candidate_id: &str| {
            (*progress.borrow_mut())(SyncProgress::CandidateStarted {
                sync_run_id,
                source_kind: source_kind.as_str().into(),
                candidate_id: candidate_id.to_string(),
            });
        };
        let mut assert_owner = || {
            let check = open_connection(&check_paths)?;
            // Lease ownership only — cancellation requested at CandidateStarted still
            // finishes the current candidate, matching the documented checkpoint policy.
            assert_lease_owned(&check, sync_run_id, &owner)
        };
        let mut on_finished = |candidate_id: &str, outcome: &str| {
            (*progress.borrow_mut())(SyncProgress::CandidateFinished {
                sync_run_id,
                source_kind: source_kind.as_str().into(),
                candidate_id: candidate_id.to_string(),
                outcome: outcome.to_string(),
            });
        };
        match ingest::ingest_adapter_with_checkpoints(
            conn,
            paths,
            adapter,
            max_capture_bytes,
            IngestCheckpoints {
                should_cancel: &mut should_cancel,
                on_candidate_started: &mut on_started,
                assert_owner_before_work: &mut assert_owner,
                on_candidate_finished: &mut on_finished,
                continue_on_candidate_error: true,
            },
        ) {
            Ok(report) => report,
            Err(LibraryError::SyncLeaseLost) => return Err(LibraryError::SyncLeaseLost),
            Err(err) if is_source_outcome_error(&err) => {
                return Ok(failed_outcome(
                    source_kind,
                    redact_error_class(&err),
                    "source sync failed",
                ));
            }
            Err(err) => return Err(err),
        }
    };

    aggregate.accepted_captures += source_report.accepted_captures;
    aggregate.skipped_duplicates += source_report.skipped_duplicates;
    aggregate.successful_attempts += source_report.successful_attempts;
    aggregate.failed_attempts += source_report.failed_attempts;
    for identity in source_report.session_identities {
        if !aggregate.session_identities.contains(&identity) {
            aggregate.session_identities.push(identity);
        }
    }

    let status = if cancelled.get() {
        "cancelled"
    } else if source_report.failed_attempts > 0 {
        if source_report.successful_attempts > 0 || source_report.accepted_captures > 0 {
            "warning"
        } else {
            "failed"
        }
    } else {
        "completed"
    };

    Ok(SyncSourceOutcome {
        source_kind: source_kind.as_str().into(),
        status: status.into(),
        accepted_captures: source_report.accepted_captures,
        skipped_duplicates: source_report.skipped_duplicates,
        successful_attempts: source_report.successful_attempts,
        failed_attempts: source_report.failed_attempts,
        error_class: None,
        error_message: None,
    })
}

fn failed_outcome(kind: SourceKind, class: &str, message: &str) -> SyncSourceOutcome {
    SyncSourceOutcome {
        source_kind: kind.as_str().into(),
        status: "failed".into(),
        accepted_captures: 0,
        skipped_duplicates: 0,
        successful_attempts: 0,
        failed_attempts: 0,
        error_class: Some(class.into()),
        error_message: Some(message.into()),
    }
}

fn load_configured_root(
    conn: &Connection,
    kind: SourceKind,
) -> LibraryResult<Option<std::path::PathBuf>> {
    let root: Option<Option<String>> = conn
        .query_row(
            "SELECT configured_root FROM sources WHERE kind = ?1",
            [kind.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    match root.flatten() {
        Some(text) => Ok(Some(crate::ops::canonicalize_configured_root(Path::new(
            &text,
        ))?)),
        None => Ok(None),
    }
}

fn is_source_outcome_error(err: &LibraryError) -> bool {
    matches!(
        err,
        LibraryError::SourceAdapter(_)
            | LibraryError::InvalidConfiguredRoot { .. }
            | LibraryError::InvalidArgument(_)
            | LibraryError::Json(_)
            | LibraryError::PathOutsideConfiguredRoot { .. }
            | LibraryError::CaptureTooLarge { .. }
            | LibraryError::StagedContentIntegrity { .. }
    )
}

fn redact_error_class(err: &LibraryError) -> &'static str {
    match err {
        LibraryError::SourceAdapter(_) => "source_adapter",
        LibraryError::InvalidConfiguredRoot { .. } => "invalid_configured_root",
        _ => err.code(),
    }
}
