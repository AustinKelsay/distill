import { countFiles, countFilesMatching, findExecutable, pathExists } from "../../distill/fs";
import { getClaudeHome } from "../../distill/paths";
import { DiscoveredSource, SourcePathCheck } from "../../shared/types";

export function detectClaudeCodeSource(): DiscoveredSource {
  const executablePath = findExecutable("claude");
  const dataRoot = getClaudeHome();
  const projectsPath = `${dataRoot}/projects`;
  const history = `${dataRoot}/history.jsonl`;
  const settings = `${dataRoot}/settings.json`;

  const checks: SourcePathCheck[] = [
    {
      label: "data_root",
      path: dataRoot,
      exists: pathExists(dataRoot)
    },
    {
      label: "projects",
      path: projectsPath,
      exists: pathExists(projectsPath),
      fileCount: countFilesMatching(projectsPath, (filePath) => filePath.endsWith(".jsonl"))
    },
    {
      label: "history",
      path: history,
      exists: pathExists(history),
      fileCount: countFiles(history)
    },
    {
      label: "settings",
      path: settings,
      exists: pathExists(settings),
      fileCount: countFiles(settings)
    }
  ];

  const installStatus =
    executablePath && checks[0].exists && checks[1].exists
      ? "installed"
      : executablePath || checks.some((check) => check.exists)
        ? "partial"
        : "not_found";

  return {
    kind: "claude_code",
    displayName: "Claude Code",
    executablePath,
    dataRoot,
    installStatus,
    checks,
    metadata: {
      primaryCapturePath: projectsPath,
      auxiliaryFiles: [history, settings]
    }
  };
}
