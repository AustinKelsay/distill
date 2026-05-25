import { countFiles, countFilesMatching, findExecutable, pathExists } from "../../distill/fs";
import { getCodexHome } from "../../distill/paths";
import { DiscoveredSource, SourcePathCheck } from "../../shared/types";

export function detectCodexSource(): DiscoveredSource {
  const executablePath = findExecutable("codex");
  const dataRoot = getCodexHome();
  const archivedSessions = `${dataRoot}/archived_sessions`;
  const liveSessions = `${dataRoot}/sessions`;
  const sessionIndex = `${dataRoot}/session_index.jsonl`;
  const history = `${dataRoot}/history.jsonl`;

  const dataRootCheck: SourcePathCheck = {
    label: "data_root",
    path: dataRoot,
    exists: pathExists(dataRoot)
  };
  const archivedSessionsCheck: SourcePathCheck = {
    label: "archived_sessions",
    path: archivedSessions,
    exists: pathExists(archivedSessions),
    fileCount: countFilesMatching(archivedSessions, (filePath) => filePath.endsWith(".jsonl"))
  };
  const liveSessionsCheck: SourcePathCheck = {
    label: "sessions",
    path: liveSessions,
    exists: pathExists(liveSessions),
    fileCount: countFilesMatching(liveSessions, (filePath) => filePath.endsWith(".jsonl"))
  };
  const sessionIndexCheck: SourcePathCheck = {
    label: "session_index",
    path: sessionIndex,
    exists: pathExists(sessionIndex),
    fileCount: countFiles(sessionIndex)
  };
  const historyCheck: SourcePathCheck = {
    label: "history",
    path: history,
    exists: pathExists(history),
    fileCount: countFiles(history)
  };

  const checks: SourcePathCheck[] = [
    dataRootCheck,
    archivedSessionsCheck,
    liveSessionsCheck,
    sessionIndexCheck,
    historyCheck
  ];
  const hasDataRoot = dataRootCheck.exists;
  const hasArchivedSessions = archivedSessionsCheck.exists;
  const hasLiveSessions = liveSessionsCheck.exists;
  const primaryCapturePath = hasLiveSessions ? liveSessions : archivedSessions;

  const installStatus =
    executablePath && hasDataRoot && (hasArchivedSessions || hasLiveSessions)
      ? "installed"
      : executablePath || checks.some((check) => check.exists)
        ? "partial"
        : "not_found";

  return {
    kind: "codex",
    displayName: "OpenAI Codex CLI",
    executablePath,
    dataRoot,
    installStatus,
    checks,
    metadata: {
      primaryCapturePath,
      capturePaths: Array.from(new Set([primaryCapturePath, archivedSessions, liveSessions])),
      auxiliaryFiles: [sessionIndex, history]
    }
  };
}
