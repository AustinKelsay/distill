#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const root = process.cwd();
const checks = [];

function read(relativePath) {
  return fs.readFileSync(path.join(root, relativePath), "utf8");
}

function check(name, fn) {
  fn();
  checks.push(name);
}

check("canonical docs package exists", () => {
  for (const relativePath of [
    "docs/README.md",
    "docs/specs/architecture.md",
    "docs/specs/data-model.md",
    "docs/specs/ingest-pipeline.md",
    "docs/specs/connectors.md",
    "docs/specs/search-curation-export.md",
    "docs/specs/activity-and-ops.md",
    "docs/specs/legacy-migration.md",
    "docs/governance/spec-governance.md",
    "docs/gaps/current-state-gap-register.md",
    "docs/roadmap/spec-alignment-plan.md",
    "docs/testing/contract-test-matrix.md",
    "docs/testing/contract-scenario-evidence.md",
    "docs/release/first-beta.md",
    "docs/legacy/electron/README.md",
  ]) {
    assert.equal(fs.existsSync(path.join(root, relativePath)), true, `${relativePath} should exist`);
  }
});

check("docs index defines authority order", () => {
  const docsIndex = read("docs/README.md");
  for (const marker of [
    "Normative vs Non-Normative",
    "How To Read The Docs",
    "Source Of Truth Files",
    "Updating Docs And Tests",
    "docs/specs/architecture.md",
    "docs/testing/contract-scenario-evidence.md",
    "docs/release/first-beta.md",
    "docs/legacy/electron/README.md",
  ]) assert.match(docsIndex, new RegExp(marker.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")));
});

check("beta root docs point to the native product", () => {
  const readme = read("README.md");
  assert.match(readme, /^# DISTILL/m);
  assert.match(readme, /0\.2\.0-beta\.1/);
  assert.match(readme, /first beta/i);
  assert.match(readme, /Rust Library/);
  assert.match(readme, /Tauri 2/);
  assert.match(readme, /docs\/README\.md/);
  assert.doesNotMatch(readme, /npm start/);
  assert.doesNotMatch(readme, /Packaging remains deferred/i);
});

check("legacy Electron source is retired, not a runtime dependency", () => {
  const packageJson = read("package.json");
  assert.doesNotMatch(packageJson, /electron/);
  assert.equal(fs.existsSync(path.join(root, "src")), false, "root src tree should be retired");
  assert.equal(fs.existsSync(path.join(root, "static")), false, "root static tree should be retired");
  assert.equal(fs.existsSync(path.join(root, "schema.sql")), false, "legacy schema artifact should be retired");
  assert.match(read("docs/legacy/electron/README.md"), /removed/i);
});

check("gap register and matrix remain governed", () => {
  const gapRegister = read("docs/gaps/current-state-gap-register.md");
  const matrix = read("docs/testing/contract-test-matrix.md");
  const governance = read("docs/governance/spec-governance.md");
  assert.match(gapRegister, /historical/i);
  assert.match(gapRegister, /GAP-R001/);
  assert.match(matrix, /legacy-baseline/);
  assert.match(governance, /Authority Order/);
  assert.match(governance, /PR Checklist/);
});

check("cutover registry covers active matrix scenarios", () => {
  const matrix = read("docs/testing/contract-test-matrix.md");
  const registry = read("docs/testing/contract-scenario-evidence.md");
  const scenarioSection = matrix.split("## Scenario Matrix")[1]?.split("## Executable")[0] ?? "";
  const matrixIds = [...scenarioSection.matchAll(/^\| `([^`]+)`/gm)].map((match) => match[1]);
  const registryRows = registry
    .split("\n")
    .filter((line) => /^\| [A-Z0-9-]+\s+\|/.test(line) && !line.startsWith("| ---"));
  const registryIds = registryRows.map((line) => line.split("|")[1].trim());
  assert.deepEqual(new Set(registryIds), new Set(matrixIds));
  assert.equal(registryRows.length, matrixIds.length);

  for (const line of registryRows) {
    const cells = line.split("|");
    const fixtureAndSymbolCell = `${cells[4] ?? ""} ${cells[5] ?? ""}`;
    const statusCell = cells[9] ?? "";
    if (statusCell.includes("legacy-baseline")) continue;
    const paths = fixtureAndSymbolCell.match(/[A-Za-z0-9_./-]+\.(?:tsx|ts|rs|mjs|json)/g) ?? [];
    const candidates = paths.flatMap((relativePath) => [
      relativePath,
      `apps/distill-desktop/src/${relativePath}`,
      `apps/distill-desktop/src-tauri/tests/${relativePath}`,
      `apps/distill-desktop/scripts/${relativePath}`,
      `crates/distill-library/tests/${relativePath}`,
      `crates/distill-cli/tests/${relativePath}`,
    ]);
    assert.ok(paths.length > 0, `registry row should name an executable file: ${fixtureAndSymbolCell}`);
    assert.ok(candidates.some((relativePath) => fs.existsSync(path.join(root, relativePath))), fixtureAndSymbolCell);
  }
});

check("agent instructions point to canonical docs", () => {
  for (const relativePath of ["AGENTS.md", "CLAUDE.md"]) {
    const content = read(relativePath);
    for (const marker of [
      "docs/README.md",
      "docs/specs/architecture.md",
      "docs/specs/data-model.md",
      "docs/specs/ingest-pipeline.md",
      "docs/specs/connectors.md",
      "docs/specs/search-curation-export.md",
      "docs/specs/activity-and-ops.md",
      "docs/gaps/current-state-gap-register.md",
      "docs/testing/contract-test-matrix.md",
      "docs/roadmap/spec-alignment-plan.md",
      "docs/governance/spec-governance.md",
      "canonical docs win",
    ]) assert.match(content, new RegExp(marker.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")));
  }
});

console.log(`documentation checks OK: ${checks.length} checks`);
