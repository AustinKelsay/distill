import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { ensureDirectory } from "../distill/fs";
import { parseClaudeCodeCapture } from "../connectors/claude_code/parse";
import { snapshotClaudeCodeCapture } from "../connectors/claude_code/snapshot";
import { parseCodexCapture } from "../connectors/codex/parse";
import { snapshotCodexCapture } from "../connectors/codex/snapshot";
import { openCodeTimestampToIso } from "../connectors/opencode/common";
import { parseOpenCodeCapture } from "../connectors/opencode/parse";
import { snapshotOpenCodeCapture } from "../connectors/opencode/snapshot";
import { DiscoveredCapture } from "../shared/types";
import {
  buildDiscoveredCaptureFromFixture,
  installIngestFixtures
} from "./support/ingest_fixtures";

function withTempHomes<T>(fn: (root: string) => T): T {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "distill-parse-"));
  const previous = {
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
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

test("parseCodexCapture consumes the shared live Codex fixture and filters bootstrap noise", () => {
  withTempHomes((root) => {
    installIngestFixtures(root, ["codex-live-session"]);
    const capture = buildDiscoveredCaptureFromFixture(root, "codex-live-session");

    const parsed = parseCodexCapture(capture, snapshotCodexCapture(capture));

    assert.equal(parsed.session.title, "Live codex fixture");
    assert.equal(parsed.session.externalSessionId, "abc12345-1111-2222-3333-abcdefabcdef");
    assert.equal(parsed.messages.length, 2);
    assert.equal(parsed.messages[0]?.text, "hello codex");
    assert.equal(parsed.messages[1]?.text, "I will update the code.");
    assert.equal(parsed.session.model, "gpt-5.4");
    assert.equal(parsed.session.modelProvider, "openai");
  });
});

test("parseCodexCapture skips AGENTS instruction blobs before the real task", () => {
  withTempHomes((root) => {
    const codexHome = path.join(root, ".codex");
    ensureDirectory(path.join(codexHome, "archived_sessions"));

    const capturePath = path.join(codexHome, "archived_sessions", "rollout-2026-03-25-session-2.jsonl");
    fs.writeFileSync(
      capturePath,
      [
        JSON.stringify({
          timestamp: "2026-03-25T10:00:00Z",
          type: "session_meta",
          payload: { id: "session-2", cwd: "/tmp/proj" }
        }),
        JSON.stringify({
          timestamp: "2026-03-25T10:00:01Z",
          type: "response_item",
          payload: {
            type: "message",
            role: "user",
            content: [
              {
                type: "input_text",
                text: "# AGENTS.md instructions for /tmp/proj\n\n<INSTRUCTIONS>\n# Repository Guidelines\nUse tests.\n</INSTRUCTIONS>"
              }
            ]
          }
        }),
        JSON.stringify({
          timestamp: "2026-03-25T10:00:02Z",
          type: "response_item",
          payload: {
            type: "message",
            role: "user",
            content: [{ type: "input_text", text: "Implement the MCP settings page." }]
          }
        })
      ].join("\n")
    );

    const capture: DiscoveredCapture = {
      sourceKind: "codex",
      captureKind: "archived_session",
      sourcePath: capturePath,
      externalSessionId: "session-2",
      metadata: {}
    };

    const parsed = parseCodexCapture(capture, snapshotCodexCapture(capture));

    assert.equal(parsed.messages.length, 1);
    assert.equal(parsed.messages[0]?.text, "Implement the MCP settings page.");
  });
});

test("parseCodexCapture falls back to the first user message for title and captures model metadata", () => {
  withTempHomes((root) => {
    const codexHome = path.join(root, ".codex");
    ensureDirectory(path.join(codexHome, "archived_sessions"));

    const capturePath = path.join(codexHome, "archived_sessions", "rollout-2026-03-25-session-2.jsonl");
    fs.writeFileSync(
      capturePath,
      [
        JSON.stringify({
          timestamp: "2026-03-25T12:00:00Z",
          type: "session_meta",
          payload: { id: "session-2", cwd: "/tmp/fallback", cli_version: "1.2.3", model_provider: "openai" }
        }),
        JSON.stringify({
          timestamp: "2026-03-25T12:00:01Z",
          type: "turn_context",
          payload: { model: "gpt-5.4" }
        }),
        JSON.stringify({
          timestamp: "2026-03-25T12:00:02Z",
          type: "response_item",
          payload: {
            type: "message",
            role: "user",
            content: [{ type: "input_text", text: "Investigate the cache invalidation regression\nwith more detail." }]
          }
        })
      ].join("\n")
    );

    const capture: DiscoveredCapture = {
      sourceKind: "codex",
      captureKind: "archived_session",
      sourcePath: capturePath,
      externalSessionId: "session-2",
      metadata: {}
    };

    const parsed = parseCodexCapture(capture, snapshotCodexCapture(capture));

    assert.equal(parsed.session.title, "Investigate the cache invalidation regression");
    assert.equal(parsed.session.model, "gpt-5.4");
    assert.equal(parsed.session.modelProvider, "openai");
    assert.equal(parsed.session.cliVersion, "1.2.3");
    assert.equal(parsed.session.projectPath, "/tmp/fallback");
    assert.equal(parsed.messages.length, 1);
  });
});

test("parseCodexCapture records synthetic external session id provenance when the source id is missing", () => {
  withTempHomes((root) => {
    const codexHome = path.join(root, ".codex");
    ensureDirectory(path.join(codexHome, "archived_sessions"));

    const capturePath = path.join(codexHome, "archived_sessions", "notes.jsonl");
    fs.writeFileSync(
      capturePath,
      [
        JSON.stringify({
          timestamp: "2026-03-25T12:10:00Z",
          type: "response_item",
          payload: {
            type: "message",
            role: "user",
            content: [{ type: "input_text", text: "Fallback to a synthetic id" }]
          }
        })
      ].join("\n")
    );

    const capture: DiscoveredCapture = {
      sourceKind: "codex",
      captureKind: "archived_session",
      sourcePath: capturePath,
      metadata: {}
    };

    const parsed = parseCodexCapture(capture, snapshotCodexCapture(capture));

    assert.equal(parsed.session.externalSessionId, "notes.jsonl");
    assert.deepEqual(parsed.session.metadata.externalSessionIdProvenance, {
      kind: "synthetic",
      strategy: "capture_path_basename"
    });
  });
});

test("parseClaudeCodeCapture uses the resolved fallback session id for history title lookup", () => {
  withTempHomes((root) => {
    const claudeHome = path.join(root, ".claude");
    ensureDirectory(path.join(claudeHome, "projects", "demo"));
    fs.writeFileSync(
      path.join(claudeHome, "history.jsonl"),
      `${JSON.stringify({
        display: "Recovered from history",
        timestamp: 1,
        project: "/tmp/demo",
        sessionId: "synthetic-claude-session"
      })}\n`
    );

    const capturePath = path.join(claudeHome, "projects", "demo", "synthetic-claude-session.jsonl");
    fs.writeFileSync(
      capturePath,
      [
        JSON.stringify({
          type: "user",
          uuid: "u1",
          timestamp: "2026-03-25T11:00:00Z",
          cwd: "/tmp/demo",
          message: {
            role: "user",
            content: [{ type: "text", text: "Fallback title from the first message" }]
          }
        })
      ].join("\n")
    );

    const capture: DiscoveredCapture = {
      sourceKind: "claude_code",
      captureKind: "project_session",
      sourcePath: capturePath,
      metadata: { projectFolder: "/tmp/demo" }
    };

    const parsed = parseClaudeCodeCapture(capture, snapshotClaudeCodeCapture(capture));

    assert.equal(parsed.session.externalSessionId, "synthetic-claude-session");
    assert.equal(parsed.session.title, "Recovered from history");
  });
});

test("parseClaudeCodeCapture consumes the shared mixed-block fixture and preserves structured artifacts", () => {
  withTempHomes((root) => {
    installIngestFixtures(root, ["claude-mixed-blocks"]);
    const capture = buildDiscoveredCaptureFromFixture(root, "claude-mixed-blocks");

    const parsed = parseClaudeCodeCapture(capture, snapshotClaudeCodeCapture(capture));

    assert.equal(parsed.session.title, "Claude mixed content fixture");
    assert.equal(parsed.messages.length, 2);
    assert.equal(parsed.messages[0]?.text, "Please review the screenshot and fix the layout.");
    assert.equal(parsed.messages[1]?.text, "I will tighten the layout.");
    assert.deepEqual(parsed.artifacts.map((artifact) => artifact.kind), ["image", "tool_call", "tool_result"]);
    assert.equal(parsed.session.gitBranch, "feature/layout");
  });
});

test("parseOpenCodeCapture consumes the shared visible-meta fixture", () => {
  withTempHomes((root) => {
    installIngestFixtures(root, ["opencode-visible-meta"]);
    const capture = buildDiscoveredCaptureFromFixture(root, "opencode-visible-meta");
    const parsed = parseOpenCodeCapture(capture, snapshotOpenCodeCapture(capture));

    assert.equal(parsed.session.title, "Do I have a project for VisiBible in GTDspace?");
    assert.equal(parsed.session.sourceUrl, "https://opencode.ai/share/ses_1");
    assert.equal(parsed.session.model, "nemotron-cascade-2:30b");
    assert.equal(parsed.session.modelProvider, "ollama");
    assert.deepEqual(parsed.messages.map((message) => ({ role: message.role, kind: message.messageKind })), [
      { role: "user", kind: "text" },
      { role: "assistant", kind: "meta" },
      { role: "assistant", kind: "meta" },
      { role: "tool", kind: "meta" },
      { role: "assistant", kind: "meta" },
      { role: "assistant", kind: "meta" },
      { role: "assistant", kind: "text" },
      { role: "assistant", kind: "meta" }
    ]);
    assert.equal(parsed.messages[2]?.text, "We should search GTDspace first.");
    assert.match(parsed.messages[3]?.text ?? "", /\[tool:completed\]/);
    assert.deepEqual(parsed.artifacts.map((artifact) => artifact.kind), ["tool_call", "tool_result", "file", "file", "raw_json"]);
  });
});

test("snapshot fixture declares a missing Codex source path", () => {
  withTempHomes((root) => {
    installIngestFixtures(root, ["snapshot-failure-missing-source"]);
    const capture = buildDiscoveredCaptureFromFixture(root, "snapshot-failure-missing-source");

    assert.equal(fs.existsSync(capture.sourcePath), false);
    assert.throws(() => snapshotCodexCapture(capture), /ENOENT|no such file/i);
  });
});

test("parseOpenCodeCapture keeps the last populated assistant model when later messages omit model metadata", () => {
  const capture: DiscoveredCapture = {
    sourceKind: "opencode",
    captureKind: "session_export",
    sourcePath: "opencode://session/ses_2",
    externalSessionId: "ses_2",
    metadata: {}
  };

  const parsed = parseOpenCodeCapture(capture, {
    rawText: JSON.stringify({
      info: {
        id: "ses_2",
        title: "Model retention"
      },
      messages: [
        {
          info: {
            id: "msg_user",
            role: "user",
            time: { created: 1774543194080 },
            model: { providerID: "ollama", modelID: "draft-model" }
          },
          parts: [{ id: "part_user", type: "text", text: "Keep the detected assistant model" }]
        },
        {
          info: {
            id: "msg_assistant_1",
            role: "assistant",
            parentID: "msg_user",
            time: { created: 1774543194090 },
            providerID: "openai",
            modelID: "final-model"
          },
          parts: [{ id: "part_assistant_1", type: "text", text: "Using the final model." }]
        },
        {
          info: {
            id: "msg_assistant_2",
            role: "assistant",
            parentID: "msg_assistant_1",
            time: { created: 1774543195000 }
          },
          parts: [{ id: "part_assistant_2", type: "text", text: "Continuing without model metadata." }]
        }
      ]
    }),
    rawSha256: "sha-2",
    sourceModifiedAt: "2026-03-26T19:24:35.213Z",
    sourceSizeBytes: 120
  });

  assert.equal(parsed.session.model, "final-model");
  assert.equal(parsed.session.modelProvider, "openai");
});

test("parseOpenCodeCapture preserves system roles from OpenCode exports", () => {
  const capture: DiscoveredCapture = {
    sourceKind: "opencode",
    captureKind: "session_export",
    sourcePath: "opencode://session/ses_system",
    externalSessionId: "ses_system",
    metadata: {}
  };

  const parsed = parseOpenCodeCapture(capture, {
    rawText: JSON.stringify({
      info: {
        id: "ses_system",
        title: "System role retention"
      },
      messages: [
        {
          info: {
            id: "msg_system",
            role: "system",
            time: { created: 1774543194000 }
          },
          parts: [{ id: "part_system", type: "text", text: "You are a focused coding assistant." }]
        }
      ]
    }),
    rawSha256: "sha-system",
    sourceModifiedAt: "2026-03-26T19:24:35.213Z",
    sourceSizeBytes: 64
  });

  assert.equal(parsed.messages.length, 1);
  assert.equal(parsed.messages[0]?.role, "system");
  assert.equal(parsed.messages[0]?.text, "You are a focused coding assistant.");
});

test("openCodeTimestampToIso returns undefined for out-of-range timestamps", () => {
  assert.equal(openCodeTimestampToIso(Number.MAX_VALUE), undefined);
  assert.equal(openCodeTimestampToIso(String(Number.MAX_VALUE)), undefined);
});
