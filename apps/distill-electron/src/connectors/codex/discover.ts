import fs from "node:fs";
import path from "node:path";
import { listFilesRecursive } from "../../distill/fs";
import { getCodexHome } from "../../distill/paths";
import { DiscoveredCapture } from "../../shared/types";

function extractSessionId(filePath: string): string | undefined {
  const baseName = path.basename(filePath, ".jsonl");
  const match = baseName.match(/^rollout-\d{4}-\d{2}-\d{2}(?:T\d{2}-\d{2}-\d{2})?-(.+)$/);
  return match?.[1];
}

export function discoverCodexCaptures(): DiscoveredCapture[] {
  const codexHome = getCodexHome();
  const archivedSessionsPath = path.join(codexHome, "archived_sessions");
  const sessionsPath = path.join(codexHome, "sessions");

  const archivedCaptures = listFilesRecursive(archivedSessionsPath)
    .filter((filePath) => filePath.endsWith(".jsonl"))
    .map((filePath) => {
      const stat = fs.statSync(filePath);

      return {
        sourceKind: "codex",
        captureKind: "archived_session",
        sourcePath: filePath,
        externalSessionId: extractSessionId(filePath),
        sourceModifiedAt: stat.mtime.toISOString(),
        sourceSizeBytes: stat.size,
        metadata: {}
      } satisfies DiscoveredCapture;
    });
  const liveCaptures = listFilesRecursive(sessionsPath)
    .filter((filePath) => filePath.endsWith(".jsonl"))
    .map((filePath) => {
      const stat = fs.statSync(filePath);

      return {
        sourceKind: "codex",
        captureKind: "live_session",
        sourcePath: filePath,
        externalSessionId: extractSessionId(filePath),
        sourceModifiedAt: stat.mtime.toISOString(),
        sourceSizeBytes: stat.size,
        metadata: {}
      } satisfies DiscoveredCapture;
    });

  const capturesBySessionId = new Map<string, DiscoveredCapture>();
  const capturesWithoutSessionId: DiscoveredCapture[] = [];

  for (const capture of [...archivedCaptures, ...liveCaptures]) {
    if (!capture.externalSessionId) {
      capturesWithoutSessionId.push(capture);
      continue;
    }

    capturesBySessionId.set(capture.externalSessionId, capture);
  }

  return [...capturesBySessionId.values(), ...capturesWithoutSessionId]
    .sort((left, right) => left.sourcePath.localeCompare(right.sourcePath));
}
