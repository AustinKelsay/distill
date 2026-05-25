import { DatabaseSync } from "node:sqlite";
import { sourceConnectors } from "../connectors";
import { CaptureSnapshot, SourceConnector } from "../connectors/types";
import {
  encodeCapturePayload,
  findCapture,
  insertActivityEvent,
  openDistillDatabase,
  replaceSessionProjection,
  updateCaptureFailure,
  upsertSource
} from "./db";
import { getDistillHome } from "./paths";
import { persistCaptureContent } from "./raw_capture";
import {
  DiscoveredCapture,
  DiscoveredSource,
  ImportFailureEntry,
  ImportReport,
  ImportedCapture
} from "../shared/types";

const PARSER_VERSION = "v0";

type RunImportOptions = {
  syncJobId?: number;
  syncReason?: string;
};

function insertSyncFailureAuditEvent(
  db: DatabaseSync,
  input: {
    syncJobId?: number;
    syncReason?: string;
    sourceKind: string;
    stage: "detect" | "discover";
    sourcePath: string;
    errorText: string;
  }
): void {
  insertActivityEvent(db, {
    eventType: "sync_failed",
    objectType: "sync_job",
    objectId: input.syncJobId ?? null,
    payload: {
      reason: input.syncReason,
      sourceKind: input.sourceKind,
      stage: input.stage,
      sourcePath: input.sourcePath,
      errorText: input.errorText,
      fatal: false,
      scope: "source"
    }
  });
}

function insertCapture(
  db: DatabaseSync,
  sourceId: number,
  capture: DiscoveredCapture,
  snapshot: Pick<CaptureSnapshot, "rawSha256" | "sourceModifiedAt" | "sourceSizeBytes">,
  rawBlobPath: string | null,
  rawPayloadJson: string
): number {
  const result = db
    .prepare(`
      INSERT INTO captures (
        source_id,
        capture_kind,
        external_session_id,
        source_path,
        source_modified_at,
        source_size_bytes,
        raw_sha256,
        raw_blob_path,
        raw_payload_json,
        parser_version,
        status,
        captured_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      RETURNING id
    `)
    .get(
      sourceId,
      capture.captureKind,
      capture.externalSessionId ?? null,
      capture.sourcePath,
      snapshot.sourceModifiedAt ?? capture.sourceModifiedAt ?? null,
      snapshot.sourceSizeBytes ?? capture.sourceSizeBytes ?? null,
      snapshot.rawSha256,
      rawBlobPath,
      rawPayloadJson,
      PARSER_VERSION,
      "captured",
      new Date().toISOString()
    ) as { id: number };

  insertActivityEvent(db, {
    eventType: "capture_recorded",
    objectType: "capture",
    objectId: result.id,
    payload: {
      sourceKind: capture.sourceKind,
      sourcePath: capture.sourcePath,
      externalSessionId: capture.externalSessionId ?? null
    }
  });

  return result.id;
}

function importSourceCaptures(
  db: DatabaseSync,
  connector: SourceConnector,
  source: DiscoveredSource,
  sourceId: number,
  captures: DiscoveredCapture[]
): {
  importedCaptures: number;
  skippedCaptures: number;
  failedCaptures: number;
  captures: ImportedCapture[];
  failedEntries: ImportFailureEntry[];
} {
  let importedCaptures = 0;
  let skippedCaptures = 0;
  let failedCaptures = 0;
  const imported: ImportedCapture[] = [];
  const failedEntries: ImportFailureEntry[] = [];

  for (const capture of captures) {
    let snapshot: CaptureSnapshot;

    try {
      snapshot = connector.snapshotCapture(capture);
    } catch (error) {
      const errorText = error instanceof Error ? error.message : String(error);
      insertActivityEvent(db, {
        eventType: "capture_failed",
        objectType: "capture",
        payload: {
          sourceKind: source.kind,
          sourcePath: capture.sourcePath,
          externalSessionId: capture.externalSessionId ?? null,
          stage: "snapshot",
          errorText
        }
      });
      imported.push({
        sourcePath: capture.sourcePath,
        externalSessionId: capture.externalSessionId,
        skipped: false,
        status: "failed",
        errorText
      });
      failedEntries.push({
        sourceKind: source.kind,
        sourcePath: capture.sourcePath,
        errorText
      });
      failedCaptures += 1;
      continue;
    }

    const existingCapture = findCapture(db, sourceId, capture.sourcePath, snapshot.rawSha256);

    if (existingCapture && (existingCapture.status === "normalized" || existingCapture.status === "failed_parse")) {
      skippedCaptures += 1;
      imported.push({
        sourcePath: capture.sourcePath,
        externalSessionId: capture.externalSessionId,
        rawSha256: snapshot.rawSha256,
        skipped: true,
        status: "skipped"
      });
      continue;
    }

    let captureId: number | undefined;
    let failureStage: "persistence" | "parse" = "persistence";
    try {
      const contentRef = persistCaptureContent(capture, snapshot);
      const rawBlobPath = contentRef.kind === "blob" ? contentRef.blobPath : null;
      const rawPayloadJson = encodeCapturePayload(capture.sourceKind, capture.metadata, contentRef);

      if (existingCapture) {
        captureId = existingCapture.id;
        db.prepare(`
          UPDATE captures
          SET source_modified_at = ?,
              source_size_bytes = ?,
              raw_blob_path = ?,
              raw_payload_json = ?,
              error_text = NULL,
              parser_version = ?
          WHERE id = ?
        `).run(
          snapshot.sourceModifiedAt ?? capture.sourceModifiedAt ?? null,
          snapshot.sourceSizeBytes ?? capture.sourceSizeBytes ?? contentRef.byteSize,
          rawBlobPath,
          rawPayloadJson,
          PARSER_VERSION,
          captureId
        );
      } else {
        captureId = insertCapture(db, sourceId, capture, snapshot, rawBlobPath, rawPayloadJson);
      }

      failureStage = "parse";
      const parsedCapture = connector.parseCapture(capture, snapshot);
      if (captureId === undefined) {
        throw new Error("Capture id was not assigned before projection replacement");
      }
      const finalizedCaptureId = captureId;

      replaceSessionProjection(db, {
        captureId: finalizedCaptureId,
        sourceId,
        session: parsedCapture.session,
        messages: parsedCapture.messages,
        artifacts: parsedCapture.artifacts,
        rawRecords: parsedCapture.rawRecords,
        projectionAudit: {
          sourceKind: source.kind,
          sourcePath: capture.sourcePath,
          externalSessionId: capture.externalSessionId ?? null
        }
      });
    } catch (error) {
      const errorText = error instanceof Error ? error.message : String(error);
      if (failureStage === "parse" && captureId !== undefined) {
        updateCaptureFailure(db, captureId, errorText);
      }
      insertActivityEvent(db, {
        eventType: "capture_failed",
        objectType: "capture",
        objectId: captureId,
        payload: {
          sourceKind: source.kind,
          sourcePath: capture.sourcePath,
          externalSessionId: capture.externalSessionId ?? null,
          stage: failureStage,
          errorText
        }
      });
      imported.push({
        sourcePath: capture.sourcePath,
        externalSessionId: capture.externalSessionId,
        rawSha256: snapshot.rawSha256,
        skipped: false,
        status: "failed",
        errorText
      });
      failedEntries.push({
        sourceKind: source.kind,
        sourcePath: capture.sourcePath,
        errorText
      });
      failedCaptures += 1;
      continue;
    }

    importedCaptures += 1;
    imported.push({
      sourcePath: capture.sourcePath,
      externalSessionId: capture.externalSessionId,
      rawSha256: snapshot.rawSha256,
      skipped: false,
      status: "imported"
    });
  }

  return {
    importedCaptures,
    skippedCaptures,
    failedCaptures,
    captures: imported,
    failedEntries
  };
}

export function runImport(options: RunImportOptions = {}): ImportReport {
  const distillDb = openDistillDatabase();
  try {
    const sourceSummaries: ImportReport["sourceSummaries"] = [];
    const failedEntries: ImportReport["failedEntries"] = [];
    const captures: ImportedCapture[] = [];

    for (const connector of sourceConnectors) {
      let source: DiscoveredSource;

      try {
        source = connector.detect();
      } catch (error) {
        const errorText = error instanceof Error ? error.message : String(error);
        insertSyncFailureAuditEvent(distillDb.db, {
          syncJobId: options.syncJobId,
          syncReason: options.syncReason,
          sourceKind: connector.kind,
          stage: "detect",
          sourcePath: connector.kind,
          errorText
        });
        console.warn(`[import] Skipping ${connector.kind} detection: ${errorText}`);
        sourceSummaries.push({
          kind: connector.kind,
          discoveredCaptures: 0,
          importedCaptures: 0,
          skippedCaptures: 0,
          failedCaptures: 0
        });
        failedEntries.push({
          sourceKind: connector.kind,
          sourcePath: connector.kind,
          errorText
        });
        continue;
      }

      const sourceId = upsertSource(distillDb.db, source);

      let discoveredCaptures: DiscoveredCapture[];

      try {
        discoveredCaptures = connector.discoverCaptures().sort((a, b) =>
          (a.sourceModifiedAt ?? "").localeCompare(b.sourceModifiedAt ?? "")
        );
      } catch (error) {
        const errorText = error instanceof Error ? error.message : String(error);
        insertSyncFailureAuditEvent(distillDb.db, {
          syncJobId: options.syncJobId,
          syncReason: options.syncReason,
          sourceKind: source.kind,
          stage: "discover",
          sourcePath: source.dataRoot ?? connector.kind,
          errorText
        });
        console.warn(`[import] Skipping ${connector.kind} discovery: ${errorText}`);
        sourceSummaries.push({
          kind: source.kind,
          discoveredCaptures: 0,
          importedCaptures: 0,
          skippedCaptures: 0,
          failedCaptures: 0
        });
        failedEntries.push({
          sourceKind: source.kind,
          sourcePath: source.dataRoot ?? connector.kind,
          errorText
        });
        continue;
      }

      const result = importSourceCaptures(distillDb.db, connector, source, sourceId, discoveredCaptures);

      sourceSummaries.push({
        kind: source.kind,
        discoveredCaptures: discoveredCaptures.length,
        importedCaptures: result.importedCaptures,
        skippedCaptures: result.skippedCaptures,
        failedCaptures: result.failedCaptures
      });

      captures.push(...result.captures);
      failedEntries.push(...result.failedEntries);
    }

    return {
      importedAt: new Date().toISOString(),
      databasePath: distillDb.databasePath,
      distillHome: getDistillHome(),
      sourceSummaries,
      failedEntries,
      captures
    };
  } finally {
    distillDb.close();
  }
}
