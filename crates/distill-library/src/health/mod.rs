//! Library health classification, safe open reconciliation, and explicit repair.

mod cas;
mod checks;
mod content;
mod repair_ops;

use std::fs;

use rusqlite::Connection;

use crate::error::LibraryResult;
use crate::storage::DistillPaths;
use crate::types::{HealthReport, OpenReconciliation, RepairAction, RepairOptions, RepairReport};

use cas::is_canonical_staging_partial_name;
use checks::{
    check_fts_projection_agreement, check_incomplete_state, check_orphan_blobs,
    check_schema_integrity, check_staging_partials,
};
use content::check_referenced_content;
use repair_ops::{rebuild_fts_from_projection, remove_orphan_blobs, resolve_incomplete_state};

/**
 * Remove only canonical disposable staging partials and return what was reconciled.
 *
 * Only `{64 lowercase hex}.partial` regular files under the Distill staging directory
 * are removed. Noncanonical names, symlinks, referenced CAS blobs, and durable SQLite
 * rows are never touched.
 */
pub fn reconcile_on_open(paths: &DistillPaths) -> LibraryResult<OpenReconciliation> {
    let mut removed = 0_u64;
    if paths.staging.is_dir() {
        for entry in fs::read_dir(&paths.staging)? {
            let entry = entry?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !is_canonical_staging_partial_name(name) {
                continue;
            }
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            if !meta.file_type().is_file() {
                continue;
            }
            fs::remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(OpenReconciliation {
        removed_staging_partials: removed,
    })
}

/**
 * Build a typed health report covering schema, content, FTS, staging, orphans,
 * incomplete durable state, and an explicit operations handoff status.
 *
 * Sync-run stale operations are not representable until issue #22; the report
 * exposes `operations_status = "not_applicable"` rather than inventing jobs.
 */
pub fn health(
    conn: &Connection,
    paths: &DistillPaths,
    open_reconciliation: &OpenReconciliation,
) -> LibraryResult<HealthReport> {
    let mut issues = Vec::new();

    let schema_status = check_schema_integrity(conn, &mut issues)?;
    let content_status = check_referenced_content(conn, &paths.home, &mut issues)?;
    let fts_status = check_fts_projection_agreement(conn, &mut issues)?;
    let staging_status = check_staging_partials(&paths.staging, &mut issues)?;
    let orphan_status = check_orphan_blobs(conn, &paths.home, &paths.blobs, &mut issues)?;
    let incomplete_status = check_incomplete_state(conn, &mut issues)?;

    Ok(HealthReport {
        ok: issues.iter().all(|issue| issue.severity == "info"),
        schema_status,
        content_status,
        fts_status,
        staging_status,
        orphan_status,
        incomplete_status,
        // Explicit until Sync job health lands in #22.
        operations_status: "not_applicable".to_string(),
        issues,
        open_reconciliation: open_reconciliation.clone(),
    })
}

/**
 * Explicit idempotent repair for documented repairable Library states.
 *
 * Never deletes referenced content or mutates immutable Captures/Facts.
 */
pub fn repair(
    conn: &mut Connection,
    paths: &DistillPaths,
    options: &RepairOptions,
    open_reconciliation: &OpenReconciliation,
) -> LibraryResult<RepairReport> {
    let mut actions = Vec::new();

    if options.remove_orphan_blobs {
        let count = remove_orphan_blobs(conn, &paths.home, &paths.blobs)?;
        actions.push(RepairAction {
            name: "removed_orphan_blobs".into(),
            count,
        });
    }

    if options.resolve_incomplete_state {
        let resolved = resolve_incomplete_state(conn)?;
        actions.push(RepairAction {
            name: "failed_pending_attempts".into(),
            count: resolved.failed_pending_attempts,
        });
        actions.push(RepairAction {
            name: "appended_capture_failed_recoveries".into(),
            count: resolved.appended_capture_failed_recoveries,
        });
        actions.push(RepairAction {
            name: "recomputed_session_counters".into(),
            count: resolved.recomputed_session_counters,
        });
    }

    if options.rebuild_fts {
        let count = rebuild_fts_from_projection(conn)?;
        actions.push(RepairAction {
            name: "rebuilt_fts_rows".into(),
            count,
        });
    }

    // Safe staging cleanup remains available through repair for homes opened
    // before partials appeared, without requiring another open cycle.
    let staging = reconcile_on_open(paths)?;
    actions.push(RepairAction {
        name: "removed_staging_partials".into(),
        count: staging.removed_staging_partials,
    });

    let health_after = health(conn, paths, open_reconciliation)?;
    Ok(RepairReport {
        actions,
        health_after,
    })
}
