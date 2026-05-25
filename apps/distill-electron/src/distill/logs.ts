import { openDistillDatabase } from "./db";
import { getBackgroundSyncStatus } from "./jobs";
import {
  DatasetExportTarget,
  ImportFailureEntry,
  ImportSourceSummary,
  LogEntry,
  LogEntryStatus,
  LogsPageData
} from "../shared/types";

type SyncJobRow = {
  id: number;
  status: string;
  last_error: string | null;
  payload_json: string;
  created_at: string;
  updated_at: string;
};

type ExportRow = {
  id: number;
  export_type: string;
  label_filter: string | null;
  output_path: string;
  record_count: number;
  metadata_json: string;
  created_at: string;
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

type ExportPayload = {
  exportedAt?: string;
  dataset?: DatasetExportTarget;
};

function parseJson<T>(value: string, fallback: T): T {
  try {
    const parsed = JSON.parse(value) as T;
    return parsed && typeof parsed === "object" ? parsed : fallback;
  } catch {
    return fallback;
  }
}

function hasSyncWarnings(payload: Pick<SyncPayload, "failedCaptures" | "failedEntries">): boolean {
  return (payload.failedCaptures ?? 0) > 0 || (payload.failedEntries?.length ?? 0) > 0;
}

function normalizeSyncStatus(status: string, payload: SyncPayload): LogEntryStatus {
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

  return "queued";
}

function summarizeSync(status: LogEntryStatus, payload: SyncPayload): string {
  if (payload.summary?.trim()) {
    return payload.summary.trim();
  }

  if (status === "queued") {
    return `Queued sync${payload.reason ? `: ${payload.reason}` : ""}`;
  }

  if (status === "running") {
    return `Sync running${payload.reason ? `: ${payload.reason}` : ""}`;
  }

  if (status === "warning") {
    return "Sync warnings";
  }

  if (status === "failed") {
    return "Sync failed";
  }

  return "Sync completed";
}

function stringifyRaw(value: Record<string, unknown>): string {
  return JSON.stringify(value, null, 2);
}

function normalizeExportDataset(value: string | null | undefined): DatasetExportTarget | undefined {
  if (value === "train" || value === "holdout") {
    return value;
  }

  return undefined;
}

function mapSyncJob(row: SyncJobRow): LogEntry {
  const payload = parseJson<SyncPayload>(row.payload_json, {});
  const status = normalizeSyncStatus(row.status, payload);
  const failedEntries = payload.failedEntries ?? [];
  const failedCaptures = payload.failedCaptures ?? 0;
  const level = status === "failed" ? "error" : "info";

  return {
    id: `sync-${row.id}`,
    kind: "sync",
    status,
    level,
    title: "Background sync",
    summary: summarizeSync(status, payload),
    createdAt: payload.startedAt ?? row.created_at,
    updatedAt: payload.finishedAt ?? row.updated_at,
    sourceLabel: payload.reason,
    metrics: {
      discoveredCaptures: payload.discoveredCaptures ?? 0,
      importedCaptures: payload.importedCaptures ?? 0,
      skippedCaptures: payload.skippedCaptures ?? 0,
      failedCaptures
    },
    details: {
      reason: payload.reason,
      sourceSummaries: payload.sourceSummaries ?? [],
      failedEntries
    },
    rawJson: stringifyRaw({
      jobId: row.id,
      status: row.status,
      lastError: row.last_error ?? undefined,
      ...payload
    })
  };
}

function mapExport(row: ExportRow): LogEntry {
  const payload = parseJson<ExportPayload>(row.metadata_json, {});
  const dataset = normalizeExportDataset(payload.dataset ?? row.label_filter);
  const datasetLabel = dataset ?? row.label_filter ?? "all";
  const recordLabel = row.record_count === 1 ? "record" : "records";

  return {
    id: `export-${row.id}`,
    kind: "export",
    status: "completed",
    level: "info",
    title: "Export",
    summary: `Exported ${row.record_count} ${datasetLabel} dataset ${recordLabel}`,
    createdAt: payload.exportedAt ?? row.created_at,
    updatedAt: payload.exportedAt ?? row.created_at,
    sourceLabel: datasetLabel,
    metrics: {
      recordCount: row.record_count
    },
    details: {
      dataset,
      outputPath: row.output_path
    },
    rawJson: stringifyRaw({
      exportId: row.id,
      exportType: row.export_type,
      dataset: datasetLabel,
      outputPath: row.output_path,
      recordCount: row.record_count,
      ...payload
    })
  };
}

function sortTime(entry: LogEntry): number {
  const timestamp = entry.updatedAt ?? entry.createdAt;
  if (!timestamp) {
    return 0;
  }

  const time = new Date(timestamp).getTime();
  return Number.isFinite(time) ? time : 0;
}

export function getLogsPageData(limit = 200): LogsPageData {
  const distillDb = openDistillDatabase();

  try {
    const syncJobs = distillDb.db
      .prepare(`
        SELECT id, status, last_error, payload_json, created_at, updated_at
        FROM jobs
        WHERE job_type = 'sync_sources'
        ORDER BY COALESCE(updated_at, created_at) DESC
        LIMIT ?
      `)
      .all(limit) as SyncJobRow[];

    const exports = distillDb.db
      .prepare(`
        SELECT id, export_type, label_filter, output_path, record_count, metadata_json, created_at
        FROM exports
        ORDER BY created_at DESC
        LIMIT ?
      `)
      .all(limit) as ExportRow[];

    const entries = [
      ...syncJobs.map(mapSyncJob),
      ...exports.map(mapExport)
    ]
      .sort((a, b) => sortTime(b) - sortTime(a))
      .slice(0, limit);

    return {
      entries,
      counts: {
        total: entries.length,
        errors: entries.filter((entry) => entry.level === "error").length,
        running: entries.filter((entry) => entry.status === "running").length
      },
      lastSyncStatus: syncJobs.length ? getBackgroundSyncStatus() : undefined
    };
  } finally {
    distillDb.close();
  }
}
