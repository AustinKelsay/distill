import fs from "node:fs";
import path from "node:path";
import { listFilesRecursive } from "../../distill/fs";
import { getClaudeHome } from "../../distill/paths";
import { DiscoveredCapture } from "../../shared/types";

function extractSessionId(filePath: string): string | undefined {
  const match = path.basename(filePath).match(/^([0-9a-f-]+)\.jsonl$/i);
  return match?.[1];
}

export function discoverClaudeCodeCaptures(): DiscoveredCapture[] {
  const projectsPath = path.join(getClaudeHome(), "projects");

  return listFilesRecursive(projectsPath)
    .filter((filePath) => filePath.endsWith(".jsonl"))
    .map((filePath) => {
      const stat = fs.statSync(filePath);

      return {
        sourceKind: "claude_code",
        captureKind: "project_session",
        sourcePath: filePath,
        externalSessionId: extractSessionId(filePath),
        sourceModifiedAt: stat.mtime.toISOString(),
        sourceSizeBytes: stat.size,
        metadata: {
          projectFolder: path.dirname(filePath)
        }
      };
    });
}
