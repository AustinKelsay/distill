import { countFiles, findExecutable, pathExists } from "../../distill/fs";
import { getOpenCodeConfigDir, getOpenCodeStateDir } from "../../distill/paths";
import { DiscoveredSource, SourcePathCheck } from "../../shared/types";
import {
  getOpenCodeDatabasePath,
  getOpenCodeDatabaseRoot,
  getOpenCodePromptHistoryPath
} from "./common";

export function detectOpenCodeSource(): DiscoveredSource {
  const executablePath = findExecutable("opencode");
  const databasePath = getOpenCodeDatabasePath(executablePath);
  const dataRoot = getOpenCodeDatabaseRoot(executablePath);
  const configDir = getOpenCodeConfigDir();
  const stateDir = getOpenCodeStateDir();
  const promptHistoryPath = getOpenCodePromptHistoryPath();

  const checks: SourcePathCheck[] = [
    {
      label: "database",
      path: databasePath,
      exists: pathExists(databasePath),
      fileCount: countFiles(databasePath)
    },
    {
      label: "config_dir",
      path: configDir,
      exists: pathExists(configDir),
      fileCount: countFiles(configDir)
    },
    {
      label: "prompt_history",
      path: promptHistoryPath,
      exists: pathExists(promptHistoryPath),
      fileCount: countFiles(promptHistoryPath)
    }
  ];

  const installStatus =
    executablePath && checks[0].exists
      ? "installed"
      : executablePath || checks.some((check) => check.exists)
        ? "partial"
        : "not_found";

  return {
    kind: "opencode",
    displayName: "OpenCode",
    executablePath,
    dataRoot,
    installStatus,
    checks,
    metadata: {
      databasePath,
      configDir,
      stateDir,
      discoveryStrategy: "opencode db --format json",
      exportStrategy: "opencode export <sessionId>"
    }
  };
}
