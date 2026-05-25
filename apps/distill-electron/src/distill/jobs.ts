import { DatabaseSync } from "node:sqlite";
import { insertActivityEvent, openDistillDatabase } from "./db";
import { runImport } from "./import";
import { BackgroundSyncStatus, ImportFailureEntry, ImportReport, ImportSourceSummary } from "../shared/types";

type JobRow = {
  id: number;
  status: string;
  last_error: string | null;
  payload_json: string;
};

type SyncPayload = {
  reason?: string;
  startedAt?: string;
  finishedAt?: string;
  discoveredCaptures?: number;
  importedCaptures?: number;
  skippedCaptures?: number;
  failedCaptures?: number;
  summary?: string;
  sourceSummaries?: ImportSourceSummary[];
  failedEntries?: ImportFailureEntry[];
  outcome?: "completed" | "warning" | "failed";
};

function parsePayload(payloadJson: string): SyncPayload {
  try {
    const parsed = JSON.parse(payloadJson) as SyncPayload;
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function hasSyncWarnings(payload: Pick<SyncPayload, "failedCaptures" | "failedEntries">): boolean {
  return (payload.failedCaptures ?? 0) > 0 || (payload.failedEntries?.length ?? 0) > 0;
}

function deriveSyncState(status: string, payload: SyncPayload): BackgroundSyncStatus["state"] {
  if (status === "pending") {
    return "queued";
  }

  if (status === "running" || status === "warning" || status === "failed") {
    return status;
  }

  if (status === "completed") {
    if (payload.outcome) {
      return payload.outcome;
    }

    return hasSyncWarnings(payload) ? "warning" : "completed";
  }

  return "idle";
}

function defaultSummary(state: BackgroundSyncStatus["state"], payload: SyncPayload): string {
  if (state === "queued") {
    return `Queued sync${payload.reason ? `: ${payload.reason}` : ""}`;
  }

  if (state === "running") {
    return `Sync running${payload.reason ? `: ${payload.reason}` : ""}`;
  }

  if (state === "warning") {
    return "Sync warnings";
  }

  if (state === "failed") {
    return "Sync failed";
  }

  if (state === "completed") {
    return "Sync completed";
  }

  return "Idle";
}

function withTransaction<T>(db: DatabaseSync, fn: () => T): T {
  let transactionOpen = false;

  try {
    db.exec("BEGIN");
    transactionOpen = true;
    const result = fn();
    db.exec("COMMIT");
    transactionOpen = false;
    return result;
  } catch (error) {
    if (transactionOpen) {
      try {
        db.exec("ROLLBACK");
      } catch {
        // Preserve the original transaction error below.
      }
    }

    throw error;
  }
}

function insertSyncJobAuditEvent(
  db: DatabaseSync,
  input: {
    eventType: "sync_queued" | "sync_started" | "sync_completed" | "sync_failed";
    jobId: number;
    payload: Record<string, unknown>;
  }
): void {
  insertActivityEvent(db, {
    eventType: input.eventType,
    objectType: "sync_job",
    objectId: input.jobId,
    payload: input.payload
  });
}

function toStatus(row: JobRow | undefined): BackgroundSyncStatus {
  if (!row) {
    return {
      state: "idle",
      discoveredCaptures: 0,
      importedCaptures: 0,
      skippedCaptures: 0,
      failedCaptures: 0,
      summary: "Idle"
    };
  }

  const payload = parsePayload(row.payload_json);
  const state = deriveSyncState(row.status, payload);

  return {
    state,
    jobId: row.id,
    reason: payload.reason,
    startedAt: payload.startedAt,
    finishedAt: payload.finishedAt,
    discoveredCaptures: payload.discoveredCaptures ?? 0,
    importedCaptures: payload.importedCaptures ?? 0,
    skippedCaptures: payload.skippedCaptures ?? 0,
    failedCaptures: payload.failedCaptures ?? 0,
    summary: payload.summary ?? defaultSummary(state, payload),
    errorText: row.last_error ?? undefined,
    sourceSummaries: payload.sourceSummaries,
    failedEntries: payload.failedEntries
  };
}

function summarizeImport(report: ImportReport): BackgroundSyncStatus {
  const discoveredCaptures = report.sourceSummaries.reduce((sum, source) => sum + source.discoveredCaptures, 0);
  const importedCaptures = report.sourceSummaries.reduce((sum, source) => sum + source.importedCaptures, 0);
  const skippedCaptures = report.sourceSummaries.reduce((sum, source) => sum + source.skippedCaptures, 0);
  const failedCaptures = report.sourceSummaries.reduce((sum, source) => sum + source.failedCaptures, 0);
  const failedEntries = report.failedEntries;
  const state = failedCaptures > 0 || failedEntries.length > 0 ? "warning" : "completed";
  const summary =
    state === "warning" ?
      `Sync warnings: ${importedCaptures} imported, ${skippedCaptures} skipped, ${failedCaptures} failed across ${discoveredCaptures} captures`
    : `Sync complete: ${importedCaptures} imported, ${skippedCaptures} skipped, ${failedCaptures} failed across ${discoveredCaptures} captures`;

  return {
    state,
    startedAt: report.importedAt,
    finishedAt: report.importedAt,
    discoveredCaptures,
    importedCaptures,
    skippedCaptures,
    failedCaptures,
    summary,
    sourceSummaries: report.sourceSummaries,
    failedEntries
  };
}

export function markStaleRunningSyncJobsFailed(): void {
  const distillDb = openDistillDatabase();
  try {
    const runningJobs = distillDb.db
      .prepare(`
        SELECT id, payload_json
        FROM jobs
        WHERE job_type = 'sync_sources'
        AND status = 'running'
      `)
      .all() as Array<{ id: number; payload_json: string }>;

    if (runningJobs.length === 0) {
      return;
    }

    withTransaction(distillDb.db, () => {
      const update = distillDb.db.prepare(`
        UPDATE jobs
        SET status = 'failed',
            last_error = ?,
            payload_json = ?,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
      `);

      for (const row of runningJobs) {
        const payload = parsePayload(row.payload_json);
        const finishedAt = new Date().toISOString();
        const errorText = "Interrupted before completion";

        update.run(
          errorText,
          JSON.stringify({
            ...payload,
            finishedAt,
            summary: "Sync failed",
            outcome: "failed"
          }),
          row.id
        );
        insertSyncJobAuditEvent(distillDb.db, {
          eventType: "sync_failed",
          jobId: row.id,
          payload: {
            reason: payload.reason,
            errorText,
            scope: "job",
            fatal: true
          }
        });
      }
    });
  } finally {
    distillDb.close();
  }
}

export function enqueueSourceSyncJob(reason: string): number {
  const distillDb = openDistillDatabase();
  try {
    const row = withTransaction(distillDb.db, () => {
      const inserted = distillDb.db.prepare(`
        INSERT INTO jobs (
          job_type,
          object_type,
          object_id,
          status,
          run_after,
          payload_json,
          updated_at
        ) VALUES ('sync_sources', 'system', 1, 'pending', CURRENT_TIMESTAMP, ?, CURRENT_TIMESTAMP)
        RETURNING id
      `).get(JSON.stringify({ reason, summary: `Queued sync: ${reason}` })) as { id: number };

      insertSyncJobAuditEvent(distillDb.db, {
        eventType: "sync_queued",
        jobId: inserted.id,
        payload: {
          reason,
          jobType: "sync_sources"
        }
      });

      return inserted;
    });

    return row.id;
  } finally {
    distillDb.close();
  }
}

export function getBackgroundSyncStatus(): BackgroundSyncStatus {
  const distillDb = openDistillDatabase();
  try {
    const row = distillDb.db.prepare(`
      SELECT id, status, last_error, payload_json
      FROM jobs
      WHERE job_type = 'sync_sources'
      ORDER BY id DESC
      LIMIT 1
    `).get() as JobRow | undefined;

    return toStatus(row);
  } finally {
    distillDb.close();
  }
}

export function runNextSourceSyncJob(): BackgroundSyncStatus {
  const distillDb = openDistillDatabase();
  let jobId: number | undefined;
  let startedAt = new Date().toISOString();
  let reason: string | undefined;

  try {
    const row = distillDb.db.prepare(`
      SELECT id, payload_json
      FROM jobs
      WHERE job_type = 'sync_sources'
      AND status = 'pending'
      AND (run_after IS NULL OR run_after <= CURRENT_TIMESTAMP)
      ORDER BY id ASC
      LIMIT 1
    `).get() as { id: number; payload_json: string } | undefined;

    if (!row) {
      return getBackgroundSyncStatus();
    }

    jobId = row.id;
    const payload = parsePayload(row.payload_json);
    reason = payload.reason;
    startedAt = new Date().toISOString();

    withTransaction(distillDb.db, () => {
      distillDb.db.prepare(`
        UPDATE jobs
        SET status = 'running',
            attempts = attempts + 1,
            payload_json = ?,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
      `).run(
        JSON.stringify({
          ...payload,
          startedAt,
          summary: `Sync running: ${payload.reason ?? "scheduled"}`
        }),
        row.id
      );
      insertSyncJobAuditEvent(distillDb.db, {
        eventType: "sync_started",
        jobId: row.id,
        payload: {
          reason,
          jobType: "sync_sources"
        }
      });
    });
  } finally {
    distillDb.close();
  }

  try {
    const report = runImport({
      syncJobId: jobId,
      syncReason: reason
    });
    const status = summarizeImport(report);

    const completeDb = openDistillDatabase();
    try {
      withTransaction(completeDb.db, () => {
        completeDb.db.prepare(`
          UPDATE jobs
          SET status = ?,
              last_error = NULL,
              payload_json = ?,
              updated_at = CURRENT_TIMESTAMP
          WHERE id = ?
        `).run(
          status.state === "warning" ? "warning" : "completed",
          JSON.stringify({
            startedAt,
            finishedAt: status.finishedAt,
            reason,
            discoveredCaptures: status.discoveredCaptures,
            importedCaptures: status.importedCaptures,
            skippedCaptures: status.skippedCaptures,
            failedCaptures: status.failedCaptures,
            summary: status.summary,
            sourceSummaries: status.sourceSummaries,
            failedEntries: status.failedEntries,
            outcome: status.state === "warning" ? "warning" : "completed"
          }),
          jobId
        );
        insertSyncJobAuditEvent(completeDb.db, {
          eventType: "sync_completed",
          jobId,
          payload: {
            reason,
            discoveredCaptures: status.discoveredCaptures,
            importedCaptures: status.importedCaptures,
            skippedCaptures: status.skippedCaptures,
            failedCaptures: status.failedCaptures,
            failedEntryCount: status.failedEntries?.length ?? 0,
            outcome: status.state === "warning" ? "warning" : "completed"
          }
        });
      });
    } finally {
      completeDb.close();
    }

    return {
      ...status,
      jobId
    };
  } catch (error) {
    const errorText = error instanceof Error ? error.message : String(error);
    const failedAt = new Date().toISOString();
    const failedDb = openDistillDatabase();

    try {
      withTransaction(failedDb.db, () => {
        failedDb.db.prepare(`
          UPDATE jobs
          SET status = 'failed',
              last_error = ?,
              payload_json = ?,
              updated_at = CURRENT_TIMESTAMP
          WHERE id = ?
        `).run(
          errorText,
          JSON.stringify({
            startedAt,
            finishedAt: failedAt,
            reason,
            summary: "Sync failed",
            failedEntries: [],
            outcome: "failed"
          }),
          jobId
        );
        insertSyncJobAuditEvent(failedDb.db, {
          eventType: "sync_failed",
          jobId,
          payload: {
            reason,
            errorText,
            scope: "job",
            fatal: true
          }
        });
      });
    } finally {
      failedDb.close();
    }

    return {
      state: "failed",
      jobId,
      reason,
      startedAt,
      finishedAt: failedAt,
      discoveredCaptures: 0,
      importedCaptures: 0,
      skippedCaptures: 0,
      failedCaptures: 0,
      summary: "Sync failed",
      errorText,
      failedEntries: []
    };
  }
}
