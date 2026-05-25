import fs from "node:fs";
import path from "node:path";
import { canExportSessionToDataset, ensureDefaultLabels } from "./curation";
import { openDistillDatabase } from "./db";
import { ensureDirectory } from "./fs";
import { getDistillHome } from "./paths";
import {
  DatasetExportTarget,
  ExportMessageRecord,
  ExportReport,
  ExportSessionRecord
} from "../shared/types";

type ExportSessionRow = {
  id: number;
  source_kind: ExportSessionRecord["source"];
  external_session_id: string;
  title: string | null;
  project_path: string | null;
  updated_at: string | null;
  started_at: string | null;
  source_url: string | null;
  model: string | null;
  git_branch: string | null;
  summary: string | null;
  metadata_json: string | null;
};

type ExportMessageRow = {
  ordinal: number;
  role: string;
  text: string;
  created_at: string | null;
  message_kind: "text" | "meta";
  metadata_json: string | null;
};

const DATASET_EXPORT_TARGETS = new Set<DatasetExportTarget>(["train", "holdout"]);

function makeSafeStem(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9_-]+/g, "-");
}

function parseJsonObject(jsonText: string | null | undefined): Record<string, unknown> {
  try {
    const parsed = JSON.parse(jsonText ?? "") as Record<string, unknown>;
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function buildTurnPairs(messages: ExportMessageRow[]): Array<{
  user: string;
  assistant: string;
}> {
  const pairs: Array<{ user: string; assistant: string }> = [];
  let pendingUser: string | null = null;

  for (const message of messages) {
    if (message.role === "user") {
      pendingUser = message.text;
      continue;
    }

    if (message.role === "assistant" && message.message_kind === "meta") {
      continue;
    }

    if (message.role === "assistant" && pendingUser) {
      pairs.push({
        user: pendingUser,
        assistant: message.text
      });
      pendingUser = null;
    }
  }

  return pairs;
}

function normalizeDatasetTarget(dataset: string): DatasetExportTarget {
  const normalized = dataset.trim().toLowerCase() as DatasetExportTarget;

  if (!DATASET_EXPORT_TARGETS.has(normalized)) {
    throw new Error("Dataset must be one of: train, holdout");
  }

  return normalized;
}

export function exportApprovedSessions(dataset: string): ExportReport {
  const normalizedDataset = normalizeDatasetTarget(dataset);

  ensureDefaultLabels();

  const exportedAt = new Date().toISOString();
  const distillHome = getDistillHome();
  const exportsDir = path.join(distillHome, "exports");
  ensureDirectory(exportsDir);

  const timestampStem = exportedAt.replace(/[:.]/g, "-");
  const outputPath = path.join(exportsDir, `${makeSafeStem(normalizedDataset)}-sessions-${timestampStem}.jsonl`);
  const tempOutputPath = `${outputPath}.tmp`;

  const distillDb = openDistillDatabase();
  try {
    const sessionRows = distillDb.db
      .prepare(`
        SELECT
          s.id,
          so.kind AS source_kind,
          s.external_session_id,
          s.title,
          s.project_path,
          s.updated_at,
          s.started_at,
          s.source_url,
          s.model,
          s.git_branch,
          s.summary,
          s.metadata_json
        FROM sessions s
        JOIN sources so ON so.id = s.source_id
        JOIN label_assignments la ON la.object_type = 'session' AND la.object_id = s.id
        JOIN labels l ON l.id = la.label_id
        WHERE l.name = ?
        ORDER BY COALESCE(s.updated_at, s.updated_recorded_at) DESC
      `)
      .all(normalizedDataset) as ExportSessionRow[];

    const lines: string[] = [];

    for (const session of sessionRows) {
      const labels = distillDb.db
        .prepare(`
          SELECT l.name
          FROM label_assignments la
          JOIN labels l ON l.id = la.label_id
          WHERE la.object_type = 'session'
          AND la.object_id = ?
          ORDER BY l.name ASC
        `)
        .all(session.id) as Array<{ name: string }>;
      const labelNames = labels.map((entry) => entry.name);

      if (!canExportSessionToDataset(labelNames, normalizedDataset)) {
        continue;
      }

      const messages = distillDb.db
        .prepare(`
          SELECT ordinal, role, text, created_at, message_kind, metadata_json
          FROM messages
          WHERE session_id = ?
          ORDER BY ordinal ASC
        `)
        .all(session.id) as ExportMessageRow[];

      const tags = distillDb.db
        .prepare(`
          SELECT t.name
          FROM tag_assignments ta
          JOIN tags t ON t.id = ta.tag_id
          WHERE ta.object_type = 'session'
          AND ta.object_id = ?
          ORDER BY t.name ASC
        `)
        .all(session.id) as Array<{ name: string }>;

      const messageRecords: ExportMessageRecord[] = messages.map((message) => ({
        ordinal: message.ordinal,
        role: message.role,
        text: message.text,
        created_at: message.created_at,
        message_kind: message.message_kind,
        metadata: parseJsonObject(message.metadata_json)
      }));

      const record: ExportSessionRecord = {
        exported_at: exportedAt,
        source: session.source_kind,
        external_session_id: session.external_session_id,
        title: session.title,
        project_path: session.project_path,
        updated_at: session.updated_at,
        started_at: session.started_at,
        source_url: session.source_url,
        model: session.model,
        git_branch: session.git_branch,
        summary: session.summary,
        metadata: parseJsonObject(session.metadata_json),
        labels: labelNames,
        tags: tags.map((tag) => tag.name),
        messages: messageRecords,
        turn_pairs: buildTurnPairs(messages)
      };

      lines.push(
        JSON.stringify(record)
      );
    }

    fs.writeFileSync(tempOutputPath, lines.join("\n") + (lines.length ? "\n" : ""));

    let transactionOpen = false;

    try {
      distillDb.db.exec("BEGIN");
      transactionOpen = true;

      const exportInsert = distillDb.db
        .prepare(`
          INSERT INTO exports (export_type, label_filter, output_path, record_count, metadata_json)
          VALUES ('jsonl', ?, ?, ?, ?)
        `)
        .run(
          normalizedDataset,
          outputPath,
          lines.length,
          JSON.stringify({
            exportedAt,
            dataset: normalizedDataset
          })
        );

      distillDb.db
        .prepare(`
          INSERT INTO activity_events (
            event_type,
            object_type,
            object_id,
            payload_json
          ) VALUES (?, ?, ?, ?)
        `)
        .run(
          "export_written",
          "export",
          Number(exportInsert.lastInsertRowid),
          JSON.stringify({
            dataset: normalizedDataset,
            outputPath,
            recordCount: lines.length,
            exportedAt
          })
        );

      distillDb.db.exec("COMMIT");
      transactionOpen = false;
      fs.renameSync(tempOutputPath, outputPath);
    } catch (error) {
      if (transactionOpen) {
        try {
          distillDb.db.exec("ROLLBACK");
        } catch {
          // Preserve the original export failure below.
        }
      }

      try {
        fs.unlinkSync(tempOutputPath);
      } catch {
        // Ignore cleanup failures so the export error remains primary.
      }

      throw error;
    }

    return {
      exportedAt,
      dataset: normalizedDataset,
      outputPath,
      recordCount: lines.length
    };
  } finally {
    distillDb.close();
  }
}
