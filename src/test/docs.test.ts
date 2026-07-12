import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const repoRoot = process.cwd();

function readRepoFile(relativePath: string): string {
  return fs.readFileSync(path.join(repoRoot, relativePath), "utf8");
}

function readRepoJson(relativePath: string): unknown {
  return JSON.parse(readRepoFile(relativePath));
}

test("canonical docs package exists", () => {
  const requiredFiles = [
    "docs/README.md",
    "docs/specs/architecture.md",
    "docs/specs/data-model.md",
    "docs/specs/ingest-pipeline.md",
    "docs/specs/connectors.md",
    "docs/specs/search-curation-export.md",
    "docs/specs/activity-and-ops.md",
    "docs/governance/spec-governance.md",
    "docs/gaps/current-state-gap-register.md",
    "docs/roadmap/spec-alignment-plan.md",
    "docs/testing/contract-test-matrix.md",
    "docs/testing/contract-scenario-evidence.md"
  ];

  for (const relativePath of requiredFiles) {
    assert.equal(fs.existsSync(path.join(repoRoot, relativePath)), true, `${relativePath} should exist`);
  }
});

test("docs index defines authority order and links the canonical spec set", () => {
  const docsIndex = readRepoFile("docs/README.md");

  assert.match(docsIndex, /Normative vs Non-Normative/);
  assert.match(docsIndex, /How To Read The Docs/);
  assert.match(docsIndex, /Source Of Truth Files/);
  assert.match(docsIndex, /Updating Docs And Tests/);

  const requiredLinks = [
    "docs/specs/architecture.md",
    "docs/specs/data-model.md",
    "docs/specs/ingest-pipeline.md",
    "docs/specs/connectors.md",
    "docs/specs/search-curation-export.md",
    "docs/specs/activity-and-ops.md",
    "docs/governance/spec-governance.md",
    "docs/gaps/current-state-gap-register.md",
    "docs/roadmap/spec-alignment-plan.md",
    "docs/testing/contract-test-matrix.md",
    "docs/testing/contract-scenario-evidence.md"
  ];

  for (const relativePath of requiredLinks) {
    assert.match(docsIndex, new RegExp(relativePath.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }
});

test("root docs point to the canonical docs package and discovery is non-normative", () => {
  const readme = readRepoFile("README.md");
  const plan = readRepoFile("PLAN.md");
  const implementation = readRepoFile("IMPLEMENTATION.md");
  const discovery = readRepoFile("DISCOVERY.md");

  assert.match(readme, /^# DISTILL/m);
  assert.match(readme, /## Status/);
  assert.match(readme, /alpha/i);
  assert.match(readme, /## Supported Sources Right Now/);
  assert.match(readme, /Codex CLI/);
  assert.match(readme, /Claude Code/);
  assert.match(readme, /OpenCode/);
  assert.match(readme, /## DISTILL Flow/);
  assert.match(readme, /Discover captures/);
  assert.match(readme, /Normalize into local SQLite/);
  assert.match(readme, /Export approved JSONL/);
  assert.match(readme, /## Local Setup/);
  assert.match(readme, /npm run doctor/);
  assert.match(readme, /npm run import/);
  assert.match(readme, /npm start/);
  assert.match(readme, /~\/\.distill/);
  assert.match(readme, /docs\/README\.md/);
  assert.match(readme, /intentionally simple/i);

  assert.match(plan, /docs\/roadmap\/spec-alignment-plan\.md/);
  assert.match(plan, /roadmap pointer/i);

  assert.match(implementation, /docs\/specs\/architecture\.md/);
  assert.match(implementation, /docs\/gaps\/current-state-gap-register\.md/);
  assert.match(implementation, /informative/i);
  assert.doesNotMatch(implementation, /raw capture contents are not yet persisted/i);
  assert.doesNotMatch(implementation, /projection semantics are implemented implicitly/i);
  assert.doesNotMatch(implementation, /activity_events coverage is incomplete/i);

  assert.match(discovery, /non-normative discovery evidence/i);
  assert.match(discovery, /docs\/specs\/architecture\.md/);
});

test("gap register and contract test matrix track the required drift-guard surface", () => {
  const gapRegister = readRepoFile("docs/gaps/current-state-gap-register.md");
  const testMatrix = readRepoFile("docs/testing/contract-test-matrix.md");
  const governance = readRepoFile("docs/governance/spec-governance.md");

  assert.match(gapRegister, /historical/i);
  assert.match(gapRegister, /No open spec-alignment gaps are currently tracked/i);

  for (const gapId of [
    "GAP-001",
    "GAP-002",
    "GAP-003",
    "GAP-004",
    "GAP-005",
    "GAP-006",
    "GAP-007",
    "GAP-008",
    "GAP-009"
  ]) {
    assert.match(gapRegister, new RegExp(gapId));
  }

  for (const suiteName of [
    "connector_contract",
    "raw_capture_persistence",
    "projection_replacement",
    "activity_audit",
    "search_indexing",
    "session_read_model",
    "manual_curation",
    "export_contract",
    "sync_jobs_and_logs",
    "doc_truthfulness"
  ]) {
    assert.match(testMatrix, new RegExp(suiteName));
  }

  for (const scenarioId of ["SRM-001", "EC-003"]) {
    assert.match(testMatrix, new RegExp(scenarioId));
  }

  assert.match(governance, /Authority Order/);
  assert.match(governance, /PR Checklist/);
  assert.match(governance, /How To Record Gaps/);
  assert.match(governance, /How To Add New Source Connectors/);
});

test("ingest fixture manifest covers the required shared connector-contract corpus", () => {
  const testMatrix = readRepoFile("docs/testing/contract-test-matrix.md");
  const fixtureManifest = readRepoJson("src/test/fixtures/ingest/manifest.json") as Array<Record<string, unknown>>;
  const requiredFixtureIds = [
    "codex-live-session",
    "codex-archived-duplicate",
    "claude-mixed-blocks",
    "opencode-visible-meta",
    "parse-failure-after-snapshot",
    "snapshot-failure-missing-source",
    "large-capture-blob"
  ];

  assert.match(testMatrix, /src\/test\/connector_contract\.test\.ts/);
  assert.match(testMatrix, /src\/test\/support\/ingest_fixtures\.ts/);
  assert.match(testMatrix, /src\/test\/fixtures\/ingest\/manifest\.json/);

  assert.equal(Array.isArray(fixtureManifest), true);

  for (const fixtureId of requiredFixtureIds) {
    const fixture = fixtureManifest.find((entry) => entry.id === fixtureId) as Record<string, unknown> | undefined;

    assert.ok(fixture, `${fixtureId} should be present in the shared ingest fixture manifest`);
    assert.equal(Array.isArray(fixture?.scenarioIds), true, `${fixtureId} should declare scenario ids`);
    assert.equal(
      fs.existsSync(path.join(repoRoot, "src/test/fixtures/ingest", String(fixture?.fixtureDir))),
      true,
      `${fixtureId} fixture directory should exist`
    );
  }
});

test("cutover registry covers every matrix scenario and cites executable files", () => {
  const matrix = readRepoFile("docs/testing/contract-test-matrix.md");
  const registry = readRepoFile("docs/testing/contract-scenario-evidence.md");
  const scenarioSection = matrix.split("## Scenario Matrix")[1]?.split("## Executable")[0] ?? "";
  const matrixIds = [...scenarioSection.matchAll(/^\| `([^`]+)`/gm)].map((match) => match[1]);
  const registryRows = registry
    .split("\n")
    .filter((line) => /^\| [A-Z0-9-]+ \|/.test(line) && !line.startsWith("| ---"));
  const registryIds = registryRows.map((line) => line.split("|")[1].trim());

  assert.deepEqual(new Set(registryIds), new Set(matrixIds));
  assert.equal(registryRows.length, matrixIds.length);

  for (const line of registryRows) {
    const symbolCell = line.split("|")[5] ?? "";
    const paths = symbolCell.match(/(?:src|crates|apps)\/[^\s:+]+\.(?:tsx|ts|rs|mjs|json)(?=[:\s]|\x60|$)/g) ?? [];
    assert.ok(paths.length > 0, `registry symbol should name an executable file: ${symbolCell}`);
    assert.ok(paths.some((relativePath) => fs.existsSync(path.join(repoRoot, relativePath))), symbolCell);

    for (const symbol of symbolCell.split(" + ").map((entry) => entry.trim())) {
      const relativePath = paths.find((candidate) => symbol.includes(candidate));
      if (!relativePath) continue;

      const source = readRepoFile(relativePath);
      const suffix = symbol.slice(symbol.indexOf(relativePath) + relativePath.length).trim();
      for (const [, , title] of suffix.matchAll(/::(?:test|it)\((["'])(.*?)\1\)/g)) {
        assert.ok(
          source.includes(`test(\"${title}\"`) ||
            source.includes(`test('${title}'`) ||
            source.includes(`it(\"${title}\"`) ||
            source.includes(`it('${title}'`),
          `${relativePath} should contain test title ${title}`
        );
      }

      if (relativePath.endsWith(".rs")) {
        const functionName = suffix.match(/^::(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)$/)?.[1];
        if (functionName) {
          assert.match(source, new RegExp(`\\bfn\\s+${functionName}\\b`), `${relativePath} should define ${functionName}`);
        }
      } else if (suffix.startsWith("::")) {
        const title = suffix.slice(2);
        if (relativePath.endsWith(".json")) {
          assert.ok(source.includes(`\"${title}\"`), `${relativePath} should define ${title}`);
        } else if (!suffix.includes("::test(") && !suffix.includes("::it(")) {
          assert.ok(
            source.includes(`it(\"${title}\"`) ||
            source.includes(`it('${title}'`) ||
            source.includes(`test(\"${title}\"`) ||
            source.includes(`test('${title}'`),
            `${relativePath} should contain test title ${title}`
          );
        }
      }
    }
  }
});

test("agent instruction files exist and point agents to the canonical docs in order", () => {
  const agents = readRepoFile("AGENTS.md");
  const claude = readRepoFile("CLAUDE.md");

  for (const content of [agents, claude]) {
    assert.match(content, /docs\/README\.md/);
    assert.match(content, /docs\/specs\/architecture\.md/);
    assert.match(content, /docs\/specs\/data-model\.md/);
    assert.match(content, /docs\/specs\/ingest-pipeline\.md/);
    assert.match(content, /docs\/specs\/connectors\.md/);
    assert.match(content, /docs\/specs\/search-curation-export\.md/);
    assert.match(content, /docs\/specs\/activity-and-ops\.md/);
    assert.match(content, /docs\/gaps\/current-state-gap-register\.md/);
    assert.match(content, /docs\/testing\/contract-test-matrix\.md/);
    assert.match(content, /docs\/roadmap\/spec-alignment-plan\.md/);
    assert.match(content, /docs\/governance\/spec-governance\.md/);
    assert.match(content, /canonical docs win/i);
    assert.match(content, /README\.md/);
    assert.match(content, /PLAN\.md/);
    assert.match(content, /IMPLEMENTATION\.md/);
    assert.match(content, /DISCOVERY\.md/);
  }
});
