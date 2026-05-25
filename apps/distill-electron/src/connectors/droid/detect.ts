import path from "node:path";
import { countFiles, countFilesMatching, findExecutable, pathExists } from "../../distill-electron/fs";
import { getDroidHome } from "../../distill-electron/paths";
import { DiscoveredSource, SourcePathCheck } from "../../shared/types";

export function detectDroidSource(): DiscoveredSource {
  const executablePath = findExecutable("droid");
  const dataRoot = getDroidHome();
  const sessionsRoot = path.join(dataRoot, "sessions");
  const sessionsIndex = path.join(dataRoot, "sessions-index.json");

  const checks: SourcePathCheck[] = [
    {
      label: "data_root",
      path: dataRoot,
      exists: pathExists(dataRoot)
    },
    {
      label: "sessions",
      path: sessionsRoot,
      exists: pathExists(sessionsRoot),
      fileCount: countFilesMatching(
        sessionsRoot,
        (filePath) => filePath.endsWith(".jsonl") && !filePath.endsWith(".settings.json")
      )
    },
    {
      label: "sessions_index",
      path: sessionsIndex,
      exists: pathExists(sessionsIndex),
      fileCount: countFiles(sessionsIndex)
    }
  ];

  const hasSessions = checks.some((check) => check.label === "sessions" && check.exists);
  const installStatus =
    executablePath && checks[0].exists && hasSessions
      ? "installed"
      : executablePath || checks.some((check) => check.exists)
        ? "partial"
        : "not_found";

  return {
    kind: "droid",
    displayName: "Factory Droid CLI",
    executablePath,
    dataRoot,
    installStatus,
    checks,
    metadata: {
      primaryCapturePath: sessionsRoot,
      auxiliaryFiles: [sessionsIndex]
    }
  };
}
