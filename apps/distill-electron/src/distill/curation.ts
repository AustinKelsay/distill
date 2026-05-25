import { DatabaseSync } from "node:sqlite";
import { insertActivityEvent, openDistillDatabase, runInTransaction } from "./db";
import { DatasetExportTarget, SessionWorkflowState } from "../shared/types";

const DEFAULT_LABELS = ["train", "holdout", "exclude", "sensitive", "favorite"] as const;
const DATASET_LABELS = ["train", "holdout", "exclude"] as const;
const REVIEW_LABELS = ["exclude", "sensitive"] as const;
const DATASET_EXPORT_TARGETS = ["train", "holdout"] as const;

type ManualLabelAssignmentRow = {
  id: number;
  label_id: number;
  label_name: string;
};

function sessionExists(db: DatabaseSync, sessionId: number): boolean {
  const row = db
    .prepare("SELECT 1 FROM sessions WHERE id = ? LIMIT 1")
    .get(sessionId) as { 1: number } | undefined;

  return Boolean(row);
}

function ensureDefaultLabelsInDb(db: DatabaseSync): void {
  const insert = db.prepare(`
    INSERT INTO labels (name, scope)
    VALUES (?, 'session')
    ON CONFLICT(name) DO NOTHING
  `);

  for (const label of DEFAULT_LABELS) {
    insert.run(label);
  }
}

export function ensureDefaultLabels(): void {
  const distillDb = openDistillDatabase();
  try {
    ensureDefaultLabelsInDb(distillDb.db);
  } finally {
    distillDb.close();
  }
}

export function getDefaultLabelNames(): string[] {
  return [...DEFAULT_LABELS];
}

export function getDatasetExportTargets(): DatasetExportTarget[] {
  return [...DATASET_EXPORT_TARGETS];
}

function normalizeLabels(labelNames: Iterable<string>): Set<string> {
  const labels = new Set<string>();

  for (const label of labelNames) {
    const normalized = label.trim().toLowerCase();
    if (normalized) {
      labels.add(normalized);
    }
  }

  return labels;
}

export function isDatasetLabel(labelName: string): boolean {
  return DATASET_LABELS.includes(labelName.trim().toLowerCase() as typeof DATASET_LABELS[number]);
}

function getConflictingDatasetLabels(labelName: string): string[] {
  const normalized = labelName.trim().toLowerCase();
  if (!isDatasetLabel(normalized)) {
    return [];
  }

  return DATASET_LABELS.filter((label) => label !== normalized);
}

export function deriveSessionWorkflowState(labelNames: Iterable<string>): SessionWorkflowState {
  const labels = normalizeLabels(labelNames);
  const hasTrain = labels.has("train");
  const hasHoldout = labels.has("holdout");

  if (REVIEW_LABELS.some((label) => labels.has(label))) {
    return "needs_review";
  }

  if (hasTrain && hasHoldout) {
    return "needs_review";
  }

  if (hasTrain) {
    return "train_ready";
  }

  if (hasHoldout) {
    return "holdout_ready";
  }

  if (labels.has("favorite")) {
    return "favorite";
  }

  return "neutral";
}

export function canExportSessionToDataset(
  labelNames: Iterable<string>,
  dataset: DatasetExportTarget
): boolean {
  const workflowState = deriveSessionWorkflowState(labelNames);

  return (workflowState === "train_ready" && dataset === "train")
    || (workflowState === "holdout_ready" && dataset === "holdout");
}

function insertLabelToggledAudit(
  db: DatabaseSync,
  sessionId: number,
  labelId: number,
  labelName: string,
  enabled: boolean
): void {
  insertActivityEvent(db, {
    eventType: "label_toggled",
    objectType: "session",
    objectId: sessionId,
    sessionId,
    payload: {
      labelId,
      labelName,
      origin: "manual",
      enabled
    }
  });
}

export function addSessionTag(sessionId: number, tagName: string): void {
  const normalized = tagName.trim().toLowerCase();
  if (!normalized) {
    return;
  }

  const distillDb = openDistillDatabase();
  try {
    runInTransaction(distillDb.db, () => {
      if (!sessionExists(distillDb.db, sessionId)) {
        return;
      }

      const tagRow = distillDb.db
        .prepare(`
          INSERT INTO tags (name, kind)
          VALUES (?, 'manual')
          ON CONFLICT(name) DO UPDATE SET name = excluded.name
          RETURNING id, name
        `)
        .get(normalized) as { id: number; name: string };

      const assignment = distillDb.db
        .prepare(`
          INSERT INTO tag_assignments (object_type, object_id, tag_id, origin)
          VALUES ('session', ?, ?, 'manual')
          ON CONFLICT(object_type, object_id, tag_id, origin) DO NOTHING
          RETURNING id
        `)
        .get(sessionId, tagRow.id) as { id: number } | undefined;

      if (!assignment) {
        return;
      }

      insertActivityEvent(distillDb.db, {
        eventType: "tag_added",
        objectType: "session",
        objectId: sessionId,
        sessionId,
        payload: {
          tagId: tagRow.id,
          tagName: tagRow.name,
          origin: "manual"
        }
      });
    });
  } finally {
    distillDb.close();
  }
}

export function removeSessionTag(sessionId: number, tagId: number): void {
  const distillDb = openDistillDatabase();
  try {
    runInTransaction(distillDb.db, () => {
      if (!sessionExists(distillDb.db, sessionId)) {
        return;
      }

      const assignment = distillDb.db
        .prepare(`
          SELECT ta.id, t.name
          FROM tag_assignments ta
          JOIN tags t ON t.id = ta.tag_id
          WHERE ta.object_type = 'session'
          AND ta.object_id = ?
          AND ta.tag_id = ?
          AND ta.origin = 'manual'
          LIMIT 1
        `)
        .get(sessionId, tagId) as { id: number; name: string } | undefined;

      if (!assignment) {
        return;
      }

      const result = distillDb.db.prepare("DELETE FROM tag_assignments WHERE id = ?").run(assignment.id);
      if (result.changes === 0) {
        return;
      }

      insertActivityEvent(distillDb.db, {
        eventType: "tag_removed",
        objectType: "session",
        objectId: sessionId,
        sessionId,
        payload: {
          tagId,
          tagName: assignment.name,
          origin: "manual"
        }
      });
    });
  } finally {
    distillDb.close();
  }
}

export function toggleSessionLabel(sessionId: number, labelName: string): void {
  const normalized = labelName.trim().toLowerCase();
  if (!normalized) {
    return;
  }

  const distillDb = openDistillDatabase();
  try {
    runInTransaction(distillDb.db, () => {
      if (!sessionExists(distillDb.db, sessionId)) {
        return;
      }

      ensureDefaultLabelsInDb(distillDb.db);

      const label = distillDb.db
        .prepare("SELECT id, name FROM labels WHERE name = ? LIMIT 1")
        .get(normalized) as { id: number; name: string } | undefined;

      if (!label) {
        return;
      }

      const existing = distillDb.db
        .prepare(`
          SELECT id
          FROM label_assignments
          WHERE object_type = 'session'
          AND object_id = ?
          AND label_id = ?
          AND origin = 'manual'
          LIMIT 1
        `)
        .get(sessionId, label.id) as { id: number } | undefined;

      if (existing) {
        const deleted = distillDb.db.prepare("DELETE FROM label_assignments WHERE id = ?").run(existing.id);
        if (deleted.changes === 0) {
          return;
        }

        insertLabelToggledAudit(distillDb.db, sessionId, label.id, label.name, false);
        return;
      }

      const conflictingLabels = getConflictingDatasetLabels(label.name);
      if (conflictingLabels.length > 0) {
        const placeholders = conflictingLabels.map(() => "?").join(", ");
        const conflictingAssignments = distillDb.db.prepare(`
          SELECT la.id, l.id AS label_id, l.name AS label_name
          FROM label_assignments la
          JOIN labels l ON l.id = la.label_id
          WHERE la.object_type = 'session'
          AND la.object_id = ?
          AND la.origin = 'manual'
          AND l.name IN (${placeholders})
          ORDER BY l.name ASC
        `).all(sessionId, ...conflictingLabels) as ManualLabelAssignmentRow[];

        const deleteAssignment = distillDb.db.prepare("DELETE FROM label_assignments WHERE id = ?");

        for (const assignment of conflictingAssignments) {
          const deleted = deleteAssignment.run(assignment.id);
          if (deleted.changes === 0) {
            continue;
          }

          insertLabelToggledAudit(
            distillDb.db,
            sessionId,
            assignment.label_id,
            assignment.label_name,
            false
          );
        }
      }

      // `inserted` stays undefined when the `label_assignments` INSERT hits `ON CONFLICT`,
      // which intentionally leaves a derived-only assignment untouched and emits no audit.
      // See "toggleSessionLabel ignores derived assignments when no manual label exists".
      const inserted = distillDb.db
        .prepare(`
          INSERT INTO label_assignments (object_type, object_id, label_id, origin)
          VALUES ('session', ?, ?, 'manual')
          ON CONFLICT(object_type, object_id, label_id) DO NOTHING
          RETURNING id
        `)
        .get(sessionId, label.id) as { id: number } | undefined;

      if (!inserted) {
        return;
      }

      insertLabelToggledAudit(distillDb.db, sessionId, label.id, label.name, true);
    });
  } finally {
    distillDb.close();
  }
}
