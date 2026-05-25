import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";
import { addSessionTag, ensureDefaultLabels, toggleSessionLabel } from "../distill/curation";
import { openDistillDatabase } from "../distill/db";
import { getSessionDetail, listRecentSessions, searchSessions } from "../distill/query";
import { escapeHtml } from "../shared/html";

function withTempDistill<T>(fn: (root: string) => T): T {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "distill-query-"));
  const previous = process.env.DISTILL_HOME;
  process.env.DISTILL_HOME = path.join(tempRoot, ".distill");

  try {
    return fn(tempRoot);
  } finally {
    process.env.DISTILL_HOME = previous;
    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

test("query layer derives a fallback title from normalized messages", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (1, 'claude_code', 'Claude Code', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (10, 1, 'session-1', NULL, '/tmp/demo', '2026-03-25T12:00:00Z', 2, 1, '{}')
    `).run();

    db.prepare(`
      INSERT INTO messages (
        session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES
      (10, 1, 'user', 'Please tighten the layout and spacing.', 'a', '2026-03-25T12:00:00Z', 'text', '{}'),
      (10, 2, 'assistant', 'I will update the styles.', 'b', '2026-03-25T12:01:00Z', 'text', '{}')
    `).run();

    const sessions = listRecentSessions();
    const detail = getSessionDetail(10);

    assert.equal(sessions[0]?.title, "Please tighten the layout and spacing.");
    assert.equal(sessions[0]?.preview, "I will update the styles.");
    assert.equal(detail?.title, "Please tighten the layout and spacing.");
    assert.equal(detail?.preview, "I will update the styles.");

    distillDb.close();
  });
});

test("query layer ignores meta messages when deriving session previews", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (2, 'opencode', 'OpenCode', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (11, 2, 'session-meta', NULL, '/tmp/demo', '2026-03-25T12:10:00Z', 3, 1, '{}')
    `).run();

    db.prepare(`
      INSERT INTO messages (
        session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES
      (11, 1, 'user', 'Plan the connector.', 'u1', '2026-03-25T12:10:00Z', 'text', '{}'),
      (11, 2, 'assistant', 'Need to inspect the repo first.', 'a1', '2026-03-25T12:10:01Z', 'meta', '{}'),
      (11, 3, 'assistant', 'I will inspect the repo first.', 'a2', '2026-03-25T12:10:02Z', 'text', '{}')
    `).run();

    const sessions = listRecentSessions();
    const detail = getSessionDetail(11);

    assert.equal(sessions[0]?.title, "Plan the connector.");
    assert.equal(sessions[0]?.preview, "I will inspect the repo first.");
    assert.equal(detail?.messages[1]?.messageKind, "meta");

    distillDb.close();
  });
});

test("query layer exposes projection metadata on session detail and tolerates malformed legacy metadata", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES
      (5, 'opencode', 'OpenCode', 'installed', '2026-03-25T00:00:00Z', '{}'),
      (6, 'codex', 'Codex', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, source_url, started_at, updated_at,
        message_count, raw_capture_count, summary, metadata_json
      ) VALUES
      (
        12, 5, 'session-projection', 'Projection rich session', '/tmp/demo',
        'https://example.test/session/12', '2026-03-25T12:00:00Z', '2026-03-25T12:30:00Z',
        1, 3, 'Preserve projection metadata in the detail view.',
        '{"capturePath":"opencode://session/session-projection","externalSessionIdProvenance":{"kind":"source"}}'
      ),
      (
        13, 6, 'session-bad-metadata', 'Legacy bad metadata', '/tmp/legacy',
        NULL, NULL, '2026-03-25T12:31:00Z', 0, 1, NULL, 'not-json'
      )
    `).run();

    db.prepare(`
      INSERT INTO messages (
        session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES
      (12, 1, 'user', 'Inspect the stored metadata.', 'meta-1', '2026-03-25T12:01:00Z', 'text', '{}')
    `).run();

    const detail = getSessionDetail(12);
    const legacyDetail = getSessionDetail(13);

    assert.equal(detail?.externalSessionId, "session-projection");
    assert.equal(detail?.startedAt, "2026-03-25T12:00:00Z");
    assert.equal(detail?.sourceUrl, "https://example.test/session/12");
    assert.equal(detail?.summary, "Preserve projection metadata in the detail view.");
    assert.equal(detail?.rawCaptureCount, 3);
    assert.deepEqual(detail?.metadata, {
      capturePath: "opencode://session/session-projection",
      externalSessionIdProvenance: {
        kind: "source"
      }
    });
    assert.deepEqual(legacyDetail?.metadata, {});
    assert.equal(legacyDetail?.externalSessionId, "session-bad-metadata");
    assert.equal(legacyDetail?.rawCaptureCount, 1);

    distillDb.close();
  });
});

test("query layer searches normalized sessions through FTS", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (1, 'codex', 'Codex', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (20, 1, 'session-2', 'Search target', '/tmp/demo', '2026-03-25T13:00:00Z', 2, 1, '{}')
    `).run();

    db.prepare(`
      INSERT INTO messages (
        id, session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES
      (100, 20, 1, 'user', 'Please investigate the analytics regression.', 'aa', '2026-03-25T13:00:00Z', 'text', '{}'),
      (101, 20, 2, 'assistant', 'I will inspect the analytics pipeline and isolate the regression.', 'bb', '2026-03-25T13:01:00Z', 'text', '{}')
    `).run();

    const results = searchSessions("analytics regression");
    assert.equal(results.length, 1);
    assert.equal(results[0]?.sessionId, 20);
    assert.equal(results[0]?.title, "Search target");
    assert.match(results[0]?.snippet ?? "", /analytics/i);

    distillDb.close();
  });
});

test("query layer normalizes punctuation-heavy search input into a safe FTS query", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (1, 'codex', 'Codex', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (21, 1, 'session-2b', 'Quoted target', '/tmp/demo', '2026-03-25T13:30:00Z', 1, 1, '{}')
    `).run();

    db.prepare(`
      INSERT INTO messages (
        id, session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES
      (102, 21, 1, 'user', 'analytics-regression in beta env', 'cc', '2026-03-25T13:30:00Z', 'text', '{}')
    `).run();

    const results = searchSessions("analytics-regression: \"beta\" env");
    assert.equal(results.length, 1);
    assert.equal(results[0]?.sessionId, 21);

    distillDb.close();
  });
});

test("query layer returns no results when search input yields zero FTS tokens", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (1, 'codex', 'Codex', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (22, 1, 'session-2c', 'Punctuation only', '/tmp/demo', '2026-03-25T13:40:00Z', 1, 1, '{}')
    `).run();

    db.prepare(`
      INSERT INTO messages (
        id, session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES
      (103, 22, 1, 'user', 'analytics regression exists here', 'dd', '2026-03-25T13:40:00Z', 'text', '{}')
    `).run();

    assert.deepEqual(searchSessions("!!! /// ???"), []);

    distillDb.close();
  });
});

test("query layer skips FTS when the requested limit clamps to zero", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (1, 'codex', 'Codex', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (23, 1, 'session-limit', 'Limit target', '/tmp/demo', '2026-03-25T13:41:00Z', 1, 1, '{}')
    `).run();

    db.prepare(`
      INSERT INTO messages (
        id, session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES
      (104, 23, 1, 'user', 'analytics limit guard', 'limit-hash', '2026-03-25T13:41:00Z', 'text', '{}')
    `).run();

    distillDb.close();

    const originalPrepare = DatabaseSync.prototype.prepare;
    let sawFtsPrepare = false;

    DatabaseSync.prototype.prepare = function patchedPrepare(sql: string): ReturnType<typeof originalPrepare> {
      if (sql.includes("FROM message_fts")) {
        sawFtsPrepare = true;
        throw new Error("FTS query should not run when maxResults is zero");
      }

      return originalPrepare.call(this, sql);
    };

    try {
      assert.deepEqual(searchSessions("analytics", 0), []);
      assert.deepEqual(searchSessions("analytics", -5), []);
    } finally {
      DatabaseSync.prototype.prepare = originalPrepare;
    }

    assert.equal(sawFtsPrepare, false);
  });
});

test("query layer returns session tags and labels after manual curation", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (1, 'claude_code', 'Claude Code', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (30, 1, 'session-3', 'Curated session', '/tmp/demo', '2026-03-25T14:00:00Z', 1, 1, '{}')
    `).run();

    db.close();

    ensureDefaultLabels();
    addSessionTag(30, "distill");
    toggleSessionLabel(30, "train");

    const sessions = listRecentSessions();
    const detail = getSessionDetail(30);
    const session = sessions.find((entry) => entry.id === 30);

    assert.deepEqual(session?.labels, ["train"]);
    assert.equal(session?.workflowState, "train_ready");
    assert.equal(detail?.tags.length, 1);
    assert.equal(detail?.tags[0]?.name, "distill");
    assert.equal(detail?.labels.length, 1);
    assert.equal(detail?.labels[0]?.name, "train");
  });
});

test("query layer returns the full session corpus and derives workflow states from labels", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (1, 'claude_code', 'Claude Code', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    const insertSession = db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (?, 1, ?, ?, '/tmp/demo', ?, 1, 1, '{}')
    `);
    const insertMessage = db.prepare(`
      INSERT INTO messages (
        session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES (?, 1, 'user', ?, ?, ?, 'text', '{}')
    `);

    for (let index = 0; index < 30; index += 1) {
      const sessionId = 100 + index;
      const updatedAt = `2026-03-25T15:${String(index).padStart(2, "0")}:00Z`;
      insertSession.run(sessionId, `session-${sessionId}`, `Session ${sessionId}`, updatedAt);
      insertMessage.run(sessionId, `Message for session ${sessionId}`, `hash-${sessionId}`, updatedAt);
    }

    distillDb.close();

    ensureDefaultLabels();
    toggleSessionLabel(100, "train");
    toggleSessionLabel(101, "holdout");
    toggleSessionLabel(102, "train");
    toggleSessionLabel(102, "sensitive");
    toggleSessionLabel(103, "train");
    toggleSessionLabel(103, "favorite");

    const sessions = listRecentSessions();
    const trainSession = sessions.find((session) => session.id === 100);
    const holdoutSession = sessions.find((session) => session.id === 101);
    const reviewSession = sessions.find((session) => session.id === 102);
    const favoriteSession = sessions.find((session) => session.id === 103);

    assert.equal(sessions.length, 30);
    assert.equal(trainSession?.workflowState, "train_ready");
    assert.equal(holdoutSession?.workflowState, "holdout_ready");
    assert.equal(reviewSession?.workflowState, "needs_review");
    assert.deepEqual(reviewSession?.labels, ["sensitive", "train"]);
    assert.equal(favoriteSession?.workflowState, "train_ready");
    assert.deepEqual(favoriteSession?.labels, ["favorite", "train"]);
  });
});

test("query layer treats conflicting dataset labels as needs_review", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (1, 'codex', 'Codex', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (104, 1, 'session-conflict', 'Conflicting labels', '/tmp/demo', '2026-03-25T15:31:00Z', 1, 1, '{}')
    `).run();

    db.prepare(`
      INSERT INTO messages (
        id, session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES
      (304, 104, 1, 'user', 'Ambiguous dataset labels need review.', 'hash-104', '2026-03-25T15:31:00Z', 'text', '{}')
    `).run();

    distillDb.close();

    ensureDefaultLabels();

    const labelDb = openDistillDatabase();
    try {
      const labels = labelDb.db.prepare(`
        SELECT id
        FROM labels
        WHERE name IN ('train', 'holdout')
        ORDER BY name ASC
      `).all() as Array<{ id: number }>;
      const insertAssignment = labelDb.db.prepare(`
        INSERT INTO label_assignments (object_type, object_id, label_id, origin)
        VALUES ('session', 104, ?, 'manual')
      `);

      for (const label of labels) {
        insertAssignment.run(label.id);
      }
    } finally {
      labelDb.close();
    }

    const session = listRecentSessions().find((entry) => entry.id === 104);
    const result = searchSessions("Ambiguous dataset labels")[0];

    assert.deepEqual(session?.labels, ["holdout", "train"]);
    assert.equal(session?.workflowState, "needs_review");
    assert.deepEqual(result?.labels, ["holdout", "train"]);
    assert.equal(result?.workflowState, "needs_review");
  });
});

test("query layer includes workflow state and labels on search results for review sessions", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (1, 'codex', 'Codex', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (140, 1, 'session-search-review', 'Searchable review session', '/tmp/demo', '2026-03-25T18:00:00Z', 1, 1, '{}')
    `).run();

    db.prepare(`
      INSERT INTO messages (
        id, session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES
      (410, 140, 1, 'user', 'Investigate the review-only analytics session.', 'hash-review', '2026-03-25T18:00:00Z', 'text', '{}')
    `).run();

    distillDb.close();

    ensureDefaultLabels();
    toggleSessionLabel(140, "train");
    toggleSessionLabel(140, "sensitive");

    const results = searchSessions("review-only analytics");

    assert.equal(results.length, 1);
    assert.equal(results[0]?.workflowState, "needs_review");
    assert.deepEqual(results[0]?.labels, ["sensitive", "train"]);
  });
});

test("query layer ignores non-manual label assignments in list, detail, and search read models", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (1, 'codex', 'Codex', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (141, 1, 'session-manual-only', 'Manual labels only', '/tmp/demo', '2026-03-25T18:10:00Z', 1, 1, '{}')
    `).run();

    db.prepare(`
      INSERT INTO messages (
        id, session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES
      (411, 141, 1, 'user', 'Filter label origins in the query layer.', 'hash-manual-only', '2026-03-25T18:10:00Z', 'text', '{}')
    `).run();

    distillDb.close();

    ensureDefaultLabels();

    const labelDb = openDistillDatabase();
    try {
      const labelRows = labelDb.db.prepare(`
        SELECT id, name
        FROM labels
        WHERE name IN ('favorite', 'sensitive', 'train')
        ORDER BY name ASC
      `).all() as Array<{ id: number; name: string }>;
      const insertAssignment = labelDb.db.prepare(`
        INSERT INTO label_assignments (object_type, object_id, label_id, origin)
        VALUES ('session', 141, ?, ?)
      `);

      for (const label of labelRows) {
        const origin =
          label.name === "train"
            ? "manual"
            : label.name === "sensitive"
              ? "auto_rule"
              : "model";
        insertAssignment.run(label.id, origin);
      }
    } finally {
      labelDb.close();
    }

    const session = listRecentSessions().find((entry) => entry.id === 141);
    const detail = getSessionDetail(141);
    const result = searchSessions("label origins")[0];

    assert.ok(detail);
    assert.ok(result);
    assert.deepEqual(session?.labels, ["train"]);
    assert.equal(session?.workflowState, "train_ready");
    assert.deepEqual(detail.labels, [
      {
        id: detail.labels[0]?.id,
        name: "train",
        scope: "session",
        origin: "manual"
      }
    ]);
    assert.deepEqual(result.labels, ["train"]);
    assert.equal(result.workflowState, "train_ready");
  });
});

test("query layer returns artifact summaries for session detail", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (1, 'claude_code', 'Claude Code', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (40, 1, 'session-4', 'Artifact session', '/tmp/demo', '2026-03-25T15:00:00Z', 1, 1, '{}')
    `).run();

    db.prepare(`
      INSERT INTO captures (
        id, source_id, capture_kind, source_path, raw_sha256, parser_version, status, captured_at
      ) VALUES (1, 1, 'project_session', '/tmp/demo/session.jsonl', 'sha', 'v0', 'normalized', '2026-03-25T15:00:00Z')
    `).run();

    db.prepare(`
      INSERT INTO capture_records (
        id, capture_id, line_no, record_type, provider_message_id, role, is_meta, content_json, metadata_json
      ) VALUES (500, 1, 8, 'assistant', 'msg-1', 'assistant', 0, '{}', '{}')
    `).run();

    db.prepare(`
      INSERT INTO messages (
        id, session_id, capture_record_id, external_message_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES
      (200, 40, 500, 'msg-1', 1, 'assistant', 'Running tool', 'hash-1', '2026-03-25T15:00:00Z', 'text', '{}')
    `).run();

    db.prepare(`
      INSERT INTO artifacts (
        session_id, capture_record_id, kind, mime_type, metadata_json, created_at
      ) VALUES
      (40, 500, 'tool_call', NULL, ?, '2026-03-25T15:00:01Z')
    `).run(JSON.stringify({
      type: "tool_use",
      name: "Read",
      input: {
        file_path: "/tmp/demo/src/app.ts"
      }
    }));

    const detail = getSessionDetail(40);
    assert.equal(detail?.artifacts.length, 1);
    assert.equal(detail?.artifacts[0]?.summary, "Tool call: Read");
    assert.match(detail?.artifacts[0]?.payloadJson ?? "", /file_path/);
    assert.equal(detail?.artifacts[0]?.messageOrdinal, 1);

    distillDb.close();
  });
});

test("query layer reads artifact/message relationships from direct artifact message links", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (4, 'claude_code', 'Claude Code', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (42, 4, 'session-direct-artifact-link', 'Direct artifact link', '/tmp/demo', '2026-03-25T15:05:00Z', 1, 1, '{}')
    `).run();

    db.prepare(`
      INSERT INTO messages (
        id, session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
      ) VALUES
      (201, 42, 1, 'assistant', 'Running linked tool', 'hash-2', '2026-03-25T15:05:00Z', 'text', '{}')
    `).run();

    db.prepare(`
      INSERT INTO artifacts (
        session_id, message_id, kind, metadata_json, created_at
      ) VALUES
      (42, 201, 'tool_call', ?, '2026-03-25T15:05:01Z')
    `).run(JSON.stringify({
      type: "tool_use",
      name: "Read",
      input: {
        file_path: "/tmp/demo/src/app.ts"
      }
    }));

    const detail = getSessionDetail(42);

    assert.equal(detail?.artifacts.length, 1);
    assert.equal(detail?.artifacts[0]?.messageOrdinal, 1);
    assert.equal(detail?.artifacts[0]?.messageRole, "assistant");

    distillDb.close();
  });
});

test("query layer prefers tool_result errors over partial output previews", () => {
  withTempDistill(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.prepare(`
      INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
      VALUES (3, 'opencode', 'OpenCode', 'installed', '2026-03-25T00:00:00Z', '{}')
    `).run();

    db.prepare(`
      INSERT INTO sessions (
        id, source_id, external_session_id, title, project_path, updated_at,
        message_count, raw_capture_count, metadata_json
      ) VALUES (41, 3, 'session-tool-error', 'Tool error session', '/tmp/demo', '2026-03-25T15:10:00Z', 1, 1, '{}')
    `).run();

    db.prepare(`
      INSERT INTO artifacts (
        session_id, kind, metadata_json, created_at
      ) VALUES
      (41, 'tool_result', ?, '2026-03-25T15:10:01Z')
    `).run(JSON.stringify({
      name: "Read",
      output: "partial stdout",
      error: "permission denied"
    }));

    const detail = getSessionDetail(41);

    assert.equal(detail?.artifacts[0]?.summary, "Tool result: permission denied");
    assert.equal(detail?.artifacts[0]?.payloadPreview, "permission denied");

    distillDb.close();
  });
});

test("escapeHtml encodes transcript text before renderer interpolation", () => {
  assert.equal(
    escapeHtml(`<script>alert("x")</script> & 'quoted'`),
    "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; &#39;quoted&#39;"
  );
});
