import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

function readRendererSource(): string {
  return fs.readFileSync(path.resolve(process.cwd(), "src/renderer/app.ts"), "utf8");
}

test("Sessions view exposes the review-lane filter set", () => {
  const source = readRendererSource();

  assert.match(source, /lane:\s*"all"/);
  assert.match(source, /lane:\s*"needs_review"/);
  assert.match(source, /lane:\s*"train_ready"/);
  assert.match(source, /lane:\s*"holdout_ready"/);
  assert.match(source, /lane:\s*"favorite"/);
});

test("Sessions export menu offers only approved dataset targets", () => {
  const source = readRendererSource();

  assert.match(source, /data-export-dataset="train"/);
  assert.match(source, /data-export-dataset="holdout"/);
  assert.doesNotMatch(source, /data-export-dataset="favorite"/);
  assert.doesNotMatch(source, /Export favorites/);
});
