import assert from "node:assert/strict";
import test from "node:test";
import { buildDoctorReport } from "../distill-electron/doctor";
import { expandHome } from "../distill-electron/fs";
import { getOpenCodeDefaultDatabasePath, getOpenCodeStateDir } from "../distill-electron/paths";
import { getAppSettingsSnapshot } from "../distill-electron/settings";

test("expandHome expands a home-relative path", () => {
  const home = process.env.HOME;
  assert.ok(home);
  assert.equal(expandHome("~/distill-electron"), `${home}/distill-electron`);
});

test("doctor report returns the expected source kinds", () => {
  const report = buildDoctorReport();

  assert.equal(typeof report.scannedAt, "string");
  assert.equal(report.sources.length, 4);

  const kinds = report.sources.map((source) => source.kind).sort();
  assert.deepEqual(kinds, ["claude_code", "codex", "droid", "opencode"]);

  for (const source of report.sources) {
    assert.ok(source.displayName.length > 0);
    assert.ok(["installed", "partial", "not_found"].includes(source.installStatus));
    assert.ok(Array.isArray(source.checks));
    assert.ok(source.checks.length > 0);
  }
});

test("OpenCode path helpers prefer env overrides when present", () => {
  const previousState = process.env.OPENCODE_STATE_DIR;
  const previousDb = process.env.OPENCODE_DB_PATH;
  process.env.OPENCODE_STATE_DIR = "/tmp/opencode-state";
  process.env.OPENCODE_DB_PATH = "/tmp/opencode.db";

  try {
    assert.equal(getOpenCodeStateDir(), "/tmp/opencode-state");
    assert.equal(getOpenCodeDefaultDatabasePath(), "/tmp/opencode.db");
  } finally {
    if (previousState === undefined) {
      delete process.env.OPENCODE_STATE_DIR;
    } else {
      process.env.OPENCODE_STATE_DIR = previousState;
    }

    if (previousDb === undefined) {
      delete process.env.OPENCODE_DB_PATH;
    } else {
      process.env.OPENCODE_DB_PATH = previousDb;
    }
  }
});

test("settings snapshot reports the OpenCode DB env override", () => {
  const previousDb = process.env.OPENCODE_DB_PATH;
  process.env.OPENCODE_DB_PATH = "/tmp/opencode.db";

  try {
    assert.equal(getAppSettingsSnapshot().envOverrides.opencodeDbPath, true);
  } finally {
    if (previousDb === undefined) {
      delete process.env.OPENCODE_DB_PATH;
    } else {
      process.env.OPENCODE_DB_PATH = previousDb;
    }
  }
});
