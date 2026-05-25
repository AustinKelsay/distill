import fs from "node:fs";
import path from "node:path";
import { listFilesRecursive } from "../../distill-electron/fs";
import { getDroidHome } from "../../distill-electron/paths";
import { DiscoveredCapture } from "../../shared/types";

function extractSessionId(filePath: string): string | undefined {
  const baseName = path.basename(filePath, ".jsonl");
  const match = baseName.match(/^([0-9a-f-]+)$/i);
  return match?.[1];
}

export function discoverDroidCaptures(): DiscoveredCapture[] {
  const sessionsRoot = path.join(getDroidHome(), "sessions");

  return listFilesRecursive(sessionsRoot)
    .filter((filePath) => filePath.endsWith(".jsonl"))
    .flatMap((filePath) => {
      let stat;
      try {
        stat = fs.statSync(filePath);
      } catch {
        return [];
      }

      return [{
        sourceKind: "droid",
        captureKind: "session",
        sourcePath: filePath,
        externalSessionId: extractSessionId(filePath),
        sourceModifiedAt: stat.mtime.toISOString(),
        sourceSizeBytes: stat.size,
        metadata: {
          sessionDirectory: path.dirname(filePath)
        }
      } satisfies DiscoveredCapture];
    });
}
