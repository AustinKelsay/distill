//! Durable Sync Run lease ownership, stale repair, and background heartbeat.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

use crate::error::{LibraryError, LibraryResult};
use crate::ops::{heartbeat_interval, lease_stale_after};
use crate::storage::{open_connection, DistillPaths};

/// Active Sync Run row used for stale classification.
pub(crate) struct ActiveRunRow {
    pub id: i64,
    pub lease_expires_at: String,
}

/**
 * Idempotently fail stale queued/running Sync Runs and append one `sync_failed`
 * Activity per newly failed run.
 */
pub(crate) fn fail_stale_active_runs_inner(
    conn: &mut Connection,
    now: DateTime<Utc>,
) -> LibraryResult<u64> {
    let active = list_active_runs(conn)?;
    let mut failed = 0_u64;
    for run in active {
        // Leave malformed lease rows active so Library::open succeeds and the
        // typed health surface can report `invalid_lease_timestamp` for repair.
        let Ok(expires) = parse_rfc3339(&run.lease_expires_at) else {
            continue;
        };
        if expires >= now {
            continue;
        }
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE sync_runs
             SET status = 'failed',
                 finished_at = ?1,
                 error_class = 'stale_sync_operation',
                 error_message = 'sync run lease expired without heartbeat'
             WHERE id = ?2
               AND status IN ('queued', 'running')
               AND lease_expires_at < ?1",
            params![now.to_rfc3339(), run.id],
        )?;
        if changed == 1 {
            tx.execute(
                "INSERT INTO activity_events (
                    event_type, occurred_at, source_kind, session_id, capture_id, attempt_id, payload_json
                 ) VALUES ('sync_failed', ?1, NULL, NULL, NULL, NULL, ?2)",
                params![
                    now.to_rfc3339(),
                    json!({
                        "object_type": "sync_job",
                        "object_id": run.id,
                        "payload": {
                            "reason": "stale_lease",
                            "error_class": "stale_sync_operation"
                        }
                    })
                    .to_string()
                ],
            )?;
            failed += 1;
        }
        tx.commit()?;
    }
    Ok(failed)
}

/**
 * Renew the Sync Run lease when this owner still holds an active run.
 *
 * Zero changed rows returns [`LibraryError::SyncLeaseLost`].
 */
pub(crate) fn refresh_lease(
    conn: &Connection,
    sync_run_id: i64,
    owner_id: &str,
) -> LibraryResult<()> {
    let now = Utc::now();
    let expires = now
        + chrono::Duration::from_std(lease_stale_after())
            .map_err(|_| LibraryError::InvalidArgument("sync lease duration is invalid".into()))?;
    let changed = conn.execute(
        "UPDATE sync_runs
         SET heartbeat_at = ?1, lease_expires_at = ?2
         WHERE id = ?3 AND owner_id = ?4 AND status IN ('queued', 'running')",
        params![
            now.to_rfc3339(),
            expires.to_rfc3339(),
            sync_run_id,
            owner_id
        ],
    )?;
    if changed == 0 {
        return Err(LibraryError::SyncLeaseLost);
    }
    Ok(())
}

/**
 * Assert lease ownership without renewing (post-progress, pre-candidate work).
 *
 * Does not observe cancellation. A worker whose lease was failed elsewhere must
 * stop before accepting new Capture work.
 */
pub(crate) fn assert_lease_owned(
    conn: &Connection,
    sync_run_id: i64,
    owner_id: &str,
) -> LibraryResult<()> {
    let owned: Option<i64> = conn
        .query_row(
            "SELECT id FROM sync_runs
             WHERE id = ?1 AND owner_id = ?2 AND status IN ('queued', 'running')",
            params![sync_run_id, owner_id],
            |row| row.get(0),
        )
        .optional()?;
    if owned.is_none() {
        return Err(LibraryError::SyncLeaseLost);
    }
    Ok(())
}

pub(crate) fn list_active_runs(conn: &Connection) -> LibraryResult<Vec<ActiveRunRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, lease_expires_at FROM sync_runs
         WHERE status IN ('queued', 'running')
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ActiveRunRow {
            id: row.get(0)?,
            lease_expires_at: row.get(1)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(crate) fn parse_rfc3339(value: &str) -> LibraryResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| LibraryError::InvalidArgument("sync run lease timestamp is invalid".into()))
}

/// Background lease heartbeat for one Sync Run lifetime.
pub(crate) struct LeaseHeartbeat {
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl LeaseHeartbeat {
    /**
     * Start a lightweight heartbeat on a separate SQLite connection.
     *
     * Stops promptly on drop. Does not renew once the run is terminal or ownership
     * is lost, so it cannot keep a stale-failed or finished run alive.
     */
    pub fn start(paths: DistillPaths, sync_run_id: i64, owner_id: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let interval = heartbeat_interval();
        let join = thread::spawn(move || {
            heartbeat_loop(paths, sync_run_id, owner_id, stop_flag, interval);
        });
        Self {
            stop,
            join: Some(join),
        }
    }
}

impl Drop for LeaseHeartbeat {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

fn heartbeat_loop(
    paths: DistillPaths,
    sync_run_id: i64,
    owner_id: String,
    stop: Arc<AtomicBool>,
    interval: Duration,
) {
    let slice = Duration::from_millis(10).min(interval);
    while !stop.load(Ordering::SeqCst) {
        if let Ok(conn) = open_connection(&paths) {
            match refresh_lease(&conn, sync_run_id, &owner_id) {
                Ok(()) => {}
                Err(LibraryError::SyncLeaseLost) => return,
                // A transient connection or SQLite contention failure must not
                // permanently disable renewal. Retry after the normal interval;
                // persistent failures still age into the ordinary stale repair.
                Err(_) => {}
            }
        }
        let mut waited = Duration::ZERO;
        while waited < interval {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            thread::sleep(slice);
            waited += slice;
        }
    }
}
