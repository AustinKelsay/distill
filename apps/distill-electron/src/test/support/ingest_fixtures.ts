import fs from "node:fs";
import path from "node:path";
import { SourceKind, DiscoveredCapture } from "../../shared/types";
import { ensureDirectory } from "../../distill/fs";

type CaptureMode = "file" | "virtual";
type FixtureFailureMode = "parse" | "snapshot" | null;

type RawFixtureManifestEntry = {
  id?: unknown;
  sourceKind?: unknown;
  captureMode?: unknown;
  fixtureDir?: unknown;
  captureKind?: unknown;
  sourcePath?: unknown;
  expectedExternalSessionId?: unknown;
  createsTranscriptMessages?: unknown;
  createsArtifacts?: unknown;
  failureMode?: unknown;
  scenarioIds?: unknown;
  sourceModifiedAt?: unknown;
  metadata?: unknown;
};

type LargeFixtureSeed = {
  projectPath?: unknown;
  messagePrefix?: unknown;
  repeatCount?: unknown;
  assistantText?: unknown;
};

export type IngestFixtureManifestEntry = {
  id: string;
  sourceKind: SourceKind;
  captureMode: CaptureMode;
  fixtureDir: string;
  captureKind: string;
  sourcePath: string;
  expectedExternalSessionId?: string;
  createsTranscriptMessages: boolean;
  createsArtifacts: boolean;
  failureMode: FixtureFailureMode;
  scenarioIds: string[];
  sourceModifiedAt?: string;
  metadata: Record<string, unknown>;
};

export const INGEST_FIXTURE_ROOT = "src/test/fixtures/ingest";
export const INGEST_FIXTURE_MANIFEST_PATH = `${INGEST_FIXTURE_ROOT}/manifest.json`;

const repoRoot = process.cwd();
const fixtureRootPath = path.join(repoRoot, INGEST_FIXTURE_ROOT);

function expectString(value: unknown, fieldName: string): string {
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`Fixture manifest field ${fieldName} must be a non-empty string`);
  }

  return value;
}

function expectBoolean(value: unknown, fieldName: string): boolean {
  if (typeof value !== "boolean") {
    throw new Error(`Fixture manifest field ${fieldName} must be a boolean`);
  }

  return value;
}

function expectStringArray(value: unknown, fieldName: string): string[] {
  if (!Array.isArray(value) || value.some((entry) => typeof entry !== "string" || !entry.trim())) {
    throw new Error(`Fixture manifest field ${fieldName} must be a string array`);
  }

  return value;
}

function normalizeSourceKind(value: unknown): SourceKind {
  if (value === "codex" || value === "claude_code" || value === "opencode") {
    return value;
  }

  throw new Error(`Unsupported fixture source kind: ${String(value)}`);
}

function normalizeCaptureMode(value: unknown): CaptureMode {
  if (value === "file" || value === "virtual") {
    return value;
  }

  throw new Error(`Unsupported fixture capture mode: ${String(value)}`);
}

function normalizeFailureMode(value: unknown): FixtureFailureMode {
  if (value === null || value === "parse" || value === "snapshot") {
    return value;
  }

  throw new Error(`Unsupported fixture failure mode: ${String(value)}`);
}

function normalizeMetadata(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {};
  }

  return { ...(value as Record<string, unknown>) };
}

function loadFixtureManifest(): IngestFixtureManifestEntry[] {
  const raw = JSON.parse(
    fs.readFileSync(path.join(repoRoot, INGEST_FIXTURE_MANIFEST_PATH), "utf8")
  ) as unknown;

  if (!Array.isArray(raw)) {
    throw new Error("Ingest fixture manifest must be a JSON array");
  }

  return raw.map((row, index) => {
    const entry = (row ?? {}) as RawFixtureManifestEntry;
    const id = expectString(entry.id, `manifest[${index}].id`);

    return {
      id,
      sourceKind: normalizeSourceKind(entry.sourceKind),
      captureMode: normalizeCaptureMode(entry.captureMode),
      fixtureDir: expectString(entry.fixtureDir, `manifest[${index}].fixtureDir`),
      captureKind: expectString(entry.captureKind, `manifest[${index}].captureKind`),
      sourcePath: expectString(entry.sourcePath, `manifest[${index}].sourcePath`),
      expectedExternalSessionId:
        typeof entry.expectedExternalSessionId === "string" && entry.expectedExternalSessionId.trim()
          ? entry.expectedExternalSessionId
          : undefined,
      createsTranscriptMessages: expectBoolean(
        entry.createsTranscriptMessages,
        `manifest[${index}].createsTranscriptMessages`
      ),
      createsArtifacts: expectBoolean(entry.createsArtifacts, `manifest[${index}].createsArtifacts`),
      failureMode: normalizeFailureMode(entry.failureMode),
      scenarioIds: expectStringArray(entry.scenarioIds, `manifest[${index}].scenarioIds`),
      sourceModifiedAt:
        typeof entry.sourceModifiedAt === "string" && entry.sourceModifiedAt.trim()
          ? entry.sourceModifiedAt
          : undefined,
      metadata: normalizeMetadata(entry.metadata)
    };
  });
}

const ingestFixtureManifest = loadFixtureManifest();

export function getIngestFixtureManifest(): readonly IngestFixtureManifestEntry[] {
  return ingestFixtureManifest;
}

export function getIngestFixture(id: string): IngestFixtureManifestEntry {
  const entry = ingestFixtureManifest.find((fixture) => fixture.id === id);
  if (!entry) {
    throw new Error(`Unknown ingest fixture: ${id}`);
  }

  return entry;
}

export function getIngestFixtureDirectory(id: string): string {
  return path.join(fixtureRootPath, getIngestFixture(id).fixtureDir);
}

function copyDirectoryTree(sourceDir: string, destinationDir: string): void {
  if (!fs.existsSync(sourceDir)) {
    return;
  }

  ensureDirectory(destinationDir);

  for (const entry of fs.readdirSync(sourceDir, { withFileTypes: true })) {
    const sourcePath = path.join(sourceDir, entry.name);
    const destinationPath = path.join(destinationDir, entry.name);

    if (entry.isDirectory()) {
      copyDirectoryTree(sourcePath, destinationPath);
      continue;
    }

    ensureDirectory(path.dirname(destinationPath));
    fs.copyFileSync(sourcePath, destinationPath);
  }
}

function installLargeCaptureFixture(root: string, fixture: IngestFixtureManifestEntry): void {
  const seedPath = path.join(getIngestFixtureDirectory(fixture.id), "seed.json");
  const rawSeed = JSON.parse(fs.readFileSync(seedPath, "utf8")) as LargeFixtureSeed;
  const projectPath =
    typeof rawSeed.projectPath === "string" && rawSeed.projectPath.trim() ? rawSeed.projectPath : "/tmp/large";
  const messagePrefix =
    typeof rawSeed.messagePrefix === "string" ? rawSeed.messagePrefix : "Large capture payload ";
  const repeatCount =
    typeof rawSeed.repeatCount === "number" && Number.isFinite(rawSeed.repeatCount)
      ? rawSeed.repeatCount
      : 70_000;
  const assistantText =
    typeof rawSeed.assistantText === "string" && rawSeed.assistantText.trim()
      ? rawSeed.assistantText
      : "Blob-backed raw capture imported successfully.";
  const capturePath = path.join(root, fixture.sourcePath);
  const largeText = `${messagePrefix}${"A".repeat(repeatCount)}`;

  ensureDirectory(path.dirname(capturePath));
  fs.writeFileSync(
    capturePath,
    [
      JSON.stringify({
        timestamp: "2026-03-26T09:00:00.000Z",
        type: "session_meta",
        payload: { id: fixture.expectedExternalSessionId, cwd: projectPath }
      }),
      JSON.stringify({
        timestamp: "2026-03-26T09:00:01.000Z",
        type: "response_item",
        payload: {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: largeText }]
        }
      }),
      JSON.stringify({
        timestamp: "2026-03-26T09:00:02.000Z",
        type: "response_item",
        payload: {
          type: "message",
          role: "assistant",
          content: [{ type: "output_text", text: assistantText }]
        }
      })
    ].join("\n")
  );
}

type FakeOpenCodeSession = Record<string, unknown>;

function installOpenCodeFixtureEntries(root: string, fixtureIds: string[]): void {
  const sessionRows: FakeOpenCodeSession[] = [];
  const exportsBySession: Record<string, string> = {};

  for (const fixtureId of fixtureIds) {
    const fixtureDir = getIngestFixtureDirectory(fixtureId);
    const sessionsPath = path.join(fixtureDir, "opencode", "sessions.json");
    const exportsDir = path.join(fixtureDir, "opencode", "exports");

    if (fs.existsSync(sessionsPath)) {
      const parsed = JSON.parse(fs.readFileSync(sessionsPath, "utf8")) as unknown;
      if (!Array.isArray(parsed)) {
        throw new Error(`OpenCode fixture ${fixtureId} sessions.json must be an array`);
      }
      sessionRows.push(...parsed.filter((row): row is FakeOpenCodeSession => Boolean(row) && typeof row === "object"));
    }

    if (fs.existsSync(exportsDir)) {
      for (const entry of fs.readdirSync(exportsDir, { withFileTypes: true })) {
        if (!entry.isFile() || !entry.name.endsWith(".json")) {
          continue;
        }

        const sessionId = path.basename(entry.name, ".json");
        exportsBySession[sessionId] = fs.readFileSync(path.join(exportsDir, entry.name), "utf8");
      }
    }
  }

  if (sessionRows.length > 0 || Object.keys(exportsBySession).length > 0) {
    writeFakeOpenCodeExecutable(root, sessionRows, exportsBySession);
  }
}

export function installIngestFixtures(root: string, fixtureIds: string[]): void {
  const opencodeFixtureIds: string[] = [];

  for (const fixtureId of fixtureIds) {
    const fixture = getIngestFixture(fixtureId);
    const fixtureDir = getIngestFixtureDirectory(fixtureId);

    copyDirectoryTree(path.join(fixtureDir, "files"), root);

    if (fixture.id === "large-capture-blob") {
      installLargeCaptureFixture(root, fixture);
    }

    if (fixture.sourceKind === "opencode") {
      opencodeFixtureIds.push(fixtureId);
    }
  }

  installOpenCodeFixtureEntries(root, opencodeFixtureIds);
}

export function getInstalledFixtureSourcePath(root: string, fixtureId: string): string {
  const fixture = getIngestFixture(fixtureId);
  return fixture.captureMode === "file" ? path.join(root, fixture.sourcePath) : fixture.sourcePath;
}

export function readFixtureCaptureText(fixtureId: string): string {
  const fixture = getIngestFixture(fixtureId);
  if (fixture.captureMode !== "file") {
    throw new Error(`Fixture ${fixtureId} does not have a file-backed capture`);
  }

  if (fixture.id === "large-capture-blob") {
    throw new Error("Large generated fixture text must be read from the installed test root");
  }

  return fs.readFileSync(path.join(getIngestFixtureDirectory(fixtureId), "files", fixture.sourcePath), "utf8");
}

export function buildDiscoveredCaptureFromFixture(root: string, fixtureId: string): DiscoveredCapture {
  const fixture = getIngestFixture(fixtureId);
  const sourcePath = getInstalledFixtureSourcePath(root, fixtureId);
  const metadata = { ...fixture.metadata };
  let sourceModifiedAt = fixture.sourceModifiedAt;
  let sourceSizeBytes: number | undefined;

  if (fixture.captureMode === "file") {
    if (fixture.sourceKind === "claude_code" && metadata.projectFolder === undefined) {
      metadata.projectFolder = path.dirname(sourcePath);
    }

    if (fs.existsSync(sourcePath)) {
      const stat = fs.statSync(sourcePath);
      sourceModifiedAt = sourceModifiedAt ?? stat.mtime.toISOString();
      sourceSizeBytes = stat.size;
    }
  }

  return {
    sourceKind: fixture.sourceKind,
    captureKind: fixture.captureKind,
    sourcePath,
    externalSessionId: fixture.expectedExternalSessionId,
    sourceModifiedAt,
    sourceSizeBytes,
    metadata
  };
}

export function writeFakeOpenCodeExecutable(
  root: string,
  sessions: Array<Record<string, unknown>> | string,
  exportsBySession: Record<string, string>
): void {
  const binDir = path.join(root, ".bin");
  const opencodeDbPath = path.join(root, ".local", "share", "opencode", "opencode.db");
  const dbQueryPath = path.join(root, "opencode-sessions.json");
  const exportDir = path.join(root, "opencode-exports");

  ensureDirectory(binDir);
  ensureDirectory(path.dirname(opencodeDbPath));
  ensureDirectory(exportDir);
  ensureDirectory(path.join(root, ".config", "opencode"));
  ensureDirectory(path.join(root, ".local", "state", "opencode"));

  for (const entry of fs.readdirSync(exportDir, { withFileTypes: true })) {
    if (entry.isFile() && entry.name.endsWith(".json")) {
      fs.unlinkSync(path.join(exportDir, entry.name));
    }
  }

  fs.writeFileSync(opencodeDbPath, "");
  fs.writeFileSync(dbQueryPath, typeof sessions === "string" ? sessions : JSON.stringify(sessions, null, 2));
  fs.writeFileSync(path.join(root, ".config", "opencode", "opencode.json"), "{}\n");

  for (const [sessionId, output] of Object.entries(exportsBySession)) {
    fs.writeFileSync(path.join(exportDir, `${sessionId}.json`), output);
  }

  const safeExecPath = process.execPath
    .replace(/%/g, "%%")
    .replace(/\^/g, "^^")
    .replace(/&/g, "^&")
    .replace(/\|/g, "^|")
    .replace(/</g, "^<")
    .replace(/>/g, "^>")
    .replace(/"/g, "\"\"");

  const scriptPath = path.join(binDir, "opencode");
  const cmdPath = path.join(binDir, "opencode.cmd");
  fs.writeFileSync(
    scriptPath,
    `#!/usr/bin/env node
const fs = require("node:fs");
const path = require("node:path");

const args = process.argv.slice(2);

if (args[0] === "db" && args[1] === "path") {
  process.stdout.write(\`\${process.env.TEST_OPENCODE_DB_PATH ?? ""}\\n\`);
  process.exit(0);
}

if (args[0] === "db") {
  process.stdout.write(fs.readFileSync(process.env.TEST_OPENCODE_DB_QUERY_JSON, "utf8"));
  process.exit(0);
}

if (args[0] === "export") {
  const session = args[1];
  const file = path.join(process.env.TEST_OPENCODE_EXPORT_DIR, \`\${session}.json\`);
  if (!fs.existsSync(file)) {
    process.stderr.write(\`missing export for \${session}\\n\`);
    process.exit(1);
  }

  const output = fs.readFileSync(file);
  const shouldTruncate =
    process.env.TEST_OPENCODE_TRUNCATE_WHEN_PIPE === "1"
    && fs.fstatSync(1).isFIFO()
    && output.length > 65536;

  process.stderr.write(\`Exporting session: \${session}\\n\`);
  process.stdout.write(shouldTruncate ? output.subarray(0, 65536) : output);
  process.exit(0);
}

process.stderr.write("unsupported fake opencode command\\n");
process.exit(1);
`
  );
  fs.writeFileSync(
    cmdPath,
    `@echo off
"${safeExecPath}" "%~dp0opencode" %*
`
  );

  if (process.platform !== "win32") {
    fs.chmodSync(scriptPath, 0o755);
  }

  process.env.OPENCODE_DB_PATH = opencodeDbPath;
  process.env.TEST_OPENCODE_DB_PATH = opencodeDbPath;
  process.env.TEST_OPENCODE_DB_QUERY_JSON = dbQueryPath;
  process.env.TEST_OPENCODE_EXPORT_DIR = exportDir;
  process.env.PATH = `${binDir}${path.delimiter}${process.env.PATH ?? ""}`;
}
