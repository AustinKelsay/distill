import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { discoverClaudeCodeCaptures } from "../connectors/claude_code/discover";
import { parseClaudeCodeCapture } from "../connectors/claude_code/parse";
import { snapshotClaudeCodeCapture } from "../connectors/claude_code/snapshot";
import { discoverCodexCaptures } from "../connectors/codex/discover";
import { parseCodexCapture } from "../connectors/codex/parse";
import { snapshotCodexCapture } from "../connectors/codex/snapshot";
import { discoverOpenCodeCaptures } from "../connectors/opencode/discover";
import { parseOpenCodeCapture } from "../connectors/opencode/parse";
import { snapshotOpenCodeCapture } from "../connectors/opencode/snapshot";
import {
  installIngestFixtures
} from "./support/ingest_fixtures";

type SavedEnv = Record<
  | "CODEX_HOME"
  | "CLAUDE_HOME"
  | "OPENCODE_DB_PATH"
  | "OPENCODE_CONFIG_DIR"
  | "OPENCODE_STATE_DIR"
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

function withTempFixtureEnv<T>(fn: (root: string) => T): T {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "distill-connector-contract-"));
  const previous: SavedEnv = {
    CODEX_HOME: process.env.CODEX_HOME,
    CLAUDE_HOME: process.env.CLAUDE_HOME,
    OPENCODE_DB_PATH: process.env.OPENCODE_DB_PATH,
    OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR,
    OPENCODE_STATE_DIR: process.env.OPENCODE_STATE_DIR,
    TEST_OPENCODE_DB_PATH: process.env.TEST_OPENCODE_DB_PATH,
    TEST_OPENCODE_DB_QUERY_JSON: process.env.TEST_OPENCODE_DB_QUERY_JSON,
    TEST_OPENCODE_EXPORT_DIR: process.env.TEST_OPENCODE_EXPORT_DIR,
    TEST_OPENCODE_TRUNCATE_WHEN_PIPE: process.env.TEST_OPENCODE_TRUNCATE_WHEN_PIPE,
    PATH: process.env.PATH
  };

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

function assertNoUnexpectedKeys(value: Record<string, unknown>, allowed: string[]): void {
  for (const key of Object.keys(value)) {
    assert.equal(allowed.includes(key), true, `Unexpected key ${key}`);
  }
}

function assertCanonicalMessageShape(message: Record<string, unknown>): void {
  assertNoUnexpectedKeys(message, [
    "sourceLineNo",
    "externalMessageId",
    "parentExternalMessageId",
    "role",
    "text",
    "createdAt",
    "messageKind",
    "metadata"
  ]);
  assert.equal(typeof message.sourceLineNo, "number");
  assert.equal(typeof message.text, "string");
  assert.match(String(message.role), /^(user|assistant|system|tool)$/);
  assert.match(String(message.messageKind), /^(text|meta)$/);
}

function assertCanonicalArtifactShape(artifact: Record<string, unknown>): void {
  assertNoUnexpectedKeys(artifact, [
    "sourceLineNo",
    "externalMessageId",
    "kind",
    "mimeType",
    "payload"
  ]);
  assert.equal(typeof artifact.sourceLineNo, "number");
  assert.match(String(artifact.kind), /^(image|file|tool_call|tool_result|raw_json)$/);
}

function assertCanonicalRawRecordShape(rawRecord: Record<string, unknown>): void {
  assertNoUnexpectedKeys(rawRecord, [
    "lineNo",
    "recordType",
    "recordTimestamp",
    "providerMessageId",
    "parentProviderMessageId",
    "role",
    "isMeta",
    "contentText",
    "contentJson",
    "metadata"
  ]);
  assert.equal(typeof rawRecord.lineNo, "number");
  assert.equal(typeof rawRecord.recordType, "string");
}

test("CC-001 Codex connector emits canonical parsed shapes and keeps the live copy authoritative", () => {
  withTempFixtureEnv((root) => {
    installIngestFixtures(root, ["codex-live-session", "codex-archived-duplicate"]);

    const captures = discoverCodexCaptures();
    const liveCapture = captures.find(
      (capture) => capture.externalSessionId === "abc12345-1111-2222-3333-abcdefabcdef"
    );

    assert.equal(captures.filter((capture) => capture.externalSessionId === "abc12345-1111-2222-3333-abcdefabcdef").length, 1);
    assert.ok(liveCapture);
    assert.equal(liveCapture?.captureKind, "live_session");
    assert.match(liveCapture?.sourcePath ?? "", /\/\.codex\/sessions\//);

    const parsed = parseCodexCapture(liveCapture!, snapshotCodexCapture(liveCapture!));

    assertNoUnexpectedKeys(parsed.session as unknown as Record<string, unknown>, [
      "sourceKind",
      "externalSessionId",
      "title",
      "projectPath",
      "sourceUrl",
      "model",
      "modelProvider",
      "cliVersion",
      "gitBranch",
      "startedAt",
      "updatedAt",
      "summary",
      "metadata"
    ]);
    assert.equal(parsed.session.externalSessionId, "abc12345-1111-2222-3333-abcdefabcdef");
    assert.equal(parsed.messages.length, 2);
    assert.deepEqual(parsed.artifacts, []);
    assert.ok(parsed.rawRecords.length >= 4);
    parsed.messages.forEach((message) => assertCanonicalMessageShape(message as unknown as Record<string, unknown>));
    parsed.rawRecords.forEach((rawRecord) => assertCanonicalRawRecordShape(rawRecord as unknown as Record<string, unknown>));
  });
});

test("CC-002 Claude connector preserves transcript text while keeping image and tool blocks as artifacts", () => {
  withTempFixtureEnv((root) => {
    installIngestFixtures(root, ["claude-mixed-blocks"]);

    const [capture] = discoverClaudeCodeCaptures();
    assert.ok(capture);
    assert.equal(capture?.externalSessionId, "123e4567-e89b-12d3-a456-426614174000");

    const parsed = parseClaudeCodeCapture(capture!, snapshotClaudeCodeCapture(capture!));

    assert.equal(parsed.messages.length, 2);
    assert.deepEqual(parsed.messages.map((message) => message.text), [
      "Please review the screenshot and fix the layout.",
      "I will tighten the layout."
    ]);
    assert.deepEqual(parsed.artifacts.map((artifact) => artifact.kind), ["image", "tool_call", "tool_result"]);
    parsed.messages.forEach((message) => assertCanonicalMessageShape(message as unknown as Record<string, unknown>));
    parsed.artifacts.forEach((artifact) => assertCanonicalArtifactShape(artifact as unknown as Record<string, unknown>));
    parsed.rawRecords.forEach((rawRecord) => assertCanonicalRawRecordShape(rawRecord as unknown as Record<string, unknown>));
  });
});

test("CC-003 OpenCode connector preserves visible meta parts and unknown structured payloads inside canonical shapes", () => {
  withTempFixtureEnv((root) => {
    installIngestFixtures(root, ["opencode-visible-meta"]);

    const [capture] = discoverOpenCodeCaptures();
    assert.ok(capture);
    assert.equal(capture?.sourcePath, "opencode://session/ses_1");
    assert.equal(capture?.externalSessionId, "ses_1");

    const parsed = parseOpenCodeCapture(capture!, snapshotOpenCodeCapture(capture!));

    assert.equal(parsed.session.externalSessionId, "ses_1");
    assert.equal(parsed.messages.some((message) => message.messageKind === "meta"), true);
    assert.equal(parsed.messages.some((message) => message.role === "tool"), true);
    assert.deepEqual(parsed.artifacts.map((artifact) => artifact.kind), ["tool_call", "tool_result", "file", "file", "raw_json"]);
    parsed.messages.forEach((message) => assertCanonicalMessageShape(message as unknown as Record<string, unknown>));
    parsed.artifacts.forEach((artifact) => assertCanonicalArtifactShape(artifact as unknown as Record<string, unknown>));
    parsed.rawRecords.forEach((rawRecord) => assertCanonicalRawRecordShape(rawRecord as unknown as Record<string, unknown>));
  });
});
