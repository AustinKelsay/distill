import { DatabaseSync } from "node:sqlite";
import { deriveSessionWorkflowState } from "./curation";
import { openDistillDatabase } from "./db";
import { buildDoctorReport } from "./doctor";
import {
  DashboardData,
  SearchResult,
  SessionArtifact,
  SessionDetail,
  SessionDetailMessage,
  SessionLabel,
  SessionListItem,
  SessionTag
} from "../shared/types";

type SessionPreviewFields = {
  title: string | null;
  first_user_text: string | null;
  first_assistant_text: string | null;
};

type SessionListRow = SessionPreviewFields & {
  id: number;
  source_kind: SessionListItem["sourceKind"];
  project_path: string | null;
  updated_at: string | null;
  message_count: number;
  model: string | null;
  git_branch: string | null;
};

type SessionDetailRow = SessionListRow & {
  external_session_id: string;
  source_url: string | null;
  started_at: string | null;
  raw_capture_count: number;
  summary: string | null;
  metadata_json: string | null;
  artifact_count: number;
};

type SessionMessageRow = {
  id: number;
  ordinal: number;
  role: string;
  text: string;
  created_at: string | null;
  message_kind: "text" | "meta";
};

type SessionTagRow = {
  id: number;
  name: string;
  kind: string;
  origin: string;
};

type SessionLabelRow = {
  id: number;
  name: string;
  scope: string;
  origin: string;
};

type SessionArtifactRow = {
  id: number;
  kind: string;
  mime_type: string | null;
  metadata_json: string;
  created_at: string | null;
  source_line_no: number | null;
  message_ordinal: number | null;
  message_role: string | null;
};

type SearchRow = {
  session_id: number;
  source_kind: SearchResult["sourceKind"];
  title: string | null;
  project_path: string | null;
  updated_at: string | null;
  first_user_text: string | null;
};

type SearchHitRow = {
  session_id: number;
  title: string | null;
  project_path: string | null;
  role: string | null;
  text: string;
};

type SessionLabelLookupRow = {
  session_id: number;
  name: string;
};

function cleanExcerpt(text: string | null | undefined, maxLength: number): string | undefined {
  const cleaned = text?.replace(/\s+/g, " ").trim();
  if (!cleaned) {
    return undefined;
  }

  return cleaned.length > maxLength ? `${cleaned.slice(0, maxLength - 1).trimEnd()}…` : cleaned;
}

function deriveSessionTitle(row: SessionPreviewFields): string {
  const directTitle = row.title?.trim();
  if (directTitle) {
    return directTitle;
  }

  const previewTitle = cleanExcerpt(row.first_user_text, 160);
  if (previewTitle) {
    return previewTitle;
  }

  return "Untitled session";
}

function deriveSessionPreview(row: SessionPreviewFields): string | undefined {
  return cleanExcerpt(row.first_assistant_text, 280) ?? cleanExcerpt(row.first_user_text, 280);
}

function normalizeSearchQuery(query: string): string {
  const tokens = query.match(/[\p{L}\p{N}_-]+/gu) ?? [];

  return tokens.map((token) => `"${token.replace(/"/g, "\"\"")}"`).join(" AND ");
}

function clampValue(value: unknown, depth = 0): unknown {
  if (depth > 4) {
    return "[truncated]";
  }

  if (typeof value === "string") {
    return value.length > 320 ? `${value.slice(0, 317)}...` : value;
  }

  if (Array.isArray(value)) {
    return value.slice(0, 12).map((entry) => clampValue(entry, depth + 1));
  }

  if (value && typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>).slice(0, 20);
    return Object.fromEntries(entries.map(([key, entry]) => [key, clampValue(entry, depth + 1)]));
  }

  return value;
}

function parseJsonObject(jsonText: string | null | undefined): Record<string, unknown> {
  try {
    const payload = JSON.parse(jsonText ?? "") as Record<string, unknown>;
    return payload && typeof payload === "object" ? payload : {};
  } catch {
    return {};
  }
}

function parseArtifactPayload(metadataJson: string): Record<string, unknown> {
  return parseJsonObject(metadataJson);
}

function payloadText(payload: Record<string, unknown>): string | undefined {
  const content = payload.content;

  if (typeof payload.text === "string" && payload.text.trim()) {
    return payload.text.trim();
  }

  if (typeof payload.error === "string" && payload.error.trim()) {
    return payload.error.trim();
  }

  if (typeof payload.output === "string" && payload.output.trim()) {
    return payload.output.trim();
  }

  if (!Array.isArray(content)) {
    return undefined;
  }

  const text = content
    .flatMap((entry) => {
      if (!entry || typeof entry !== "object") {
        return [];
      }

      const row = entry as Record<string, unknown>;
      return typeof row.text === "string" && row.text.trim() ? [row.text.trim()] : [];
    })
    .join(" ");

  return text || undefined;
}

function summarizeArtifact(row: SessionArtifactRow, payload: Record<string, unknown>): string {
  if (row.kind === "tool_call") {
    const name =
      typeof payload.name === "string" ? payload.name
      : typeof payload.tool === "string" ? payload.tool : undefined;
    return name ? `Tool call: ${name}` : "Tool call";
  }

  if (row.kind === "tool_result") {
    const text = cleanExcerpt(payloadText(payload), 140);
    return text ? `Tool result: ${text}` : "Tool result";
  }

  if (row.kind === "file") {
    const name =
      typeof payload.filename === "string" ? payload.filename
      : payload.source && typeof payload.source === "object" && typeof (payload.source as { path?: unknown }).path === "string"
        ? ((payload.source as { path: string }).path)
        : undefined;
    return name ? `File: ${name}` : "File artifact";
  }

  if (row.kind === "image") {
    return row.mime_type ? `Image: ${row.mime_type}` : "Image artifact";
  }

  return row.kind.replace(/_/g, " ");
}

function artifactPreview(payload: Record<string, unknown>): string {
  const text = cleanExcerpt(payloadText(payload), 220);
  if (text) {
    return text;
  }

  return JSON.stringify(clampValue(payload)) || "{}";
}

function mapArtifact(row: SessionArtifactRow): SessionArtifact {
  const payload = parseArtifactPayload(row.metadata_json);

  return {
    id: row.id,
    kind: row.kind,
    mimeType: row.mime_type ?? undefined,
    sourceLineNo: row.source_line_no ?? undefined,
    messageOrdinal: row.message_ordinal ?? undefined,
    messageRole: row.message_role ?? undefined,
    createdAt: row.created_at ?? undefined,
    summary: summarizeArtifact(row, payload),
    payloadPreview: artifactPreview(payload),
    payloadJson: JSON.stringify(clampValue(payload), null, 2)
  };
}

function loadSessionLabels(db: DatabaseSync, sessionIds: number[]): Map<number, string[]> {
  const labelsBySessionId = new Map<number, string[]>();

  for (const sessionId of sessionIds) {
    labelsBySessionId.set(sessionId, []);
  }

  if (sessionIds.length === 0) {
    return labelsBySessionId;
  }

  const placeholders = sessionIds.map(() => "?").join(", ");
  const rows = db.prepare(`
    SELECT la.object_id AS session_id, l.name
    FROM label_assignments la
    JOIN labels l ON l.id = la.label_id
    WHERE la.object_type = 'session'
    AND la.origin = 'manual'
    AND la.object_id IN (${placeholders})
    ORDER BY la.object_id ASC, l.name ASC
  `).all(...sessionIds) as SessionLabelLookupRow[];

  for (const row of rows) {
    labelsBySessionId.get(row.session_id)?.push(row.name);
  }

  return labelsBySessionId;
}

export function listRecentSessions(limit?: number): SessionListItem[] {
  const distillDb = openDistillDatabase();
  try {
    const statement = distillDb.db.prepare(`
        SELECT
          s.id,
          so.kind AS source_kind,
          s.title,
          s.project_path,
          s.updated_at,
          s.message_count,
          s.model,
          s.git_branch,
          (
            SELECT m.text
            FROM messages m
            WHERE m.session_id = s.id
            AND m.role = 'user'
            AND m.message_kind = 'text'
            ORDER BY m.ordinal ASC
            LIMIT 1
          ) AS first_user_text,
          (
            SELECT m.text
            FROM messages m
            WHERE m.session_id = s.id
            AND m.role = 'assistant'
            AND m.message_kind = 'text'
            ORDER BY m.ordinal ASC
            LIMIT 1
          ) AS first_assistant_text
        FROM sessions s
        JOIN sources so ON so.id = s.source_id
        ORDER BY COALESCE(s.updated_at, s.updated_recorded_at) DESC
        ${typeof limit === "number" ? "LIMIT ?" : ""}
      `);

    const rows = (
      typeof limit === "number"
        ? statement.all(limit)
        : statement.all()
    ) as SessionListRow[];
    const labelsBySessionId = loadSessionLabels(
      distillDb.db,
      rows.map((row) => row.id)
    );

    return rows.map((row) => ({
      id: row.id,
      sourceKind: row.source_kind,
      title: deriveSessionTitle(row),
      projectPath: row.project_path ?? undefined,
      updatedAt: row.updated_at ?? undefined,
      messageCount: row.message_count,
      model: row.model ?? undefined,
      gitBranch: row.git_branch ?? undefined,
      labels: labelsBySessionId.get(row.id) ?? [],
      workflowState: deriveSessionWorkflowState(labelsBySessionId.get(row.id) ?? []),
      preview: deriveSessionPreview(row)
    }));
  } finally {
    distillDb.close();
  }
}

export function getSessionDetail(sessionId: number): SessionDetail | undefined {
  const distillDb = openDistillDatabase();
  try {
    const row = distillDb.db
      .prepare(`
        SELECT
          s.id,
          so.kind AS source_kind,
          s.external_session_id,
          s.title,
          s.project_path,
          s.source_url,
          s.started_at,
          s.updated_at,
          s.message_count,
          s.raw_capture_count,
          s.model,
          s.git_branch,
          s.summary,
          s.metadata_json,
          (
            SELECT m.text
            FROM messages m
            WHERE m.session_id = s.id
            AND m.role = 'user'
            AND m.message_kind = 'text'
            ORDER BY m.ordinal ASC
            LIMIT 1
          ) AS first_user_text,
          (
            SELECT m.text
            FROM messages m
            WHERE m.session_id = s.id
            AND m.role = 'assistant'
            AND m.message_kind = 'text'
            ORDER BY m.ordinal ASC
            LIMIT 1
          ) AS first_assistant_text,
          (
            SELECT COUNT(*)
            FROM artifacts a
            WHERE a.session_id = s.id
          ) AS artifact_count
        FROM sessions s
        JOIN sources so ON so.id = s.source_id
        WHERE s.id = ?
      `)
      .get(sessionId) as SessionDetailRow | undefined;

    if (!row) {
      return undefined;
    }

    const messages = distillDb.db
      .prepare(`
        SELECT id, ordinal, role, text, created_at, message_kind
        FROM messages
        WHERE session_id = ?
        ORDER BY ordinal ASC
      `)
      .all(sessionId) as SessionMessageRow[];

    const tags = distillDb.db
      .prepare(`
        SELECT t.id, t.name, t.kind, ta.origin
        FROM tag_assignments ta
        JOIN tags t ON t.id = ta.tag_id
        WHERE ta.object_type = 'session'
        AND ta.object_id = ?
        ORDER BY t.name ASC
      `)
      .all(sessionId) as SessionTagRow[];

    const labels = distillDb.db
      .prepare(`
        SELECT l.id, l.name, l.scope, la.origin
        FROM label_assignments la
        JOIN labels l ON l.id = la.label_id
        WHERE la.object_type = 'session'
        AND la.object_id = ?
        AND la.origin = 'manual'
        ORDER BY l.name ASC
      `)
      .all(sessionId) as SessionLabelRow[];

    const artifacts = distillDb.db
      .prepare(`
        SELECT
          a.id,
          a.kind,
          a.mime_type,
          a.metadata_json,
          a.created_at,
          cr.line_no AS source_line_no,
          m.ordinal AS message_ordinal,
          m.role AS message_role
        FROM artifacts a
        LEFT JOIN capture_records cr ON cr.id = a.capture_record_id
        LEFT JOIN messages m ON m.id = a.message_id
        WHERE a.session_id = ?
        ORDER BY COALESCE(m.ordinal, 999999), COALESCE(cr.line_no, 999999), a.id
      `)
      .all(sessionId) as SessionArtifactRow[];

    return {
      id: row.id,
      sourceKind: row.source_kind,
      externalSessionId: row.external_session_id,
      title: deriveSessionTitle(row),
      projectPath: row.project_path ?? undefined,
      startedAt: row.started_at ?? undefined,
      updatedAt: row.updated_at ?? undefined,
      sourceUrl: row.source_url ?? undefined,
      messageCount: row.message_count,
      rawCaptureCount: row.raw_capture_count,
      model: row.model ?? undefined,
      gitBranch: row.git_branch ?? undefined,
      summary: row.summary ?? undefined,
      preview: deriveSessionPreview(row),
      metadata: parseJsonObject(row.metadata_json),
      artifactCount: row.artifact_count,
      tags: tags.map(
        (tag): SessionTag => ({
          id: tag.id,
          name: tag.name,
          kind: tag.kind,
          origin: tag.origin
        })
      ),
      labels: labels.map(
        (label): SessionLabel => ({
          id: label.id,
          name: label.name,
          scope: label.scope,
          origin: label.origin
        })
      ),
      artifacts: artifacts.map(mapArtifact),
      messages: messages.map(
        (message): SessionDetailMessage => ({
          id: message.id,
          ordinal: message.ordinal,
          role: message.role,
          text: message.text,
          createdAt: message.created_at ?? undefined,
          messageKind: message.message_kind
        })
      )
    };
  } finally {
    distillDb.close();
  }
}

export function searchSessions(query: string, limit?: number): SearchResult[] {
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) {
    return [];
  }

  const distillDb = openDistillDatabase();
  try {
    const totalCount = (
      distillDb.db.prepare("SELECT COUNT(*) AS count FROM sessions").get() as { count: number }
    ).count;
    const requestedLimit = limit ?? totalCount;
    const maxResults = Math.max(0, Math.min(totalCount, requestedLimit));
    if (maxResults === 0) {
      return [];
    }

    const hitLimit = Math.max(maxResults * 8, 20);

    const hitRows = distillDb.db
      .prepare(`
        SELECT
          CAST(session_id AS INTEGER) AS session_id,
          title,
          project_path,
          role,
          text
        FROM message_fts
        WHERE message_fts MATCH ?
        LIMIT ?
      `)
      .all(normalizedQuery, hitLimit) as SearchHitRow[];

    const firstHitBySession = new Map<number, SearchHitRow>();
    for (const row of hitRows) {
      if (!firstHitBySession.has(row.session_id)) {
        firstHitBySession.set(row.session_id, row);
      }
      if (firstHitBySession.size >= maxResults) {
        break;
      }
    }

    const sessionIds = [...firstHitBySession.keys()];
    if (sessionIds.length === 0) {
      return [];
    }

    const placeholders = sessionIds.map(() => "?").join(", ");
    const sessionRows = distillDb.db
      .prepare(`
        SELECT
          s.id AS session_id,
          so.kind AS source_kind,
          s.title,
          s.project_path,
          s.updated_at,
          (
            SELECT m.text
            FROM messages m
            WHERE m.session_id = s.id
            AND m.role = 'user'
            AND m.message_kind = 'text'
            ORDER BY m.ordinal ASC
            LIMIT 1
          ) AS first_user_text
        FROM sessions s
        JOIN sources so ON so.id = s.source_id
        WHERE s.id IN (${placeholders})
      `)
      .all(...sessionIds) as SearchRow[];

    const sessionRowById = new Map(sessionRows.map((row) => [row.session_id, row]));
    const labelsBySessionId = loadSessionLabels(distillDb.db, sessionIds);

    return sessionIds.flatMap((sessionId) => {
      const row = sessionRowById.get(sessionId);
      const hit = firstHitBySession.get(sessionId);
      if (!row || !hit) {
        return [];
      }

      const labels = labelsBySessionId.get(sessionId) ?? [];

      return [
        {
          sessionId: row.session_id,
          sourceKind: row.source_kind,
          title: row.title?.trim() || cleanExcerpt(row.first_user_text, 160) || "Untitled session",
          projectPath: row.project_path ?? undefined,
          updatedAt: row.updated_at ?? undefined,
          labels,
          workflowState: deriveSessionWorkflowState(labels),
          snippet: cleanExcerpt(hit.text, 280) ?? ""
        }
      ];
    });
  } finally {
    distillDb.close();
  }
}

export function getDashboardData(): DashboardData {
  return {
    doctor: buildDoctorReport(),
    sessions: listRecentSessions()
  };
}
