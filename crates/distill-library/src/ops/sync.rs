//! Durable Sync Run queueing, execution, cancellation, and lease health.

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

use crate::adapter::{ParserIdentity, SourceKind};
use crate::error::{LibraryError, LibraryResult};
use crate::ops::sync_execute::sync_one_source;
use crate::ops::sync_lease::{fail_stale_active_runs_inner, refresh_lease, LeaseHeartbeat};
use crate::ops::{self, lease_stale_after};
use crate::storage::DistillPaths;
use crate::types::{
    IngestReport, SyncProgress, SyncRequest, SyncRunResult, SyncRunSummary, SyncSourceOutcome,
};

/// Active Sync Run leases older than this threshold are stale in production.
pub const SYNC_LEASE_STALE_AFTER: std::time::Duration =
    std::time::Duration::from_secs(ops::SYNC_LEASE_STALE_AFTER_SECS);

#[cfg(feature = "test-leases")]
/// Test-only Sync lease timing overrides. Absent from production default builds.
pub mod test_leases {
    use std::time::Duration;

    use crate::ops;

    /// Override the Sync lease stale threshold for the current process.
    pub fn set_lease_stale_after_for_test(duration: Duration) {
        ops::set_test_lease_stale_ms(duration.as_millis() as u64);
    }

    /// Override the background heartbeat interval for the current process.
    pub fn set_heartbeat_interval_for_test(duration: Duration) {
        ops::set_test_heartbeat_interval_ms(duration.as_millis() as u64);
    }

    /// Restore production lease/heartbeat timing.
    pub fn reset_lease_timing_for_test() {
        ops::reset_test_lease_timing();
    }
}

/**
 * Idempotently fail stale queued/running Sync Runs and append one `sync_failed`
 * Activity per newly failed run.
 */
pub fn fail_stale_active_runs(conn: &mut Connection) -> LibraryResult<u64> {
    fail_stale_active_runs_inner(conn, Utc::now())
}

/**
 * Classify Sync Run operations for health: `ok`, `active`, or `failed`.
 */
pub fn active_sync_operations_status(
    conn: &Connection,
) -> LibraryResult<(String, Vec<crate::types::HealthIssue>)> {
    let mut issues = Vec::new();
    let now = Utc::now();
    let active = crate::ops::sync_lease::list_active_runs(conn)?;
    let mut has_active = false;
    let mut has_stale = false;
    for run in &active {
        let expires = match crate::ops::sync_lease::parse_rfc3339(&run.lease_expires_at) {
            Ok(expires) => expires,
            Err(_) => {
                has_stale = true;
                issues.push(crate::types::HealthIssue {
                    code: "invalid_lease_timestamp".into(),
                    severity: "repairable".into(),
                    category: "sync".into(),
                    summary: "a sync run has an invalid lease expiration timestamp".into(),
                });
                continue;
            }
        };
        if expires < now {
            has_stale = true;
            issues.push(crate::types::HealthIssue {
                code: "stale_sync_operation".into(),
                severity: "repairable".into(),
                category: "sync".into(),
                summary: "a sync run lease expired without a heartbeat".into(),
            });
        } else {
            has_active = true;
        }
    }

    if has_stale {
        return Ok(("failed".into(), issues));
    }
    if has_active {
        return Ok(("active".into(), issues));
    }
    Ok(("ok".into(), issues))
}

/**
 * Persist a cancellation request for an active Sync Run.
 *
 * Idempotent for terminal runs: returns `Ok(())` without changing terminal state.
 * Missing ids return [`LibraryError::NotFound`].
 */
pub fn request_cancel(conn: &Connection, sync_run_id: i64) -> LibraryResult<()> {
    let changed = conn.execute(
        "UPDATE sync_runs SET cancel_requested = 1
         WHERE id = ?1 AND status IN ('queued', 'running')",
        [sync_run_id],
    )?;
    if changed == 0 {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM sync_runs WHERE id = ?1",
                [sync_run_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(LibraryError::NotFound(format!("sync run {sync_run_id}")));
        }
    }
    Ok(())
}

/**
 * Load one Sync Run summary including per-source outcomes.
 */
pub fn load_sync_run(conn: &Connection, sync_run_id: i64) -> LibraryResult<SyncRunSummary> {
    let (
        id,
        status,
        cancel_requested,
        metrics_json,
        error_class,
        error_message,
        warning_details_json,
    ): (
        i64,
        String,
        i64,
        String,
        Option<String>,
        Option<String>,
        String,
    ) = conn
        .query_row(
            "SELECT id, status, cancel_requested, metrics_json, error_class, error_message,
                    warning_details_json
             FROM sync_runs WHERE id = ?1",
            [sync_run_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| LibraryError::NotFound(format!("sync run {sync_run_id}")))?;

    let metrics: serde_json::Value =
        serde_json::from_str(&metrics_json).unwrap_or_else(|_| json!({}));
    let warning_details: Vec<String> =
        serde_json::from_str(&warning_details_json).unwrap_or_default();
    let mut summary = SyncRunSummary {
        id,
        status,
        cancel_requested: cancel_requested != 0,
        accepted_captures: metrics
            .get("accepted_captures")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        skipped_duplicates: metrics
            .get("skipped_duplicates")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        successful_attempts: metrics
            .get("successful_attempts")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        failed_attempts: metrics
            .get("failed_attempts")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        error_class,
        error_message,
        warning_details,
        sources: Vec::new(),
    };

    let mut stmt = conn.prepare(
        "SELECT source_kind, status, accepted_captures, skipped_duplicates,
                successful_attempts, failed_attempts, error_class, error_message
         FROM sync_run_source_outcomes
         WHERE sync_run_id = ?1
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([sync_run_id], |row| {
        Ok(SyncSourceOutcome {
            source_kind: row.get(0)?,
            status: row.get(1)?,
            accepted_captures: row.get::<_, i64>(2)? as u64,
            skipped_duplicates: row.get::<_, i64>(3)? as u64,
            successful_attempts: row.get::<_, i64>(4)? as u64,
            failed_attempts: row.get::<_, i64>(5)? as u64,
            error_class: row.get(6)?,
            error_message: row.get(7)?,
        })
    })?;
    for row in rows {
        summary.sources.push(row?);
    }
    Ok(summary)
}

/**
 * Queue and execute a Sync Run through the durable Sync Run seam.
 *
 * Validates Source selection before any Sync Run or Activity side effects.
 * Every post-queue path reaches exactly one terminal durable state/Activity, or
 * returns [`LibraryError::SyncLeaseLost`] after another opener terminalized the run.
 */
pub fn start_sync<F>(
    conn: &mut Connection,
    paths: &DistillPaths,
    owner_id: &str,
    fixture_parser: &ParserIdentity,
    max_capture_bytes: u64,
    request: &SyncRequest,
    mut on_progress: F,
) -> LibraryResult<SyncRunResult>
where
    F: FnMut(SyncProgress),
{
    let sources = resolve_sync_sources(conn, request)?;
    if sources.is_empty() {
        return Err(LibraryError::SyncNoEnabledSources);
    }

    let now = Utc::now();
    let lease_expires = now + lease_chrono()?;

    let sync_run_id = {
        let tx = conn.transaction()?;
        let insert = tx.execute(
            "INSERT INTO sync_runs (
                status, requested_at, cancel_requested, owner_id, heartbeat_at, lease_expires_at,
                metrics_json, warning_details_json
             ) VALUES ('queued', ?1, 0, ?2, ?1, ?3, '{}', '[]')",
            params![now.to_rfc3339(), owner_id, lease_expires.to_rfc3339()],
        );
        match insert {
            Ok(1) => {}
            Ok(_) => {
                return Err(LibraryError::InvalidArgument(
                    "sync run insert affected unexpected rows".into(),
                ));
            }
            Err(err) => {
                if is_unique_violation(&err) {
                    return Err(LibraryError::SyncAlreadyRunning);
                }
                return Err(LibraryError::Sqlite(err));
            }
        }
        let sync_run_id = tx.last_insert_rowid();
        emit_sync_activity(
            &tx,
            "sync_queued",
            sync_run_id,
            json!({ "sync_run_id": sync_run_id }),
        )?;
        tx.commit()?;
        sync_run_id
    };
    on_progress(SyncProgress::RunQueued { sync_run_id });

    let execution = execute_queued_run(
        conn,
        paths,
        owner_id,
        fixture_parser,
        max_capture_bytes,
        sync_run_id,
        sources,
        &mut on_progress,
    );

    match execution {
        Ok(result) => Ok(result),
        Err(LibraryError::SyncLeaseLost) => Err(LibraryError::SyncLeaseLost),
        Err(err) => {
            let _ = best_effort_fail_run(
                conn,
                sync_run_id,
                owner_id,
                "sync_execution_failed",
                "sync run failed without a durable terminal state",
            );
            Err(err)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_queued_run<F>(
    conn: &mut Connection,
    paths: &DistillPaths,
    owner_id: &str,
    fixture_parser: &ParserIdentity,
    max_capture_bytes: u64,
    sync_run_id: i64,
    sources: Vec<SourceKind>,
    on_progress: &mut F,
) -> LibraryResult<SyncRunResult>
where
    F: FnMut(SyncProgress),
{
    let started_at = Utc::now();
    {
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE sync_runs
             SET status = 'running', started_at = ?1, heartbeat_at = ?1, lease_expires_at = ?2
             WHERE id = ?3 AND owner_id = ?4 AND status = 'queued'",
            params![
                started_at.to_rfc3339(),
                (started_at + lease_chrono()?).to_rfc3339(),
                sync_run_id,
                owner_id
            ],
        )?;
        if changed != 1 {
            return Err(LibraryError::SyncLeaseLost);
        }
        emit_sync_activity(
            &tx,
            "sync_started",
            sync_run_id,
            json!({ "sync_run_id": sync_run_id }),
        )?;
        tx.commit()?;
    }
    let heartbeat = LeaseHeartbeat::start(paths.clone(), sync_run_id, owner_id.to_string());
    on_progress(SyncProgress::RunStarted { sync_run_id });

    let mut aggregate = IngestReport::default();
    let mut source_outcomes = Vec::new();
    let mut had_partial_failure = false;
    let mut had_success = false;
    let mut cancelled = false;

    for source_kind in sources {
        refresh_lease(conn, sync_run_id, owner_id)?;
        if cancel_requested(conn, sync_run_id)? {
            cancelled = true;
            break;
        }

        on_progress(SyncProgress::SourceStarted {
            sync_run_id,
            source_kind: source_kind.as_str().into(),
        });

        let outcome = sync_one_source(
            conn,
            paths,
            sync_run_id,
            owner_id,
            source_kind,
            fixture_parser,
            max_capture_bytes,
            on_progress,
            &mut aggregate,
        )?;

        match outcome.status.as_str() {
            "completed" => had_success = true,
            "warning" => {
                had_success = true;
                had_partial_failure = true;
            }
            "failed" => had_partial_failure = true,
            "cancelled" => cancelled = true,
            _ => {}
        }
        on_progress(SyncProgress::SourceFinished {
            sync_run_id,
            source_kind: source_kind.as_str().into(),
            status: outcome.status.clone(),
        });
        source_outcomes.push(outcome);
        if cancelled {
            break;
        }
    }

    // Stop heartbeat before terminalization so it cannot revive a finished run.
    drop(heartbeat);

    let terminal = if cancelled {
        "cancelled"
    } else if !had_success && had_partial_failure {
        "failed"
    } else if had_partial_failure {
        "warning"
    } else {
        "completed"
    };

    terminalize_run(
        conn,
        sync_run_id,
        owner_id,
        terminal,
        &aggregate,
        &source_outcomes,
    )?;

    Ok(SyncRunResult {
        run: load_sync_run(conn, sync_run_id)?,
        session_identities: aggregate.session_identities,
    })
}

fn terminalize_run(
    conn: &mut Connection,
    sync_run_id: i64,
    owner_id: &str,
    terminal: &str,
    aggregate: &IngestReport,
    source_outcomes: &[SyncSourceOutcome],
) -> LibraryResult<()> {
    let finished_at = Utc::now();
    let metrics = json!({
        "accepted_captures": aggregate.accepted_captures,
        "skipped_duplicates": aggregate.skipped_duplicates,
        "successful_attempts": aggregate.successful_attempts,
        "failed_attempts": aggregate.failed_attempts,
    });
    let warning_details: Vec<String> = if terminal == "warning" {
        source_outcomes
            .iter()
            .filter(|outcome| outcome.status == "warning" || outcome.status == "failed")
            .map(|outcome| {
                outcome.error_message.clone().unwrap_or_else(|| {
                    format!(
                        "{} reported {} failed attempts",
                        outcome.source_kind, outcome.failed_attempts
                    )
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    let warning_details_json = serde_json::to_string(&warning_details)?;
    let (error_class, error_message, activity_type, payload) = match terminal {
        "cancelled" => (
            Some("cancelled"),
            Some("sync run cancelled at a safe checkpoint"),
            "sync_failed",
            json!({
                "sync_run_id": sync_run_id,
                "reason": "cancelled",
                "metrics": &metrics,
            }),
        ),
        "failed" => (
            Some("sync_execution_failed"),
            Some("sync run failed without progress"),
            "sync_failed",
            json!({
                "sync_run_id": sync_run_id,
                "reason": "failed",
                "metrics": &metrics,
            }),
        ),
        "warning" => (
            None,
            None,
            "sync_completed",
            json!({
                "sync_run_id": sync_run_id,
                "status": "warning",
                "metrics": &metrics,
                "warning_details": &warning_details,
            }),
        ),
        _ => (
            None,
            None,
            "sync_completed",
            json!({
                "sync_run_id": sync_run_id,
                "status": "completed",
                "metrics": &metrics,
            }),
        ),
    };

    let tx = conn.transaction()?;
    let changed = tx.execute(
        "UPDATE sync_runs
         SET status = ?1,
             finished_at = ?2,
             metrics_json = ?3,
             error_class = ?4,
             error_message = ?5,
             warning_details_json = ?6
         WHERE id = ?7 AND owner_id = ?8 AND status IN ('queued', 'running')",
        params![
            terminal,
            finished_at.to_rfc3339(),
            metrics.to_string(),
            error_class,
            error_message,
            warning_details_json,
            sync_run_id,
            owner_id
        ],
    )?;
    if changed != 1 {
        return Err(LibraryError::SyncLeaseLost);
    }
    for outcome in source_outcomes {
        tx.execute(
            "INSERT INTO sync_run_source_outcomes (
                sync_run_id, source_kind, status, accepted_captures, skipped_duplicates,
                successful_attempts, failed_attempts, error_class, error_message, metrics_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '{}')",
            params![
                sync_run_id,
                outcome.source_kind,
                outcome.status,
                outcome.accepted_captures as i64,
                outcome.skipped_duplicates as i64,
                outcome.successful_attempts as i64,
                outcome.failed_attempts as i64,
                outcome.error_class,
                outcome.error_message
            ],
        )?;
    }
    emit_sync_activity(&tx, activity_type, sync_run_id, payload)?;
    tx.commit()?;
    Ok(())
}

fn best_effort_fail_run(
    conn: &mut Connection,
    sync_run_id: i64,
    owner_id: &str,
    error_class: &str,
    error_message: &str,
) -> LibraryResult<()> {
    let finished_at = Utc::now();
    let tx = conn.transaction()?;
    let changed = tx.execute(
        "UPDATE sync_runs
         SET status = 'failed',
             finished_at = ?1,
             error_class = ?2,
             error_message = ?3
         WHERE id = ?4 AND owner_id = ?5 AND status IN ('queued', 'running')",
        params![
            finished_at.to_rfc3339(),
            error_class,
            error_message,
            sync_run_id,
            owner_id
        ],
    )?;
    if changed == 1 {
        emit_sync_activity(
            &tx,
            "sync_failed",
            sync_run_id,
            json!({
                "sync_run_id": sync_run_id,
                "reason": "failed",
                "error_class": error_class
            }),
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn resolve_sync_sources(
    conn: &Connection,
    request: &SyncRequest,
) -> LibraryResult<Vec<SourceKind>> {
    let prefs = crate::ops::prefs::list_source_preferences(conn)?;
    let mut kinds = Vec::new();
    if request.source_kinds.is_empty() {
        for pref in prefs {
            if pref.enabled {
                if let Some(kind) = SourceKind::parse(&pref.kind) {
                    kinds.push(kind);
                }
            }
        }
    } else {
        for name in &request.source_kinds {
            let kind = SourceKind::parse(name).ok_or_else(|| {
                LibraryError::InvalidArgument(format!("unknown source kind: {name}"))
            })?;
            let enabled = prefs
                .iter()
                .find(|pref| pref.kind == kind.as_str())
                .map(|pref| pref.enabled)
                .unwrap_or(false);
            if enabled {
                kinds.push(kind);
            }
        }
    }
    Ok(kinds)
}

/// Read whether a durable cancel request is pending for a Sync Run.
pub(crate) fn cancel_requested(conn: &Connection, sync_run_id: i64) -> LibraryResult<bool> {
    let flag: i64 = conn.query_row(
        "SELECT cancel_requested FROM sync_runs WHERE id = ?1",
        [sync_run_id],
        |row| row.get(0),
    )?;
    Ok(flag != 0)
}

/// Append a canonical sync Activity Event.
pub(crate) fn emit_sync_activity(
    conn: &Connection,
    event_type: &str,
    sync_run_id: i64,
    payload: serde_json::Value,
) -> LibraryResult<()> {
    conn.execute(
        "INSERT INTO activity_events (
            event_type, occurred_at, source_kind, session_id, capture_id, attempt_id, payload_json
         ) VALUES (?1, ?2, NULL, NULL, NULL, NULL, ?3)",
        params![
            event_type,
            Utc::now().to_rfc3339(),
            json!({ "object_type": "sync_job", "object_id": sync_run_id, "payload": payload })
                .to_string()
        ],
    )?;
    Ok(())
}

fn is_unique_violation(err: &rusqlite::Error) -> bool {
    match err {
        rusqlite::Error::SqliteFailure(code, _) => {
            code.code == rusqlite::ErrorCode::ConstraintViolation
        }
        _ => false,
    }
}

/// Convert the configured lease stale duration into a chrono duration.
pub(crate) fn lease_chrono() -> LibraryResult<chrono::Duration> {
    chrono::Duration::from_std(lease_stale_after())
        .map_err(|_| LibraryError::InvalidArgument("sync lease duration is invalid".into()))
}
