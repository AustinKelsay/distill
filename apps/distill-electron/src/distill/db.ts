import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { ensureDirectory, getTextSha1 } from "./fs";
import { getDistillDatabasePath, getDistillHome } from "./paths";
import { readCaptureContentText } from "./raw_capture";
import {
  CaptureContentRef,
  CaptureStatus,
  DiscoveredSource,
  NormalizedArtifact,
  NormalizedMessage,
  NormalizedSession,
  ParsedCaptureRecord
} from "../shared/types";

type SourceRow = {
  id: number;
};

type CapturePayload = {
  sourceKind?: string;
  metadata?: Record<string, unknown>;
  contentRef?: CaptureContentRef;
};

type CaptureStorageRow = {
  raw_payload_json: string | null;
};

type TableInfoRow = {
  name: string;
  notnull: number;
};

type ProjectionMessageLinkIndex = {
  messageIdsBySourceLine: Map<number, number>;
  messageIdsByExternalMessageId: Map<string, number>;
};

export type DistillDatabase = {
  db: DatabaseSync;
  databasePath: string;
  close: () => void;
};

export type SessionProjectionInput = {
  captureId: number;
  sourceId: number;
  session: NormalizedSession;
  messages: NormalizedMessage[];
  artifacts: NormalizedArtifact[];
  rawRecords: ParsedCaptureRecord[];
  projectionAudit: {
    sourceKind: string;
    sourcePath: string;
    externalSessionId?: string | null;
  };
};

export type SessionProjectionResult = {
  sessionId: number;
  messageCount: number;
  artifactCount: number;
};

type NonPromise<T> = T extends PromiseLike<unknown> ? never : T;

function loadSchema(): string {
  const schemaPath = path.resolve(process.cwd(), "schema.sql");
  return fs.readFileSync(schemaPath, "utf8");
}

function migrateLegacySchema(db: DatabaseSync): void {
  const activityEventColumns = db
    .prepare("PRAGMA table_info(activity_events)")
    .all() as TableInfoRow[];
  const objectIdColumn = activityEventColumns.find((column) => column.name === "object_id");
  const needsActivityEventMigration = Boolean(objectIdColumn && objectIdColumn.notnull !== 0);
  const needsArtifactMessageIdMigration = !tableHasColumn(db, "artifacts", "message_id");
  const hasLegacyFailedCaptureStatus = Boolean(
    db.prepare("SELECT 1 FROM captures WHERE status = 'failed' LIMIT 1").get() as { 1: number } | undefined
  );

  if (!needsActivityEventMigration && !needsArtifactMessageIdMigration && !hasLegacyFailedCaptureStatus) {
    return;
  }

  db.exec("BEGIN");
  try {
    if (needsArtifactMessageIdMigration) {
      db.exec("ALTER TABLE artifacts ADD COLUMN message_id INTEGER REFERENCES messages(id) ON DELETE CASCADE");
    }

    if (hasLegacyFailedCaptureStatus) {
      db.exec("UPDATE captures SET status = 'failed_parse' WHERE status = 'failed'");
    }

    if (needsActivityEventMigration) {
      // Normalize legacy sentinel values while making snapshot-stage failures nullable.
      db.exec("DROP TABLE IF EXISTS activity_events__new");
      db.exec(`
        CREATE TABLE activity_events__new (
          id INTEGER PRIMARY KEY,
          event_type TEXT NOT NULL,
          object_type TEXT NOT NULL,
          object_id INTEGER,
          session_id INTEGER REFERENCES sessions(id) ON DELETE CASCADE,
          payload_json TEXT NOT NULL DEFAULT '{}',
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
      `);
      db.exec(`
        INSERT INTO activity_events__new (
          id,
          event_type,
          object_type,
          object_id,
          session_id,
          payload_json,
          created_at
        )
        SELECT
          id,
          event_type,
          object_type,
          NULLIF(object_id, 0),
          session_id,
          payload_json,
          created_at
        FROM activity_events
      `);
      db.exec("DROP TABLE activity_events");
      db.exec("ALTER TABLE activity_events__new RENAME TO activity_events");
      db.exec(`
        CREATE INDEX IF NOT EXISTS idx_activity_events_created
          ON activity_events(created_at DESC)
      `);
      db.exec(`
        CREATE INDEX IF NOT EXISTS idx_activity_events_session
          ON activity_events(session_id, created_at DESC)
      `);
    }

    db.exec("COMMIT");
  } catch (error) {
    try {
      db.exec("ROLLBACK");
    } catch {
      // Preserve the original migration error below.
    }

    throw error;
  }
}

function tableHasColumn(db: DatabaseSync, tableName: string, columnName: string): boolean {
  const columns = db
    .prepare(`PRAGMA table_info(${tableName})`)
    .all() as TableInfoRow[];

  return columns.some((column) => column.name === columnName);
}

function backfillArtifactMessageLinks(db: DatabaseSync): void {
  if (!tableHasColumn(db, "artifacts", "message_id")) {
    return;
  }

  db.prepare(`
    UPDATE artifacts
    SET message_id = (
      SELECT m.id
      FROM messages m
      WHERE m.session_id = artifacts.session_id
      AND m.capture_record_id = artifacts.capture_record_id
      ORDER BY m.ordinal DESC
      LIMIT 1
    )
    WHERE message_id IS NULL
    AND session_id IS NOT NULL
    AND capture_record_id IS NOT NULL
    AND EXISTS (
      SELECT 1
      FROM messages m
      WHERE m.session_id = artifacts.session_id
      AND m.capture_record_id = artifacts.capture_record_id
    )
  `).run();
}

export function openDistillDatabase(): DistillDatabase {
  const distillHome = getDistillHome();
  ensureDirectory(distillHome);
  ensureDirectory(path.join(distillHome, "blobs"));
  ensureDirectory(path.join(distillHome, "imports"));
  ensureDirectory(path.join(distillHome, "exports"));

  const databasePath = getDistillDatabasePath();
  const db = new DatabaseSync(databasePath);
  db.exec(loadSchema());
  migrateLegacySchema(db);
  backfillArtifactMessageLinks(db);
  db.exec("PRAGMA journal_mode = WAL;");
  db.exec("PRAGMA foreign_keys = ON;");

  return {
    db,
    databasePath,
    close: () => db.close()
  };
}

export function runInTransaction<T>(db: DatabaseSync, fn: () => NonPromise<T>): NonPromise<T> {
  type PromiseLikeResult = {
    then: (...args: unknown[]) => unknown;
  };

  if (fn.constructor?.name === "AsyncFunction") {
    throw new Error("runInTransaction does not support async functions");
  }

  let transactionOpen = false;

  try {
    db.exec("BEGIN");
    transactionOpen = true;

    const result = fn();
    if (
      (typeof result === "object" || typeof result === "function")
      && result !== null
      && "then" in result
      && typeof (result as PromiseLikeResult).then === "function"
    ) {
      throw new Error("runInTransaction does not support async functions");
    }

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

export function upsertSource(db: DatabaseSync, source: DiscoveredSource): number {
  const statement = db.prepare(`
    INSERT INTO sources (
      kind,
      display_name,
      executable_path,
      data_root,
      install_status,
      detected_at,
      metadata_json,
      updated_at
    ) VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
    ON CONFLICT(kind) DO UPDATE SET
      display_name = excluded.display_name,
      executable_path = excluded.executable_path,
      data_root = excluded.data_root,
      install_status = excluded.install_status,
      detected_at = excluded.detected_at,
      metadata_json = excluded.metadata_json,
      updated_at = CURRENT_TIMESTAMP
    RETURNING id
  `);

  const row = statement.get(
    source.kind,
    source.displayName,
    source.executablePath ?? null,
    source.dataRoot ?? null,
    source.installStatus,
    new Date().toISOString(),
    JSON.stringify({
      checks: source.checks,
      ...source.metadata
    })
  ) as SourceRow;

  return row.id;
}

export function hasCapture(
  db: DatabaseSync,
  sourceId: number,
  sourcePath: string,
  rawSha256: string
): boolean {
  const row = db
    .prepare(
      "SELECT 1 FROM captures WHERE source_id = ? AND source_path = ? AND raw_sha256 = ? LIMIT 1"
    )
    .get(sourceId, sourcePath, rawSha256) as { 1: number } | undefined;

  return Boolean(row);
}

export function findCapture(
  db: DatabaseSync,
  sourceId: number,
  sourcePath: string,
  rawSha256: string
): { id: number; status: CaptureStatus } | undefined {
  return db
    .prepare(
      "SELECT id, status FROM captures WHERE source_id = ? AND source_path = ? AND raw_sha256 = ? LIMIT 1"
    )
    .get(sourceId, sourcePath, rawSha256) as { id: number; status: CaptureStatus } | undefined;
}

export function updateCaptureStatus(db: DatabaseSync, captureId: number, status: CaptureStatus): void {
  db.prepare("UPDATE captures SET status = ?, error_text = NULL WHERE id = ?").run(status, captureId);
}

export function updateCaptureFailure(db: DatabaseSync, captureId: number, errorText: string): void {
  db.prepare("UPDATE captures SET status = ?, error_text = ? WHERE id = ?").run("failed_parse", errorText, captureId);
}

export function insertCaptureRecords(
  db: DatabaseSync,
  captureId: number,
  records: ParsedCaptureRecord[]
): Map<number, number> {
  db.prepare("DELETE FROM capture_records WHERE capture_id = ?").run(captureId);

  const statement = db.prepare(`
    INSERT INTO capture_records (
      capture_id,
      line_no,
      record_type,
      record_timestamp,
      provider_message_id,
      parent_provider_message_id,
      role,
      is_meta,
      content_text,
      content_json,
      metadata_json
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    RETURNING id
  `);

  const lineToRecordId = new Map<number, number>();

  for (const record of records) {
    const row = statement.get(
      captureId,
      record.lineNo,
      record.recordType,
      record.recordTimestamp ?? null,
      record.providerMessageId ?? null,
      record.parentProviderMessageId ?? null,
      record.role ?? null,
      record.isMeta ? 1 : 0,
      record.contentText ?? null,
      JSON.stringify(record.contentJson),
      JSON.stringify(record.metadata)
    ) as { id: number };

    lineToRecordId.set(record.lineNo, row.id);
  }

  return lineToRecordId;
}

export function upsertSession(
  db: DatabaseSync,
  sourceId: number,
  session: NormalizedSession,
  messageCount: number
): number {
  const statement = db.prepare(`
    INSERT INTO sessions (
      source_id,
      external_session_id,
      title,
      project_path,
      source_url,
      model,
      model_provider,
      cli_version,
      git_branch,
      started_at,
      updated_at,
      message_count,
      raw_capture_count,
      summary,
      metadata_json,
      updated_recorded_at
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, CURRENT_TIMESTAMP)
    ON CONFLICT(source_id, external_session_id) DO UPDATE SET
      title = COALESCE(excluded.title, sessions.title),
      project_path = COALESCE(excluded.project_path, sessions.project_path),
      source_url = COALESCE(excluded.source_url, sessions.source_url),
      model = COALESCE(excluded.model, sessions.model),
      model_provider = COALESCE(excluded.model_provider, sessions.model_provider),
      cli_version = COALESCE(excluded.cli_version, sessions.cli_version),
      git_branch = COALESCE(excluded.git_branch, sessions.git_branch),
      started_at = COALESCE(sessions.started_at, excluded.started_at),
      updated_at = COALESCE(excluded.updated_at, sessions.updated_at),
      message_count = excluded.message_count,
      raw_capture_count = sessions.raw_capture_count + 1,
      summary = COALESCE(excluded.summary, sessions.summary),
      metadata_json = excluded.metadata_json,
      updated_recorded_at = CURRENT_TIMESTAMP
    RETURNING id
  `);

  const row = statement.get(
    sourceId,
    session.externalSessionId,
    session.title ?? null,
    session.projectPath ?? null,
    session.sourceUrl ?? null,
    session.model ?? null,
    session.modelProvider ?? null,
    session.cliVersion ?? null,
    session.gitBranch ?? null,
    session.startedAt ?? null,
    session.updatedAt ?? null,
    messageCount,
    session.summary ?? null,
    JSON.stringify(session.metadata)
  ) as { id: number };

  return row.id;
}

export function replaceSessionMessages(
  db: DatabaseSync,
  sessionId: number,
  messages: NormalizedMessage[],
  captureRecordIdsByLine: Map<number, number>
): ProjectionMessageLinkIndex {
  db.prepare("DELETE FROM messages WHERE session_id = ?").run(sessionId);

  const insert = db.prepare(`
    INSERT INTO messages (
      session_id,
      capture_record_id,
      external_message_id,
      parent_external_message_id,
      ordinal,
      role,
      text,
      text_hash,
      created_at,
      message_kind,
      metadata_json
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    RETURNING id
  `);
  const messageLinks: ProjectionMessageLinkIndex = {
    messageIdsBySourceLine: new Map(),
    messageIdsByExternalMessageId: new Map()
  };

  messages.forEach((message, index) => {
    const row = insert.get(
      sessionId,
      captureRecordIdsByLine.get(message.sourceLineNo) ?? null,
      message.externalMessageId ?? null,
      message.parentExternalMessageId ?? null,
      index + 1,
      message.role,
      message.text,
      getTextSha1(message.text),
      message.createdAt ?? null,
      message.messageKind,
      JSON.stringify(message.metadata)
    ) as { id: number };

    messageLinks.messageIdsBySourceLine.set(message.sourceLineNo, row.id);
    if (message.externalMessageId) {
      messageLinks.messageIdsByExternalMessageId.set(message.externalMessageId, row.id);
    }
  });

  return messageLinks;
}

function resolveArtifactMessageId(
  artifact: NormalizedArtifact,
  messageLinks: ProjectionMessageLinkIndex
): number | null {
  const linkedByExternalMessageId =
    artifact.externalMessageId ? messageLinks.messageIdsByExternalMessageId.get(artifact.externalMessageId) : undefined;
  if (linkedByExternalMessageId) {
    return linkedByExternalMessageId;
  }

  return messageLinks.messageIdsBySourceLine.get(artifact.sourceLineNo) ?? null;
}

export function replaceSessionArtifacts(
  db: DatabaseSync,
  sessionId: number,
  artifacts: NormalizedArtifact[],
  captureRecordIdsByLine: Map<number, number>,
  messageLinks: ProjectionMessageLinkIndex
): void {
  db.prepare("DELETE FROM artifacts WHERE session_id = ?").run(sessionId);

  const insert = db.prepare(`
    INSERT INTO artifacts (
      session_id,
      message_id,
      capture_record_id,
      kind,
      mime_type,
      metadata_json
    ) VALUES (?, ?, ?, ?, ?, ?)
  `);

  for (const artifact of artifacts) {
    insert.run(
      sessionId,
      resolveArtifactMessageId(artifact, messageLinks),
      captureRecordIdsByLine.get(artifact.sourceLineNo) ?? null,
      artifact.kind,
      artifact.mimeType ?? null,
      JSON.stringify(artifact.payload)
    );
  }
}

export function replaceSessionProjection(
  db: DatabaseSync,
  input: SessionProjectionInput
): SessionProjectionResult {
  return runInTransaction(db, () => {
    const captureRecordIdsByLine = insertCaptureRecords(db, input.captureId, input.rawRecords);
    const sessionId = upsertSession(db, input.sourceId, input.session, input.messages.length);
    const messageLinks = replaceSessionMessages(db, sessionId, input.messages, captureRecordIdsByLine);

    replaceSessionArtifacts(db, sessionId, input.artifacts, captureRecordIdsByLine, messageLinks);
    updateCaptureStatus(db, input.captureId, "normalized");
    insertActivityEvent(db, {
      eventType: "projection_replaced",
      objectType: "session",
      objectId: sessionId,
      sessionId,
      payload: {
        captureId: input.captureId,
        sourceKind: input.projectionAudit.sourceKind,
        sourcePath: input.projectionAudit.sourcePath,
        externalSessionId: input.projectionAudit.externalSessionId ?? null,
        messageCount: input.messages.length,
        artifactCount: input.artifacts.length
      }
    });

    return {
      sessionId,
      messageCount: input.messages.length,
      artifactCount: input.artifacts.length
    };
  });
}

function parseCapturePayload(rawPayloadJson: string | null): CapturePayload {
  if (!rawPayloadJson) {
    return {};
  }

  try {
    const payload = JSON.parse(rawPayloadJson) as CapturePayload;
    return payload && typeof payload === "object" ? payload : {};
  } catch {
    return {};
  }
}

export function encodeCapturePayload(
  sourceKind: string,
  metadata: Record<string, unknown>,
  contentRef: CaptureContentRef
): string {
  return JSON.stringify({
    sourceKind,
    metadata,
    contentRef
  });
}

export function getCaptureContentRef(db: DatabaseSync, captureId: number): CaptureContentRef | undefined {
  const row = db
    .prepare(`
      SELECT raw_payload_json
      FROM captures
      WHERE id = ?
      LIMIT 1
    `)
    .get(captureId) as CaptureStorageRow | undefined;

  if (!row) {
    return undefined;
  }

  const payload = parseCapturePayload(row.raw_payload_json);
  if (payload.contentRef) {
    return payload.contentRef;
  }

  return undefined;
}

export function readCaptureText(db: DatabaseSync, captureId: number): string | undefined {
  const contentRef = getCaptureContentRef(db, captureId);
  if (!contentRef) {
    return undefined;
  }

  return readCaptureContentText(contentRef);
}

export function insertActivityEvent(
  db: DatabaseSync,
  input: {
    eventType: string;
    objectType: string;
    objectId?: number | null;
    sessionId?: number | null;
    payload: Record<string, unknown>;
  }
): void {
  db.prepare(`
    INSERT INTO activity_events (
      event_type,
      object_type,
      object_id,
      session_id,
      payload_json
    ) VALUES (?, ?, ?, ?, ?)
  `).run(
    input.eventType,
    input.objectType,
    input.objectId ?? null,
    input.sessionId ?? null,
    JSON.stringify(input.payload)
  );
}
