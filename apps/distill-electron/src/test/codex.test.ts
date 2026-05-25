import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { detectCodexSource } from "../connectors/codex/detect";
import { discoverCodexCaptures } from "../connectors/codex/discover";
import { ensureDirectory } from "../distill/fs";

function withTempCodexHome<T>(fn: (root: string) => T): T {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "distill-codex-"));
  const previousCodexHome = process.env.CODEX_HOME;

  process.env.CODEX_HOME = path.join(tempRoot, ".codex");

  try {
    return fn(tempRoot);
  } finally {
    if (previousCodexHome === undefined) {
      delete process.env.CODEX_HOME;
    } else {
      process.env.CODEX_HOME = previousCodexHome;
    }

    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

test("detectCodexSource points primaryCapturePath at an existing live sessions directory", () => {
  withTempCodexHome((root) => {
    const sessionsPath = path.join(root, ".codex", "sessions");
    ensureDirectory(sessionsPath);

    const source = detectCodexSource();
    const capturePaths = source.metadata.capturePaths as string[] | undefined;

    assert.equal(source.metadata.primaryCapturePath, sessionsPath);
    assert.equal(capturePaths?.[0], sessionsPath);
  });
});

test("discoverCodexCaptures leaves non-standard filenames without a synthetic session id", () => {
  withTempCodexHome((root) => {
    const archivedSessionsPath = path.join(root, ".codex", "archived_sessions");
    ensureDirectory(archivedSessionsPath);
    fs.writeFileSync(path.join(archivedSessionsPath, "notes.jsonl"), "{\"type\":\"message\"}\n");

    const captures = discoverCodexCaptures();

    assert.equal(captures.length, 1);
    assert.equal(captures[0]?.externalSessionId, undefined);
  });
});

test("discoverCodexCaptures marks live session files distinctly", () => {
  withTempCodexHome((root) => {
    const liveSessionsPath = path.join(root, ".codex", "sessions", "2026", "03", "30");
    ensureDirectory(liveSessionsPath);
    const codexHome = path.join(root, ".codex");
    fs.writeFileSync(
      path.join(liveSessionsPath, "rollout-2026-03-30T08-09-36-live-session-id.jsonl"),
      "{\"type\":\"message\"}\n"
    );

    const captures = JSON.parse(execFileSync(
      process.execPath,
      [
        "-e",
        "const { discoverCodexCaptures } = require('./dist/connectors/codex/discover.js'); process.stdout.write(JSON.stringify(discoverCodexCaptures()));"
      ],
      {
        cwd: process.cwd(),
        env: {
          ...process.env,
          CODEX_HOME: codexHome
        },
        encoding: "utf8"
      }
    )) as Array<{ captureKind: string; externalSessionId?: string }>;

    assert.equal(captures.length, 1);
    assert.equal(captures[0]?.captureKind, "live_session");
    assert.equal(captures[0]?.externalSessionId, "live-session-id");
  });
});
