import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";
import { sourceConnectors } from "../connectors";
import { SourceConnector } from "../connectors/types";
import { getCaptureContentRef, openDistillDatabase, readCaptureText, runInTransaction } from "../distill/db";
import { ensureDirectory } from "../distill/fs";
import { getInlineCaptureMaxBytes, readCaptureContentText, resolveCaptureBlobPath } from "../distill/raw_capture";
import { runImport } from "../distill/import";
import { DiscoveredCapture } from "../shared/types";
import {
  getInstalledFixtureSourcePath,
  installIngestFixtures,
  readFixtureCaptureText,
  writeFakeOpenCodeExecutable
} from "./support/ingest_fixtures";

type SavedEnv = Record<
  | "DISTILL_HOME"
  | "CODEX_HOME"
  | "CLAUDE_HOME"
  | "OPENCODE_DB_PATH"
  | "OPENCODE_CONFIG_DIR"
  | "OPENCODE_STATE_DIR"
  | "HOME"
  | "TEST_OPENCODE_DB_PATH"
  | "TEST_OPENCODE_DB_QUERY_JSON"
  | "TEST_OPENCODE_EXPORT_DIR"
  | "TEST_OPENCODE_TRUNCATE_WHEN_PIPE"
  | "PATH",
  string | undefined
>;

function restoreEnv(saved: SavedEnv): void {
  for (const [key, value] of Object.entries(saved)) {
    if (value === undefined) {
      delete process.env[key];
    } else {
      process.env[key] = value;
    }
  }
}

function withTempEnv<T>(fn: (root: string) => T): T {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "distill-test-"));
  const previous: SavedEnv = {
    DISTILL_HOME: process.env.DISTILL_HOME,
    CODEX_HOME: process.env.CODEX_HOME,
    CLAUDE_HOME: process.env.CLAUDE_HOME,
    OPENCODE_DB_PATH: process.env.OPENCODE_DB_PATH,
    OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR,
    OPENCODE_STATE_DIR: process.env.OPENCODE_STATE_DIR,
    HOME: process.env.HOME,
    TEST_OPENCODE_DB_PATH: process.env.TEST_OPENCODE_DB_PATH,
    TEST_OPENCODE_DB_QUERY_JSON: process.env.TEST_OPENCODE_DB_QUERY_JSON,
    TEST_OPENCODE_EXPORT_DIR: process.env.TEST_OPENCODE_EXPORT_DIR,
    TEST_OPENCODE_TRUNCATE_WHEN_PIPE: process.env.TEST_OPENCODE_TRUNCATE_WHEN_PIPE,
    PATH: process.env.PATH
  };

  process.env.HOME = tempRoot;
  process.env.DISTILL_HOME = path.join(tempRoot, ".distill");
  process.env.CODEX_HOME = path.join(tempRoot, ".codex");
  process.env.CLAUDE_HOME = path.join(tempRoot, ".claude");
  process.env.OPENCODE_DB_PATH = path.join(tempRoot, ".local", "share", "opencode", "opencode.db");
  process.env.OPENCODE_CONFIG_DIR = path.join(tempRoot, ".config", "opencode");
  process.env.OPENCODE_STATE_DIR = path.join(tempRoot, ".local", "state", "opencode");

  try {
    return fn(tempRoot);
  } finally {
    restoreEnv(previous);
    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

function writeFixtureFiles(root: string): void {
  installIngestFixtures(root, ["codex-live-session", "claude-mixed-blocks"]);
  writeFakeOpenCodeExecutable(root, [], {});
}

test("writeFakeOpenCodeExecutable replaces stale export fixtures between calls", () => {
  withTempEnv((root) => {
    writeFakeOpenCodeExecutable(root, [], {
      stale_session: "{\"stale\":true}\n"
    });
    writeFakeOpenCodeExecutable(root, [], {
      fresh_session: "{\"fresh\":true}\n"
    });

    assert.deepEqual(
      fs.readdirSync(path.join(root, "opencode-exports")).sort(),
      ["fresh_session.json"]
    );
  });
});

test("runImport bootstraps the database and records discovered captures", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const report = runImport();
    const db = new DatabaseSync(report.databasePath);

    try {
      const sourceCount = db.prepare("SELECT COUNT(*) AS count FROM sources").get() as { count: number };
      const captureCount = db.prepare("SELECT COUNT(*) AS count FROM captures").get() as { count: number };
      const captureRecordCount = db.prepare("SELECT COUNT(*) AS count FROM capture_records").get() as { count: number };
      const sessionCount = db.prepare("SELECT COUNT(*) AS count FROM sessions").get() as { count: number };
      const messageCount = db.prepare("SELECT COUNT(*) AS count FROM messages").get() as { count: number };
      const activityCount = db.prepare("SELECT COUNT(*) AS count FROM activity_events").get() as { count: number };
      const activityEvents = db
        .prepare("SELECT event_type FROM activity_events ORDER BY id ASC")
        .all() as Array<{ event_type: string }>;

      assert.equal(sourceCount.count, 3);
      assert.equal(captureCount.count, 2);
      assert.ok(captureRecordCount.count >= 2);
      assert.equal(sessionCount.count, 2);
      assert.ok(messageCount.count >= 2);
      assert.equal(activityCount.count, 4);
      assert.deepEqual(activityEvents.map((row) => row.event_type), [
        "capture_recorded",
        "projection_replaced",
        "capture_recorded",
        "projection_replaced"
      ]);
      assert.equal(report.sourceSummaries.length, 3);
      assert.equal(report.sourceSummaries.every((summary) => summary.failedCaptures === 0), true);
    } finally {
      db.close();
    }
  });
});

test("runImport persists recoverable inline raw content for file-backed captures", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const report = runImport();
    const db = new DatabaseSync(report.databasePath);
    const capture = db
      .prepare(`
        SELECT id, raw_sha256, source_size_bytes
        FROM captures
        WHERE external_session_id = 'abc12345-1111-2222-3333-abcdefabcdef'
      `)
      .get() as { id: number; raw_sha256: string; source_size_bytes: number };
    const expectedRawText = fs.readFileSync(getInstalledFixtureSourcePath(root, "codex-live-session"), "utf8");

    try {
      const contentRef = getCaptureContentRef(db, capture.id);

      assert.ok(contentRef);
      assert.equal(contentRef?.kind, "inline");
      assert.match(contentRef?.mediaType ?? "", /application\/x-ndjson/i);
      assert.equal(contentRef?.sha256, capture.raw_sha256);
      assert.equal(contentRef?.byteSize, capture.source_size_bytes);
      assert.equal(readCaptureText(db, capture.id), expectedRawText);
    } finally {
      db.close();
    }
  });
});

test("runImport persists blob-backed raw content for the large shared fixture", () => {
  withTempEnv((root) => {
    installIngestFixtures(root, ["large-capture-blob"]);
    writeFakeOpenCodeExecutable(root, [], {});

    const report = runImport();
    const db = new DatabaseSync(report.databasePath);
    const capture = db
      .prepare(`
        SELECT id, raw_sha256, source_size_bytes
        FROM captures
        WHERE external_session_id = 'blob-large-session'
      `)
      .get() as { id: number; raw_sha256: string; source_size_bytes: number };
    const expectedRawText = fs.readFileSync(getInstalledFixtureSourcePath(root, "large-capture-blob"), "utf8");

    try {
      const contentRef = getCaptureContentRef(db, capture.id);

      assert.ok(contentRef);
      assert.equal(contentRef?.kind, "blob");
      assert.equal(contentRef?.sha256, capture.raw_sha256);
      assert.equal(contentRef?.byteSize, capture.source_size_bytes);
      assert.ok((contentRef?.byteSize ?? 0) > getInlineCaptureMaxBytes());
      assert.equal(readCaptureText(db, capture.id), expectedRawText);
      assert.equal(report.sourceSummaries.find((summary) => summary.kind === "codex")?.importedCaptures, 1);
    } finally {
      db.close();
    }
  });
});

test("resolveCaptureBlobPath rejects paths outside the Distill blob root", () => {
  withTempEnv(() => {
    assert.throws(() => resolveCaptureBlobPath("../escape.json"), /blob root|must be relative/i);
    assert.throws(() => resolveCaptureBlobPath("/tmp/escape.json"), /blob root|must be relative/i);
    assert.throws(
      () => readCaptureContentText({
        kind: "blob",
        mediaType: "application/json; charset=utf-8",
        blobPath: "../escape.json",
        sha256: "abc123",
        byteSize: 1
      }),
      /blob root|must be relative/i
    );
  });
});

test("runImport is idempotent for unchanged raw captures", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const first = runImport();
    const second = runImport();

    const db = new DatabaseSync(first.databasePath);
    const captureCount = db.prepare("SELECT COUNT(*) AS count FROM captures").get() as { count: number };

    assert.equal(captureCount.count, 2);
    assert.equal(second.sourceSummaries.every((summary) => summary.importedCaptures === 0), true);
    assert.equal(second.sourceSummaries.filter((summary) => summary.kind !== "opencode").every((summary) => summary.skippedCaptures >= 1), true);
    assert.equal(second.sourceSummaries.every((summary) => summary.failedCaptures === 0), true);

    db.close();
  });
});

test("runImport refreshes parser_version when retrying an unfinished capture", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const first = runImport();
    const seededDb = new DatabaseSync(first.databasePath);
    seededDb
      .prepare(`
        UPDATE captures
        SET status = ?, parser_version = ?, error_text = ?
        WHERE external_session_id = ?
      `)
      .run("captured", "legacy-v1", "stale retry state", "abc12345-1111-2222-3333-abcdefabcdef");
    seededDb.close();

    const second = runImport();
    const db = new DatabaseSync(first.databasePath);
    const capture = db
      .prepare("SELECT parser_version, status, error_text FROM captures WHERE external_session_id = ?")
      .get("abc12345-1111-2222-3333-abcdefabcdef") as {
      parser_version: string;
      status: string;
      error_text: string | null;
    };

    assert.equal(second.sourceSummaries.find((summary) => summary.kind === "codex")?.importedCaptures, 1);
    assert.equal(capture.parser_version, "v0");
    assert.equal(capture.status, "normalized");
    assert.equal(capture.error_text, null);

    db.close();
  });
});

test("runImport reimports changed captures and refreshes normalized session content", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const first = runImport();
    const codexCapturePath = getInstalledFixtureSourcePath(root, "codex-live-session");

    fs.writeFileSync(
      codexCapturePath,
      [
        JSON.stringify({
          timestamp: "2026-03-25T10:00:00.000Z",
          type: "session_meta",
          payload: { id: "abc12345-1111-2222-3333-abcdefabcdef", cwd: "/tmp/demo" }
        }),
        JSON.stringify({
          timestamp: "2026-03-25T10:01:00.000Z",
          type: "response_item",
          payload: { type: "message", role: "user", content: [{ type: "input_text", text: "hello codex" }] }
        }),
        JSON.stringify({
          timestamp: "2026-03-25T10:02:00.000Z",
          type: "response_item",
          payload: {
            type: "message",
            role: "assistant",
            content: [{ type: "output_text", text: "updated answer" }]
          }
        })
      ].join("\n")
    );

    const second = runImport();
    const db = new DatabaseSync(first.databasePath);

    const captureCount = db.prepare("SELECT COUNT(*) AS count FROM captures").get() as { count: number };
    const session = db
      .prepare("SELECT raw_capture_count, message_count FROM sessions WHERE external_session_id = ?")
      .get("abc12345-1111-2222-3333-abcdefabcdef") as { raw_capture_count: number; message_count: number };
    const messages = db
      .prepare(
        "SELECT role, text, ordinal FROM messages WHERE session_id = (SELECT id FROM sessions WHERE external_session_id = ?) ORDER BY ordinal"
      )
      .all("abc12345-1111-2222-3333-abcdefabcdef")
      .map((row) => ({ ...row })) as Array<{ role: string; text: string; ordinal: number }>;

    assert.equal(captureCount.count, 3);
    assert.equal(second.sourceSummaries.find((summary) => summary.kind === "codex")?.importedCaptures, 1);
    assert.equal(session.raw_capture_count, 2);
    assert.equal(session.message_count, 2);
    assert.deepEqual(messages, [
      { role: "user", text: "hello codex", ordinal: 1 },
      { role: "assistant", text: "updated answer", ordinal: 2 }
    ]);

    db.close();
  });
});

test("runImport preserves the prior projection when a fixture-backed changed capture fails to parse", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const first = runImport();
    const codexCapturePath = getInstalledFixtureSourcePath(root, "codex-live-session");

    fs.writeFileSync(codexCapturePath, readFixtureCaptureText("parse-failure-after-snapshot"));

    const second = runImport();
    const db = new DatabaseSync(first.databasePath);
    const captures = db
      .prepare(`
        SELECT status
        FROM captures
        WHERE external_session_id = 'abc12345-1111-2222-3333-abcdefabcdef'
        ORDER BY id ASC
      `)
      .all() as Array<{ status: string }>;
    const messages = db
      .prepare(`
        SELECT role, text
        FROM messages
        WHERE session_id = (
          SELECT id
          FROM sessions
          WHERE external_session_id = 'abc12345-1111-2222-3333-abcdefabcdef'
        )
        ORDER BY ordinal ASC
      `)
      .all()
      .map((row) => ({ ...row })) as Array<{ role: string; text: string }>;

    try {
      assert.deepEqual(captures.map((row) => row.status), ["normalized", "failed_parse"]);
      assert.deepEqual(messages, [
        { role: "user", text: "hello codex" },
        { role: "assistant", text: "I will update the code." }
      ]);
      assert.equal(second.sourceSummaries.find((summary) => summary.kind === "codex")?.importedCaptures, 0);
      assert.equal(second.sourceSummaries.find((summary) => summary.kind === "codex")?.failedCaptures, 1);
    } finally {
      db.close();
    }
  });
});

test("runImport imports Codex sessions from the live sessions directory", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const liveCodexPath = path.join(
      root,
      ".codex",
      "sessions",
      "2026",
      "03",
      "30"
    );
    ensureDirectory(liveCodexPath);

    fs.writeFileSync(
      path.join(liveCodexPath, "rollout-2026-03-30T08-09-36-live1234-1111-2222-3333-abcdefabcdef.jsonl"),
      [
        JSON.stringify({
          timestamp: "2026-03-30T08:09:36.000Z",
          type: "session_meta",
          payload: { id: "live1234-1111-2222-3333-abcdefabcdef", cwd: "/tmp/live-demo" }
        }),
        JSON.stringify({
          timestamp: "2026-03-30T08:10:00.000Z",
          type: "response_item",
          payload: { type: "message", role: "user", content: [{ type: "input_text", text: "recent live codex session" }] }
        })
      ].join("\n")
    );

    const report = runImport();
    const db = new DatabaseSync(report.databasePath);

    const session = db
      .prepare("SELECT title, project_path, updated_at FROM sessions WHERE external_session_id = ?")
      .get("live1234-1111-2222-3333-abcdefabcdef") as
      | { title: string | null; project_path: string | null; updated_at: string | null }
      | undefined;
    const codexSummary = report.sourceSummaries.find((summary) => summary.kind === "codex");

    assert.ok(session);
    assert.equal(session?.title, "recent live codex session");
    assert.equal(session?.project_path, "/tmp/live-demo");
    assert.equal(session?.updated_at, "2026-03-30T08:10:00.000Z");
    assert.equal(codexSummary?.importedCaptures, 2);

    db.close();
  });
});

test("runImport links imported artifacts to projected messages while preserving capture provenance", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const claudePath = path.join(root, ".claude", "projects", "demo-project");
    fs.writeFileSync(
      path.join(claudePath, "artifact-session.jsonl"),
      [
        JSON.stringify({
          type: "user",
          uuid: "u1",
          sessionId: "artifact-session",
          timestamp: "2026-03-25T11:05:00.000Z",
          message: {
            role: "user",
            content: [{ type: "text", text: "Inspect the file read tool activity" }]
          }
        }),
        JSON.stringify({
          type: "assistant",
          uuid: "a1",
          parentUuid: "u1",
          sessionId: "artifact-session",
          timestamp: "2026-03-25T11:05:05.000Z",
          message: {
            role: "assistant",
            content: [
              { type: "text", text: "Running tool" },
              { type: "tool_use", name: "Read", input: { file_path: "/tmp/demo/src/app.ts" } },
              { type: "tool_result", content: "ok" }
            ]
          }
        })
      ].join("\n")
    );

    const report = runImport();
    const db = new DatabaseSync(report.databasePath);
    const artifactRows = db
      .prepare(`
        SELECT
          a.kind,
          a.message_id,
          a.capture_record_id,
          m.external_message_id
        FROM artifacts a
        LEFT JOIN messages m ON m.id = a.message_id
        WHERE a.session_id = (
          SELECT id
          FROM sessions
          WHERE external_session_id = 'artifact-session'
        )
        ORDER BY a.id ASC
      `)
      .all() as Array<{
      kind: string;
      message_id: number | null;
      capture_record_id: number | null;
      external_message_id: string | null;
    }>;

    assert.equal(report.sourceSummaries.find((summary) => summary.kind === "claude_code")?.importedCaptures, 2);
    assert.deepEqual(artifactRows.map((row) => row.kind), ["tool_call", "tool_result"]);
    assert.equal(artifactRows.every((row) => typeof row.message_id === "number"), true);
    assert.equal(artifactRows.every((row) => typeof row.capture_record_id === "number"), true);
    assert.equal(new Set(artifactRows.map((row) => row.message_id)).size, 1);
    assert.equal(artifactRows[0]?.external_message_id, "a1");

    db.close();
  });
});

test("runImport prefers live Codex sessions over archived duplicates", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const externalSessionId = "live1234-1111-2222-3333-abcdefabcdef";
    const archivedCodexPath = path.join(root, ".codex", "archived_sessions");
    const liveCodexPath = path.join(root, ".codex", "sessions", "2026", "03", "30");
    ensureDirectory(archivedCodexPath);
    ensureDirectory(liveCodexPath);

    fs.writeFileSync(
      path.join(archivedCodexPath, `rollout-2026-03-29T07-00-00-${externalSessionId}.jsonl`),
      [
        JSON.stringify({
          timestamp: "2026-03-29T07:00:00.000Z",
          type: "session_meta",
          payload: { id: externalSessionId, cwd: "/tmp/archived-demo" }
        }),
        JSON.stringify({
          timestamp: "2026-03-29T07:01:00.000Z",
          type: "response_item",
          payload: { type: "message", role: "user", content: [{ type: "input_text", text: "stale archived codex session" }] }
        })
      ].join("\n")
    );

    fs.writeFileSync(
      path.join(liveCodexPath, `rollout-2026-03-30T08-09-36-${externalSessionId}.jsonl`),
      [
        JSON.stringify({
          timestamp: "2026-03-30T08:09:36.000Z",
          type: "session_meta",
          payload: { id: externalSessionId, cwd: "/tmp/live-demo" }
        }),
        JSON.stringify({
          timestamp: "2026-03-30T08:10:00.000Z",
          type: "response_item",
          payload: { type: "message", role: "user", content: [{ type: "input_text", text: "recent live codex session" }] }
        })
      ].join("\n")
    );

    const report = runImport();
    const db = new DatabaseSync(report.databasePath);

    const sessions = db
      .prepare("SELECT title, project_path, updated_at FROM sessions WHERE external_session_id = ?")
      .all(externalSessionId) as Array<{ title: string | null; project_path: string | null; updated_at: string | null }>;
    const codexSummary = report.sourceSummaries.find((summary) => summary.kind === "codex");

    assert.equal(sessions.length, 1);
    assert.equal(sessions[0]?.title, "recent live codex session");
    assert.equal(sessions[0]?.project_path, "/tmp/live-demo");
    assert.equal(sessions[0]?.updated_at, "2026-03-30T08:10:00.000Z");
    assert.equal(codexSummary?.importedCaptures, 2);

    db.close();
  });
});

test("runImport imports OpenCode sessions through the fake CLI and keeps failures isolated", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);
    writeFakeOpenCodeExecutable(
      root,
      [
        {
          id: "ses_ok",
          title: "New session - 2026-03-26T19:15:49.354Z",
          directory: "/tmp/opencode-demo",
          version: "1.3.3",
          time_created: 1774543194067,
          time_updated: 1774543475213,
          time_archived: null,
          share_url: null
        },
        {
          id: "ses_fail",
          title: "Broken export",
          directory: "/tmp/opencode-demo",
          version: "1.3.3",
          time_created: 1774543194068,
          time_updated: 1774543475214,
          time_archived: null,
          share_url: null
        }
      ],
      {
        ses_ok: JSON.stringify({
          info: {
            id: "ses_ok",
            directory: "/tmp/opencode-demo",
            title: "New session - 2026-03-26T19:15:49.354Z",
            version: "1.3.3",
            time: { created: 1774543194067, updated: 1774543475213 }
          },
          messages: [
            {
              info: {
                id: "msg_user",
                role: "user",
                time: { created: 1774543194080 },
                model: { providerID: "ollama", modelID: "nemotron-cascade-2:30b" }
              },
              parts: [{ id: "part_user_text", type: "text", text: "Ship the OpenCode connector" }]
            },
            {
              info: {
                id: "msg_assistant",
                role: "assistant",
                parentID: "msg_user",
                time: { created: 1774543194090 },
                providerID: "ollama",
                modelID: "nemotron-cascade-2:30b"
              },
              parts: [
                { id: "part_reasoning", type: "reasoning", text: "Need to inspect the repo first." },
                { id: "part_text", type: "text", text: "I will inspect the repo first." }
              ]
            }
          ]
        })
      }
    );

    const report = runImport();
    const db = new DatabaseSync(report.databasePath);
    const opencodeSummary = report.sourceSummaries.find((summary) => summary.kind === "opencode");
    const session = db
      .prepare("SELECT title, message_count, model, cli_version FROM sessions WHERE external_session_id = 'ses_ok'")
      .get() as { title: string; message_count: number; model: string; cli_version: string };
    const failedCapture = db
      .prepare("SELECT status, error_text FROM captures WHERE external_session_id = 'ses_fail'")
      .get() as { status: string; error_text: string | null } | undefined;
    const failedActivity = db
      .prepare(`
        SELECT event_type, object_id, payload_json
        FROM activity_events
        WHERE event_type = 'capture_failed'
        ORDER BY id DESC
        LIMIT 1
      `)
      .get() as { event_type: string; object_id: number | null; payload_json: string };

    assert.equal(opencodeSummary?.discoveredCaptures, 2);
    assert.equal(opencodeSummary?.importedCaptures, 1);
    assert.equal(opencodeSummary?.failedCaptures, 1);
    assert.equal(session.title, "Ship the OpenCode connector");
    assert.equal(session.message_count, 3);
    assert.equal(session.model, "nemotron-cascade-2:30b");
    assert.equal(session.cli_version, "1.3.3");
    assert.equal(failedCapture, undefined);
    assert.equal(failedActivity.event_type, "capture_failed");
    assert.equal(failedActivity.object_id, null);
    assert.match(failedActivity.payload_json, /missing export/i);
    assert.equal(report.captures.find((capture) => capture.externalSessionId === "ses_ok")?.status, "imported");
    assert.equal(report.captures.find((capture) => capture.externalSessionId === "ses_fail")?.status, "failed");
    assert.match(report.captures.find((capture) => capture.externalSessionId === "ses_fail")?.errorText ?? "", /missing export/i);

    db.close();
  });
});

test("runImport imports large OpenCode exports even when pipe stdout truncates", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);
    process.env.TEST_OPENCODE_TRUNCATE_WHEN_PIPE = "1";

    const largeText = "A".repeat(70_000);
    const exportedSessionJson = JSON.stringify({
      info: {
        id: "ses_large",
        directory: "/tmp/opencode-demo",
        title: "Large export",
        version: "1.3.3",
        time: { created: 1774543194067, updated: 1774543475213 }
      },
      messages: [
        {
          info: {
            id: "msg_user",
            role: "user",
            time: { created: 1774543194080 },
            model: { providerID: "openai", modelID: "gpt-5.4" }
          },
          parts: [{ id: "part_user_text", type: "text", text: "Import the large export" }]
        },
        {
          info: {
            id: "msg_assistant",
            role: "assistant",
            parentID: "msg_user",
            time: { created: 1774543194090 },
            providerID: "openai",
            modelID: "gpt-5.4"
          },
          parts: [{ id: "part_large_text", type: "text", text: largeText }]
        }
      ]
    });

    writeFakeOpenCodeExecutable(
      root,
      [
        {
          id: "ses_large",
          title: "Large export",
          directory: "/tmp/opencode-demo",
          version: "1.3.3",
          time_created: 1774543194067,
          time_updated: 1774543475213,
          time_archived: null,
          share_url: null
        }
      ],
      {
        ses_large: exportedSessionJson
      }
    );

    const report = runImport();
    const db = new DatabaseSync(report.databasePath);
    const opencodeSummary = report.sourceSummaries.find((summary) => summary.kind === "opencode");
    const capture = report.captures.find((entry) => entry.externalSessionId === "ses_large");
    const captureRow = db
      .prepare(`
        SELECT id, raw_sha256
        FROM captures
        WHERE external_session_id = 'ses_large'
      `)
      .get() as { id: number; raw_sha256: string };
    const session = db
      .prepare("SELECT message_count FROM sessions WHERE external_session_id = 'ses_large'")
      .get() as { message_count: number };
    const messages = db
      .prepare(`
        SELECT text
        FROM messages
        WHERE session_id = (SELECT id FROM sessions WHERE external_session_id = 'ses_large')
        ORDER BY ordinal
      `)
      .all() as Array<{ text: string }>;
    const contentRef = getCaptureContentRef(db, captureRow.id);

    assert.equal(opencodeSummary?.importedCaptures, 1);
    assert.equal(opencodeSummary?.failedCaptures, 0);
    assert.equal(capture?.status, "imported");
    assert.equal(session.message_count, 2);
    assert.equal(messages[1]?.text.length, largeText.length);
    assert.equal(messages[1]?.text, largeText);
    assert.equal(contentRef?.kind, "blob");
    assert.match(contentRef?.mediaType ?? "", /application\/json/i);
    assert.equal(contentRef?.sha256, captureRow.raw_sha256);
    assert.ok(contentRef?.byteSize && contentRef.byteSize > getInlineCaptureMaxBytes());
    assert.equal(readCaptureText(db, captureRow.id), exportedSessionJson);
    assert.equal(
      contentRef?.kind === "blob" ? fs.existsSync(resolveCaptureBlobPath(contentRef.blobPath)) : false,
      true
    );

    db.close();
  });
});

test("runImport audits raw persistence failures and continues with later captures", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const largeText = "B".repeat(getInlineCaptureMaxBytes() + 4_096);
    const largeExport = JSON.stringify({
      info: {
        id: "ses_blob_fail",
        directory: "/tmp/opencode-demo",
        title: "Blob persistence failure",
        version: "1.3.3",
        time: { created: 1774543194067, updated: 1774543475213 }
      },
      messages: [
        {
          info: {
            id: "msg_user_large",
            role: "user",
            time: { created: 1774543194080 },
            model: { providerID: "openai", modelID: "gpt-5.4" }
          },
          parts: [{ id: "part_user_large", type: "text", text: "Force blob persistence to fail" }]
        },
        {
          info: {
            id: "msg_assistant_large",
            role: "assistant",
            parentID: "msg_user_large",
            time: { created: 1774543194090 },
            providerID: "openai",
            modelID: "gpt-5.4"
          },
          parts: [{ id: "part_large_text", type: "text", text: largeText }]
        }
      ]
    });
    const smallExport = JSON.stringify({
      info: {
        id: "ses_small_ok",
        directory: "/tmp/opencode-demo",
        title: "Inline persistence success",
        version: "1.3.3",
        time: { created: 1774543195067, updated: 1774543476213 }
      },
      messages: [
        {
          info: {
            id: "msg_user_small",
            role: "user",
            time: { created: 1774543195080 },
            model: { providerID: "openai", modelID: "gpt-5.4" }
          },
          parts: [{ id: "part_user_small", type: "text", text: "Keep importing after the blob write fails" }]
        },
        {
          info: {
            id: "msg_assistant_small",
            role: "assistant",
            parentID: "msg_user_small",
            time: { created: 1774543195090 },
            providerID: "openai",
            modelID: "gpt-5.4"
          },
          parts: [{ id: "part_small_text", type: "text", text: "The smaller capture still imports." }]
        }
      ]
    });

    writeFakeOpenCodeExecutable(
      root,
      [
        {
          id: "ses_blob_fail",
          title: "Blob persistence failure",
          directory: "/tmp/opencode-demo",
          version: "1.3.3",
          time_created: 1774543194067,
          time_updated: 1774543475213,
          time_archived: null,
          share_url: null
        },
        {
          id: "ses_small_ok",
          title: "Inline persistence success",
          directory: "/tmp/opencode-demo",
          version: "1.3.3",
          time_created: 1774543195067,
          time_updated: 1774543476213,
          time_archived: null,
          share_url: null
        }
      ],
      {
        ses_blob_fail: largeExport,
        ses_small_ok: smallExport
      }
    );

    const blobRoot = path.join(process.env.DISTILL_HOME ?? path.join(root, ".distill"), "blobs");
    ensureDirectory(blobRoot);
    fs.writeFileSync(path.join(blobRoot, "captures"), "block blob subdirectories");

    const report = runImport();
    const db = new DatabaseSync(report.databasePath);
    const opencodeSummary = report.sourceSummaries.find((summary) => summary.kind === "opencode");
    const captureRows = db
      .prepare("SELECT external_session_id FROM captures WHERE external_session_id LIKE 'ses_%' ORDER BY external_session_id ASC")
      .all() as Array<{ external_session_id: string | null }>;
    const sessionRows = db
      .prepare("SELECT external_session_id FROM sessions WHERE external_session_id LIKE 'ses_%' ORDER BY external_session_id ASC")
      .all() as Array<{ external_session_id: string }>;
    const failureEvents = db
      .prepare(`
        SELECT object_id, payload_json
        FROM activity_events
        WHERE event_type = 'capture_failed'
        ORDER BY id ASC
      `)
      .all() as Array<{ object_id: number | null; payload_json: string }>;
    const failedCapture = report.captures.find((capture) => capture.externalSessionId === "ses_blob_fail");
    const importedCapture = report.captures.find((capture) => capture.externalSessionId === "ses_small_ok");
    const persistenceFailureEvent = failureEvents.find((event) => /"externalSessionId":"ses_blob_fail"/.test(event.payload_json));

    assert.equal(opencodeSummary?.discoveredCaptures, 2);
    assert.equal(opencodeSummary?.importedCaptures, 1);
    assert.equal(opencodeSummary?.failedCaptures, 1);
    assert.deepEqual(captureRows.map((row) => row.external_session_id), ["ses_small_ok"]);
    assert.deepEqual(sessionRows.map((row) => row.external_session_id), ["ses_small_ok"]);
    assert.equal(failedCapture?.status, "failed");
    assert.ok(failedCapture?.rawSha256);
    assert.match(failedCapture?.errorText ?? "", /ENOTDIR|EEXIST|not a directory|file exists/i);
    assert.equal(importedCapture?.status, "imported");
    assert.ok(persistenceFailureEvent);
    assert.equal(persistenceFailureEvent?.object_id, null);
    assert.match(persistenceFailureEvent?.payload_json ?? "", /"stage":"persistence"/);

    db.close();
  });
});

test("runImport keeps unfinished captures retryable when persistence fails before parse", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const first = runImport();
    const seededDb = new DatabaseSync(first.databasePath);
    const captureRow = seededDb
      .prepare(`
        SELECT id
        FROM captures
        WHERE external_session_id = 'abc12345-1111-2222-3333-abcdefabcdef'
      `)
      .get() as { id: number };
    seededDb
      .prepare("UPDATE captures SET status = ?, parser_version = ?, error_text = NULL WHERE id = ?")
      .run("captured", "legacy-v1", captureRow.id);
    seededDb.exec(`
      CREATE TRIGGER captures_retry_persist_abort
      BEFORE UPDATE ON captures
      WHEN OLD.id = ${captureRow.id} AND NEW.parser_version <> OLD.parser_version
      BEGIN
        SELECT RAISE(FAIL, 'retry persistence update exploded');
      END;
    `);
    seededDb.close();

    const second = runImport();
    const db = new DatabaseSync(first.databasePath);
    const retriedCapture = db
      .prepare(`
        SELECT status, error_text
        FROM captures
        WHERE external_session_id = 'abc12345-1111-2222-3333-abcdefabcdef'
      `)
      .get() as { status: string; error_text: string | null };
    const failureEvent = db
      .prepare(`
        SELECT object_id, payload_json
        FROM activity_events
        WHERE event_type = 'capture_failed'
        ORDER BY id DESC
        LIMIT 1
      `)
      .get() as { object_id: number | null; payload_json: string };
    const failedCapture = second.captures.find((capture) => capture.externalSessionId === "abc12345-1111-2222-3333-abcdefabcdef");

    assert.equal(second.sourceSummaries.find((summary) => summary.kind === "codex")?.failedCaptures, 1);
    assert.equal(retriedCapture.status, "captured");
    assert.equal(retriedCapture.error_text, null);
    assert.equal(failureEvent.object_id, captureRow.id);
    assert.match(failureEvent.payload_json, /"stage":"persistence"/);
    assert.equal(failedCapture?.status, "failed");
    assert.match(failedCapture?.errorText ?? "", /retry persistence update exploded/);

    db.close();
  });
});

test("runImport rolls back partial normalization writes when session replacement fails", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);
    writeFakeOpenCodeExecutable(
      root,
      [
        {
          id: "ses_tx_fail",
          title: "Duplicate parts",
          directory: "/tmp/opencode-demo",
          version: "1.3.3",
          time_created: 1774543194067,
          time_updated: 1774543475213,
          time_archived: null,
          share_url: null
        }
      ],
      {
        ses_tx_fail: JSON.stringify({
          info: {
            id: "ses_tx_fail",
            directory: "/tmp/opencode-demo",
            title: "Duplicate parts",
            version: "1.3.3",
            time: { created: 1774543194067, updated: 1774543475213 }
          },
          messages: [
            {
              info: {
                id: "msg_user",
                role: "user",
                time: { created: 1774543194080 },
                model: { providerID: "ollama", modelID: "draft-model" }
              },
              parts: [{ id: "part_user_text", type: "text", text: "Trigger a transaction failure" }]
            },
            {
              info: {
                id: "msg_assistant",
                role: "assistant",
                parentID: "msg_user",
                time: { created: 1774543194090 },
                providerID: "openai",
                modelID: "final-model"
              },
              parts: [
                { id: "duplicate_part", type: "text", text: "first assistant response" },
                { id: "duplicate_part", type: "text", text: "second assistant response" }
              ]
            }
          ]
        })
      }
    );

    const report = runImport();
    const db = new DatabaseSync(report.databasePath);
    const failedCapture = db
      .prepare("SELECT id, status, error_text FROM captures WHERE external_session_id = 'ses_tx_fail'")
      .get() as { id: number; status: string; error_text: string | null };
    const captureRecordCount = db
      .prepare("SELECT COUNT(*) AS count FROM capture_records WHERE capture_id = ?")
      .get(failedCapture.id) as { count: number };
    const sessionCount = db
      .prepare("SELECT COUNT(*) AS count FROM sessions WHERE external_session_id = 'ses_tx_fail'")
      .get() as { count: number };
    const contentRef = getCaptureContentRef(db, failedCapture.id);
    const captureFailedEvent = db
      .prepare(`
        SELECT event_type, payload_json
        FROM activity_events
        WHERE event_type = 'capture_failed'
        ORDER BY id DESC
        LIMIT 1
      `)
      .get() as { event_type: string; payload_json: string };
    const failedReport = report.captures.find((capture) => capture.externalSessionId === "ses_tx_fail");
    const opencodeSummary = report.sourceSummaries.find((summary) => summary.kind === "opencode");

    assert.equal(failedCapture.status, "failed_parse");
    assert.match(failedCapture.error_text ?? "", /unique|constraint/i);
    assert.equal(captureRecordCount.count, 0);
    assert.equal(sessionCount.count, 0);
    assert.ok(contentRef);
    assert.match(readCaptureText(db, failedCapture.id) ?? "", /Trigger a transaction failure/);
    assert.equal(captureFailedEvent.event_type, "capture_failed");
    assert.match(captureFailedEvent.payload_json, /"stage":"parse"/);
    assert.equal(opencodeSummary?.failedCaptures, 1);
    assert.equal(failedReport?.status, "failed");
    assert.match(failedReport?.errorText ?? "", /unique|constraint/i);

    db.close();
  });
});

test("runImport preserves the prior projection when a changed capture fails to normalize", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const sessions = [
      {
        id: "ses_reimport_fail",
        title: "Reimport failure",
        directory: "/tmp/opencode-demo",
        version: "1.3.3",
        time_created: 1774543194067,
        time_updated: 1774543475213,
        time_archived: null,
        share_url: null
      }
    ];
    const successfulExport = JSON.stringify({
      info: {
        id: "ses_reimport_fail",
        directory: "/tmp/opencode-demo",
        title: "Reimport failure",
        version: "1.3.3",
        time: { created: 1774543194067, updated: 1774543475213 }
      },
      messages: [
        {
          info: {
            id: "msg_user",
            role: "user",
            time: { created: 1774543194080 },
            model: { providerID: "openai", modelID: "gpt-5.4" }
          },
          parts: [{ id: "part_user_text", type: "text", text: "Import once successfully" }]
        },
        {
          info: {
            id: "msg_assistant",
            role: "assistant",
            parentID: "msg_user",
            time: { created: 1774543194090 },
            providerID: "openai",
            modelID: "gpt-5.4"
          },
          parts: [{ id: "part_assistant_text", type: "text", text: "Initial assistant answer" }]
        }
      ]
    });
    const failedExport = JSON.stringify({
      info: {
        id: "ses_reimport_fail",
        directory: "/tmp/opencode-demo",
        title: "Reimport failure",
        version: "1.3.3",
        time: { created: 1774543194067, updated: 1774543475213 }
      },
      messages: [
        {
          info: {
            id: "msg_user",
            role: "user",
            time: { created: 1774543194080 },
            model: { providerID: "openai", modelID: "gpt-5.4" }
          },
          parts: [{ id: "part_user_text", type: "text", text: "Import once successfully" }]
        },
        {
          info: {
            id: "msg_assistant",
            role: "assistant",
            parentID: "msg_user",
            time: { created: 1774543194090 },
            providerID: "openai",
            modelID: "gpt-5.4"
          },
          parts: [
            { id: "duplicate_part", type: "text", text: "Broken response one" },
            { id: "duplicate_part", type: "text", text: "Broken response two" }
          ]
        }
      ]
    });

    writeFakeOpenCodeExecutable(root, sessions, { ses_reimport_fail: successfulExport });
    const first = runImport();
    writeFakeOpenCodeExecutable(root, sessions, { ses_reimport_fail: failedExport });
    const second = runImport();

    const db = new DatabaseSync(first.databasePath);
    const captures = db
      .prepare(`
        SELECT status
        FROM captures
        WHERE external_session_id = 'ses_reimport_fail'
        ORDER BY id ASC
      `)
      .all() as Array<{ status: string }>;
    const messages = db
      .prepare(`
        SELECT role, text
        FROM messages
        WHERE session_id = (SELECT id FROM sessions WHERE external_session_id = 'ses_reimport_fail')
        ORDER BY ordinal ASC
      `)
      .all()
      .map((row) => ({ ...row })) as Array<{ role: string; text: string }>;
    const opencodeSummary = second.sourceSummaries.find((summary) => summary.kind === "opencode");

    try {
      assert.deepEqual(captures.map((row) => row.status), ["normalized", "failed_parse"]);
      assert.deepEqual(messages, [
        { role: "user", text: "Import once successfully" },
        { role: "assistant", text: "Initial assistant answer" }
      ]);
      assert.equal(opencodeSummary?.importedCaptures, 0);
      assert.equal(opencodeSummary?.failedCaptures, 1);
    } finally {
      db.close();
    }
  });
});

test("runImport keeps other connectors importing when OpenCode discovery fails", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);
    writeFakeOpenCodeExecutable(root, "{not json", {});

    const report = runImport();
    const db = new DatabaseSync(report.databasePath);
    const opencodeSummary = report.sourceSummaries.find((summary) => summary.kind === "opencode");
    const codexSummary = report.sourceSummaries.find((summary) => summary.kind === "codex");
    const claudeSummary = report.sourceSummaries.find((summary) => summary.kind === "claude_code");
    const syncFailureEvent = db
      .prepare(`
        SELECT event_type, object_type, object_id, payload_json
        FROM activity_events
        WHERE event_type = 'sync_failed'
        ORDER BY id DESC
        LIMIT 1
      `)
      .get() as { event_type: string; object_type: string; object_id: number | null; payload_json: string };

    assert.equal(codexSummary?.importedCaptures, 1);
    assert.equal(claudeSummary?.importedCaptures, 1);
    assert.equal(opencodeSummary?.discoveredCaptures, 0);
    assert.equal(opencodeSummary?.importedCaptures, 0);
    assert.equal(opencodeSummary?.failedCaptures, 0);
    assert.equal(report.captures.length, 2);
    assert.match(
      report.failedEntries.find((entry) => entry.sourceKind === "opencode")?.errorText ?? "",
      /OpenCode session discovery failed: OpenCode command returned invalid JSON/
    );

    assert.equal(syncFailureEvent.event_type, "sync_failed");
    assert.equal(syncFailureEvent.object_type, "sync_job");
    assert.equal(syncFailureEvent.object_id, null);
    assert.match(syncFailureEvent.payload_json, /"stage":"discover"/);
    assert.match(syncFailureEvent.payload_json, /"sourceKind":"opencode"/);

    db.close();
  });
});

test("runImport audits connector detection failures and keeps healthy connectors importing", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const connectorIndex = sourceConnectors.findIndex((connector) => connector.kind === "opencode");
    assert.notEqual(connectorIndex, -1);

    const originalConnector = sourceConnectors[connectorIndex] as SourceConnector;
    sourceConnectors[connectorIndex] = {
      ...originalConnector,
      detect: () => {
        throw new Error("detect exploded");
      }
    };

    try {
      const report = runImport();
      const db = new DatabaseSync(report.databasePath);
      const opencodeSummary = report.sourceSummaries.find((summary) => summary.kind === "opencode");
      const codexSummary = report.sourceSummaries.find((summary) => summary.kind === "codex");
      const claudeSummary = report.sourceSummaries.find((summary) => summary.kind === "claude_code");
      const syncFailureEvent = db
        .prepare(`
          SELECT event_type, object_type, object_id, payload_json
          FROM activity_events
          WHERE event_type = 'sync_failed'
          ORDER BY id DESC
          LIMIT 1
        `)
        .get() as { event_type: string; object_type: string; object_id: number | null; payload_json: string };

      assert.equal(codexSummary?.importedCaptures, 1);
      assert.equal(claudeSummary?.importedCaptures, 1);
      assert.equal(opencodeSummary?.discoveredCaptures, 0);
      assert.equal(opencodeSummary?.importedCaptures, 0);
      assert.equal(opencodeSummary?.failedCaptures, 0);
      assert.equal(report.captures.length, 2);
      assert.match(
        report.failedEntries.find((entry) => entry.sourceKind === "opencode")?.errorText ?? "",
        /detect exploded/
      );
      assert.equal(syncFailureEvent.event_type, "sync_failed");
      assert.equal(syncFailureEvent.object_type, "sync_job");
      assert.equal(syncFailureEvent.object_id, null);
      assert.match(syncFailureEvent.payload_json, /"stage":"detect"/);
      assert.match(syncFailureEvent.payload_json, /"sourceKind":"opencode"/);

      db.close();
    } finally {
      sourceConnectors[connectorIndex] = originalConnector;
    }
  });
});

test("runImport does not create capture rows for snapshot failures and audits them", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const failingCapture: DiscoveredCapture = {
      sourceKind: "opencode",
      captureKind: "session_export",
      sourcePath: "opencode://session/snapshot-failure",
      externalSessionId: "snapshot-failure",
      sourceModifiedAt: "2026-03-30T08:10:00.000Z",
      sourceSizeBytes: 42,
      metadata: {}
    };

    const connectorIndex = sourceConnectors.findIndex((connector) => connector.kind === "opencode");
    assert.notEqual(connectorIndex, -1);

    const originalConnector = sourceConnectors[connectorIndex] as SourceConnector;
    let attempt = 0;
    sourceConnectors[connectorIndex] = {
      ...originalConnector,
      discoverCaptures: () => [failingCapture],
      snapshotCapture: () => {
        attempt += 1;
        throw new Error(`snapshot exploded ${attempt}`);
      }
    };

    try {
      const first = runImport();
      const second = runImport();
      const db = new DatabaseSync(first.databasePath);
      const rows = db
        .prepare(`
          SELECT c.raw_sha256, c.status, c.error_text
          FROM captures c
          JOIN sources s ON s.id = c.source_id
          WHERE s.kind = 'opencode' AND c.source_path = ?
        `)
        .all(failingCapture.sourcePath) as Array<{ raw_sha256: string; status: string; error_text: string | null }>;
      const activityEvents = db
        .prepare(`
          SELECT event_type, object_id, payload_json
          FROM activity_events
          WHERE event_type = 'capture_failed'
          ORDER BY id ASC
        `)
        .all() as Array<{ event_type: string; object_id: number | null; payload_json: string }>;
      const secondFailure = second.captures.find((capture) => capture.sourcePath === failingCapture.sourcePath);

      assert.equal(rows.length, 0);
      assert.equal(first.sourceSummaries.find((summary) => summary.kind === "opencode")?.failedCaptures, 1);
      assert.equal(second.sourceSummaries.find((summary) => summary.kind === "opencode")?.failedCaptures, 1);
      assert.equal(activityEvents.length, 2);
      assert.equal(activityEvents[0]?.event_type, "capture_failed");
      assert.equal(activityEvents[0]?.object_id, null);
      assert.match(activityEvents[0]?.payload_json ?? "", /"stage":"snapshot"/);
      assert.equal(activityEvents[1]?.object_id, null);
      assert.match(activityEvents[1]?.payload_json ?? "", /snapshot exploded 2/);
      assert.equal(secondFailure?.status, "failed");
      assert.match(secondFailure?.errorText ?? "", /snapshot exploded 2/);
      assert.equal(secondFailure?.rawSha256, undefined);

      db.close();
    } finally {
      sourceConnectors[connectorIndex] = originalConnector;
    }
  });
});

test("runImport migrates legacy activity events away from zero object ids", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const databasePath = path.join(process.env.DISTILL_HOME ?? path.join(root, ".distill"), "distill.db");
    ensureDirectory(path.dirname(databasePath));

    const legacyDb = new DatabaseSync(databasePath);
    legacyDb.exec(fs.readFileSync(path.resolve(process.cwd(), "schema.sql"), "utf8"));
    legacyDb.exec("DROP INDEX IF EXISTS idx_activity_events_created");
    legacyDb.exec("DROP INDEX IF EXISTS idx_activity_events_session");
    legacyDb.exec("DROP TABLE activity_events");
    legacyDb.exec(`
      CREATE TABLE activity_events (
        id INTEGER PRIMARY KEY,
        event_type TEXT NOT NULL,
        object_type TEXT NOT NULL,
        object_id INTEGER NOT NULL,
        session_id INTEGER REFERENCES sessions(id) ON DELETE CASCADE,
        payload_json TEXT NOT NULL DEFAULT '{}',
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
      )
    `);
    legacyDb.exec(`
      CREATE INDEX idx_activity_events_created
        ON activity_events(created_at DESC)
    `);
    legacyDb.exec(`
      CREATE INDEX idx_activity_events_session
        ON activity_events(session_id, created_at DESC)
    `);
    legacyDb
      .prepare(`
        INSERT INTO activity_events (
          event_type,
          object_type,
          object_id,
          payload_json,
          created_at
        ) VALUES (?, ?, ?, ?, ?)
      `)
      .run(
        "capture_failed",
        "capture",
        0,
        JSON.stringify({ stage: "snapshot", errorText: "legacy sentinel" }),
        "2026-03-29T00:00:00.000Z"
      );
    legacyDb.close();

    const failingCapture: DiscoveredCapture = {
      sourceKind: "opencode",
      captureKind: "session_export",
      sourcePath: "opencode://session/migrated-snapshot-failure",
      externalSessionId: "migrated-snapshot-failure",
      sourceModifiedAt: "2026-03-30T08:10:00.000Z",
      sourceSizeBytes: 42,
      metadata: {}
    };

    const connectorIndex = sourceConnectors.findIndex((connector) => connector.kind === "opencode");
    assert.notEqual(connectorIndex, -1);

    const originalConnector = sourceConnectors[connectorIndex] as SourceConnector;
    sourceConnectors[connectorIndex] = {
      ...originalConnector,
      discoverCaptures: () => [failingCapture],
      snapshotCapture: () => {
        throw new Error("snapshot exploded after migration");
      }
    };

    try {
      runImport();

      const db = new DatabaseSync(databasePath);
      const activityEventColumns = db
        .prepare("PRAGMA table_info(activity_events)")
        .all() as Array<{ name: string; notnull: number }>;
      const migratedEvents = db
        .prepare(`
          SELECT object_id, payload_json
          FROM activity_events
          WHERE event_type = 'capture_failed'
          ORDER BY id ASC
        `)
        .all() as Array<{ object_id: number | null; payload_json: string }>;

      assert.equal(activityEventColumns.find((column) => column.name === "object_id")?.notnull, 0);
      assert.deepEqual(
        migratedEvents.map((event) => event.object_id),
        [null, null]
      );
      assert.match(migratedEvents[0]?.payload_json ?? "", /legacy sentinel/);
      assert.match(migratedEvents[1]?.payload_json ?? "", /snapshot exploded after migration/);

      db.close();
    } finally {
      sourceConnectors[connectorIndex] = originalConnector;
    }
  });
});

test("openDistillDatabase migrates legacy failed capture statuses to failed_parse", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const databasePath = path.join(process.env.DISTILL_HOME ?? path.join(root, ".distill"), "distill.db");
    ensureDirectory(path.dirname(databasePath));

    const legacyDb = new DatabaseSync(databasePath);
    legacyDb.exec(fs.readFileSync(path.resolve(process.cwd(), "schema.sql"), "utf8"));
    legacyDb
      .prepare(`
        INSERT INTO sources (id, kind, display_name, install_status, metadata_json)
        VALUES (1, 'codex', 'Codex', 'installed', '{}')
      `)
      .run();
    legacyDb
      .prepare(`
        INSERT INTO captures (
          source_id,
          capture_kind,
          external_session_id,
          source_path,
          raw_sha256,
          raw_payload_json,
          parser_version,
          status,
          captured_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
      `)
      .run(
        1,
        "session_file",
        "legacy-failed",
        "/tmp/legacy-failed.jsonl",
        "abc123",
        JSON.stringify({
          sourceKind: "codex",
          metadata: {},
          contentRef: {
            kind: "inline",
            mediaType: "application/x-ndjson; charset=utf-8",
            text: "{}",
            sha256: "abc123",
            byteSize: 2
          }
        }),
        "legacy-v1",
        "failed",
        "2026-03-25T00:00:00.000Z"
      );
    legacyDb.close();

    const distillDb = openDistillDatabase();
    distillDb.close();

    const db = new DatabaseSync(databasePath);
    const capture = db
      .prepare("SELECT status FROM captures WHERE external_session_id = ?")
      .get("legacy-failed") as { status: string };

    assert.equal(capture.status, "failed_parse");

    db.close();
  });
});

test("openDistillDatabase adds the legacy artifacts.message_id column before artifact writes resume", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const databasePath = path.join(process.env.DISTILL_HOME ?? path.join(root, ".distill"), "distill.db");
    ensureDirectory(path.dirname(databasePath));

    const legacyDb = new DatabaseSync(databasePath);
    legacyDb.exec(fs.readFileSync(path.resolve(process.cwd(), "schema.sql"), "utf8"));
    legacyDb.exec("DROP TABLE artifacts");
    legacyDb.exec(`
      CREATE TABLE artifacts (
        id INTEGER PRIMARY KEY,
        session_id INTEGER REFERENCES sessions(id) ON DELETE CASCADE,
        capture_record_id INTEGER REFERENCES capture_records(id) ON DELETE SET NULL,
        kind TEXT NOT NULL,
        mime_type TEXT,
        blob_path TEXT,
        sha256 TEXT,
        byte_size INTEGER,
        metadata_json TEXT NOT NULL DEFAULT '{}',
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
      )
    `);
    legacyDb.exec(`
      CREATE INDEX idx_artifacts_session
        ON artifacts(session_id)
    `);
    legacyDb.close();

    const distillDb = openDistillDatabase();
    try {
      distillDb.db.prepare(`
        INSERT INTO artifacts (message_id, kind, metadata_json)
        VALUES (?, ?, ?)
      `).run(null, "tool_call", "{}");
    } finally {
      distillDb.close();
    }

    const db = new DatabaseSync(databasePath);
    const artifactColumns = db
      .prepare("PRAGMA table_info(artifacts)")
      .all() as Array<{ name: string; type: string }>;
    const artifact = db
      .prepare("SELECT message_id, kind FROM artifacts LIMIT 1")
      .get() as { message_id: number | null; kind: string };

    assert.equal(artifactColumns.some((column) => column.name === "message_id"), true);
    assert.equal(artifactColumns.find((column) => column.name === "message_id")?.type, "INTEGER");
    assert.equal(artifact.message_id, null);
    assert.equal(artifact.kind, "tool_call");

    db.close();
  });
});

test("openDistillDatabase backfills legacy artifact message links from the last projected message for shared provenance", () => {
  withTempEnv((root) => {
    writeFixtureFiles(root);

    const databasePath = path.join(process.env.DISTILL_HOME ?? path.join(root, ".distill"), "distill.db");
    ensureDirectory(path.dirname(databasePath));

    const legacyDb = new DatabaseSync(databasePath);
    legacyDb.exec(fs.readFileSync(path.resolve(process.cwd(), "schema.sql"), "utf8"));
    legacyDb
      .prepare(`
        INSERT INTO sources (id, kind, display_name, install_status, metadata_json)
        VALUES (1, 'claude_code', 'Claude Code', 'installed', '{}')
      `)
      .run();
    legacyDb
      .prepare(`
        INSERT INTO sessions (
          id, source_id, external_session_id, title, message_count, raw_capture_count, metadata_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?)
      `)
      .run(40, 1, "legacy-artifact-link", "Legacy artifact link", 2, 1, "{}");
    legacyDb
      .prepare(`
        INSERT INTO captures (
          id, source_id, capture_kind, source_path, raw_sha256, parser_version, status, captured_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
      `)
      .run(7, 1, "project_session", "/tmp/demo/session.jsonl", "legacy-sha", "v0", "normalized", "2026-03-25T15:00:00Z");
    legacyDb
      .prepare(`
        INSERT INTO capture_records (
          id, capture_id, line_no, record_type, provider_message_id, role, is_meta, content_json, metadata_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
      `)
      .run(500, 7, 8, "assistant", "msg-1", "assistant", 0, "{}", "{}");
    legacyDb
      .prepare(`
        INSERT INTO messages (
          id, session_id, capture_record_id, external_message_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      `)
      .run(200, 40, 500, "msg-1", 1, "assistant", "Running tool", "hash-1", "2026-03-25T15:00:00Z", "text", "{}");
    legacyDb
      .prepare(`
        INSERT INTO messages (
          id, session_id, capture_record_id, external_message_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      `)
      .run(201, 40, 500, "msg-1b", 2, "assistant", "Tool finished", "hash-2", "2026-03-25T15:00:02Z", "text", "{}");
    legacyDb
      .prepare(`
        INSERT INTO artifacts (
          session_id, message_id, capture_record_id, kind, metadata_json, created_at
        ) VALUES (?, ?, ?, ?, ?, ?)
      `)
      .run(40, null, 500, "tool_call", "{}", "2026-03-25T15:00:01Z");
    legacyDb.close();

    const distillDb = openDistillDatabase();
    distillDb.close();

    const db = new DatabaseSync(databasePath);
    const artifact = db
      .prepare(`
        SELECT message_id, capture_record_id
        FROM artifacts
        WHERE session_id = 40
      `)
      .get() as { message_id: number | null; capture_record_id: number | null };

    assert.equal(artifact.message_id, 201);
    assert.equal(artifact.capture_record_id, 500);

    db.close();
  });
});

test("runInTransaction rejects async callbacks before executing their bodies", () => {
  withTempEnv(() => {
    const distillDb = openDistillDatabase();
    const db = distillDb.db;

    db.exec(`
      CREATE TABLE tx_async_guard (
        id INTEGER PRIMARY KEY,
        value TEXT NOT NULL
      )
    `);

    let invoked = false;
    const asyncCallback = async () => {
      invoked = true;
      db.prepare("INSERT INTO tx_async_guard (value) VALUES (?)").run("leaked");
      await Promise.resolve();
      return 1;
    };

    assert.throws(
      () => runInTransaction(db, asyncCallback as unknown as () => never),
      /runInTransaction does not support async functions/
    );

    const rowCount = db
      .prepare("SELECT COUNT(*) AS count FROM tx_async_guard")
      .get() as { count: number };

    assert.equal(invoked, false);
    assert.equal(rowCount.count, 0);

    distillDb.close();
  });
});
