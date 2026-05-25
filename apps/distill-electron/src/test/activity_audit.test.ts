import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { addSessionTag, ensureDefaultLabels, removeSessionTag, toggleSessionLabel } from "../distill/curation";
import { openDistillDatabase } from "../distill/db";

function withTempDistill<T>(fn: (root: string) => T): T {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "distill-activity-"));
  const previous = process.env.DISTILL_HOME;
  process.env.DISTILL_HOME = path.join(tempRoot, ".distill");

  try {
    return fn(tempRoot);
  } finally {
    process.env.DISTILL_HOME = previous;
    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

function seedSession(sessionId: number): void {
  const distillDb = openDistillDatabase();
  try {
    distillDb.db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (1, 'claude_code', 'Claude Code', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    distillDb.db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (?, 1, ?, 'Audited session', '/tmp/demo', '2026-03-25T15:00:00Z', 1, 1, '{}')
    `).run(sessionId, `session-${sessionId}`);
  } finally {
    distillDb.close();
  }
}

test("addSessionTag audits new assignments and ignores duplicates", () => {
  withTempDistill(() => {
    seedSession(10);

    addSessionTag(10, " Distill ");
    addSessionTag(10, "distill");

    const distillDb = openDistillDatabase();
    try {
      const assignments = distillDb.db.prepare(`
        SELECT COUNT(*) AS count
        FROM tag_assignments
        WHERE object_type = 'session' AND object_id = 10
      `).get() as { count: number };
      const events = distillDb.db.prepare(`
        SELECT event_type, object_id, session_id, payload_json
        FROM activity_events
        ORDER BY id ASC
      `).all() as Array<{ event_type: string; object_id: number | null; session_id: number | null; payload_json: string }>;
      const payload = JSON.parse(events[0]?.payload_json ?? "{}") as Record<string, unknown>;

      assert.equal(assignments.count, 1);
      assert.deepEqual(events.map((event) => event.event_type), ["tag_added"]);
      assert.equal(events[0]?.object_id, 10);
      assert.equal(events[0]?.session_id, 10);
      assert.equal(payload.tagName, "distill");
      assert.equal(payload.origin, "manual");
    } finally {
      distillDb.close();
    }
  });
});

test("removeSessionTag audits actual deletions and ignores missing assignments", () => {
  withTempDistill(() => {
    seedSession(11);
    addSessionTag(11, "research");

    const seededDb = openDistillDatabase();
    const tag = seededDb.db.prepare("SELECT id FROM tags WHERE name = 'research'").get() as { id: number };
    seededDb.close();

    removeSessionTag(11, tag.id);
    removeSessionTag(11, tag.id);

    const distillDb = openDistillDatabase();
    try {
      const assignments = distillDb.db.prepare(`
        SELECT COUNT(*) AS count
        FROM tag_assignments
        WHERE object_type = 'session' AND object_id = 11
      `).get() as { count: number };
      const events = distillDb.db.prepare(`
        SELECT event_type, payload_json
        FROM activity_events
        ORDER BY id ASC
      `).all() as Array<{ event_type: string; payload_json: string }>;
      const payload = JSON.parse(events[1]?.payload_json ?? "{}") as Record<string, unknown>;

      assert.equal(assignments.count, 0);
      assert.deepEqual(events.map((event) => event.event_type), ["tag_added", "tag_removed"]);
      assert.equal(payload.tagName, "research");
      assert.equal(payload.origin, "manual");
    } finally {
      distillDb.close();
    }
  });
});

test("removeSessionTag ignores derived assignments and emits no manual removal audit", () => {
  withTempDistill(() => {
    seedSession(13);

    let tagId = 0;
    const seededDb = openDistillDatabase();
    try {
      tagId = (seededDb.db
        .prepare("INSERT INTO tags (name, kind) VALUES ('derived', 'general') RETURNING id")
        .get() as { id: number }).id;

      seededDb.db.prepare(`
        INSERT INTO tag_assignments (object_type, object_id, tag_id, origin)
        VALUES ('session', 13, ?, 'auto_rule')
      `).run(tagId);
    } finally {
      seededDb.close();
    }

    removeSessionTag(13, tagId);

    const verifyDb = openDistillDatabase();
    try {
      const assignments = verifyDb.db.prepare(`
        SELECT COUNT(*) AS count
        FROM tag_assignments
        WHERE object_type = 'session' AND object_id = 13
      `).get() as { count: number };
      const events = verifyDb.db.prepare(`
        SELECT COUNT(*) AS count
        FROM activity_events
      `).get() as { count: number };

      assert.equal(assignments.count, 1);
      assert.equal(events.count, 0);
    } finally {
      verifyDb.close();
    }
  });
});

test("toggleSessionLabel audits enable and disable transitions", () => {
  withTempDistill(() => {
    seedSession(12);

    toggleSessionLabel(12, "train");
    toggleSessionLabel(12, "train");

    const distillDb = openDistillDatabase();
    try {
      const assignments = distillDb.db.prepare(`
        SELECT COUNT(*) AS count
        FROM label_assignments
        WHERE object_type = 'session' AND object_id = 12
      `).get() as { count: number };
      const events = distillDb.db.prepare(`
        SELECT event_type, object_id, session_id, payload_json
        FROM activity_events
        ORDER BY id ASC
      `).all() as Array<{ event_type: string; object_id: number | null; session_id: number | null; payload_json: string }>;
      const enabledPayload = JSON.parse(events[0]?.payload_json ?? "{}") as Record<string, unknown>;
      const disabledPayload = JSON.parse(events[1]?.payload_json ?? "{}") as Record<string, unknown>;

      assert.equal(assignments.count, 0);
      assert.deepEqual(events.map((event) => event.event_type), ["label_toggled", "label_toggled"]);
      assert.equal(events[0]?.object_id, 12);
      assert.equal(events[0]?.session_id, 12);
      assert.equal(enabledPayload.labelName, "train");
      assert.equal(enabledPayload.origin, "manual");
      assert.equal(enabledPayload.enabled, true);
      assert.equal(disabledPayload.enabled, false);
    } finally {
      distillDb.close();
    }
  });
});

test("toggleSessionLabel removes conflicting dataset labels and audits both transitions", () => {
  withTempDistill(() => {
    seedSession(15);

    toggleSessionLabel(15, "train");
    toggleSessionLabel(15, "holdout");

    const distillDb = openDistillDatabase();
    try {
      const labels = distillDb.db.prepare(`
        SELECT l.name
        FROM label_assignments la
        JOIN labels l ON l.id = la.label_id
        WHERE la.object_type = 'session'
        AND la.object_id = 15
        ORDER BY l.name ASC
      `).all() as Array<{ name: string }>;
      const events = distillDb.db.prepare(`
        SELECT payload_json
        FROM activity_events
        ORDER BY id ASC
      `).all() as Array<{ payload_json: string }>;
      const payloads = events.map((event) => JSON.parse(event.payload_json) as Record<string, unknown>);

      assert.deepEqual(labels.map((label) => label.name), ["holdout"]);
      assert.deepEqual(
        payloads.map((payload) => ({ labelName: payload.labelName, enabled: payload.enabled })),
        [
          { labelName: "train", enabled: true },
          { labelName: "train", enabled: false },
          { labelName: "holdout", enabled: true }
        ]
      );
    } finally {
      distillDb.close();
    }
  });
});

test("toggleSessionLabel keeps orthogonal labels while review labels take priority", () => {
  withTempDistill(() => {
    seedSession(16);

    toggleSessionLabel(16, "holdout");
    toggleSessionLabel(16, "favorite");
    toggleSessionLabel(16, "sensitive");
    toggleSessionLabel(16, "exclude");

    const distillDb = openDistillDatabase();
    try {
      const labels = distillDb.db.prepare(`
        SELECT l.name
        FROM label_assignments la
        JOIN labels l ON l.id = la.label_id
        WHERE la.object_type = 'session'
        AND la.object_id = 16
        ORDER BY l.name ASC
      `).all() as Array<{ name: string }>;
      const events = distillDb.db.prepare(`
        SELECT payload_json
        FROM activity_events
        ORDER BY id ASC
      `).all() as Array<{ payload_json: string }>;
      const payloads = events.map((event) => JSON.parse(event.payload_json) as Record<string, unknown>);

      assert.deepEqual(labels.map((label) => label.name), ["exclude", "favorite", "sensitive"]);
      assert.deepEqual(
        payloads.slice(-2).map((payload) => ({ labelName: payload.labelName, enabled: payload.enabled })),
        [
          { labelName: "holdout", enabled: false },
          { labelName: "exclude", enabled: true }
        ]
      );
    } finally {
      distillDb.close();
    }
  });
});

test("toggleSessionLabel ignores derived assignments when no manual label exists", () => {
  withTempDistill(() => {
    seedSession(14);
    ensureDefaultLabels();

    const seededDb = openDistillDatabase();
    try {
      const label = seededDb.db
        .prepare("SELECT id FROM labels WHERE name = 'train' LIMIT 1")
        .get() as { id: number };

      seededDb.db.prepare(`
        INSERT INTO label_assignments (object_type, object_id, label_id, origin)
        VALUES ('session', 14, ?, 'model')
      `).run(label.id);
    } finally {
      seededDb.close();
    }

    toggleSessionLabel(14, "train");

    const verifyDb = openDistillDatabase();
    try {
      const assignments = verifyDb.db.prepare(`
        SELECT COUNT(*) AS count
        FROM label_assignments
        WHERE object_type = 'session' AND object_id = 14
      `).get() as { count: number };
      const manualAssignments = verifyDb.db.prepare(`
        SELECT COUNT(*) AS count
        FROM label_assignments
        WHERE object_type = 'session' AND object_id = 14 AND origin = 'manual'
      `).get() as { count: number };
      const events = verifyDb.db.prepare(`
        SELECT COUNT(*) AS count
        FROM activity_events
      `).get() as { count: number };

      assert.equal(assignments.count, 1);
      assert.equal(manualAssignments.count, 0);
      assert.equal(events.count, 0);
    } finally {
      verifyDb.close();
    }
  });
});

test("curation operations are no-ops when the session is missing", () => {
  withTempDistill(() => {
    addSessionTag(999, "ghost");
    removeSessionTag(999, 1);
    toggleSessionLabel(999, "train");

    const distillDb = openDistillDatabase();
    try {
      const activityCount = distillDb.db.prepare("SELECT COUNT(*) AS count FROM activity_events").get() as { count: number };
      const tagAssignmentCount = distillDb.db.prepare("SELECT COUNT(*) AS count FROM tag_assignments").get() as { count: number };
      const labelAssignmentCount = distillDb.db.prepare("SELECT COUNT(*) AS count FROM label_assignments").get() as { count: number };
      const labelCount = distillDb.db.prepare("SELECT COUNT(*) AS count FROM labels").get() as { count: number };

      assert.equal(activityCount.count, 0);
      assert.equal(tagAssignmentCount.count, 0);
      assert.equal(labelAssignmentCount.count, 0);
      assert.equal(labelCount.count, 0);
    } finally {
      distillDb.close();
    }
  });
});
