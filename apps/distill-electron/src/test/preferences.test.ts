import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { DEFAULT_SOURCE_COLORS, getSourceColors, setSourceColor } from "../distill/preferences";

function withTempDistillHome<T>(fn: () => T): T {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "distill-preferences-"));
  const previousDistillHome = process.env.DISTILL_HOME;

  process.env.DISTILL_HOME = path.join(tempRoot, ".distill");

  try {
    return fn();
  } finally {
    if (previousDistillHome === undefined) {
      delete process.env.DISTILL_HOME;
    } else {
      process.env.DISTILL_HOME = previousDistillHome;
    }

    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

test("source color preferences bootstrap from schema and persist updates", () => {
  withTempDistillHome(() => {
    assert.deepEqual(getSourceColors(), DEFAULT_SOURCE_COLORS);

    const updated = setSourceColor("codex", "#112233");

    assert.equal(updated.codex, "#112233");
    assert.equal(getSourceColors().codex, "#112233");
  });
});
