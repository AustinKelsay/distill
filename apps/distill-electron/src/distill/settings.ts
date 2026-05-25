import { sourceConnectors } from "../connectors";
import { getOpenCodeDatabasePath } from "../connectors/opencode/common";
import { getDefaultLabelNames } from "./curation";
import {
  getClaudeHome,
  getCodexHome,
  getDistillDatabasePath,
  getDistillHome,
  getOpenCodeConfigDir,
  getOpenCodeStateDir
} from "./paths";
import { getSourceColors } from "./preferences";
import { AppSettingsSnapshot } from "../shared/types";

export const BACKGROUND_SYNC_INTERVAL_MINUTES = 2;
const sourceKinds = sourceConnectors.map((connector) => connector.kind);

export function getAppSettingsSnapshot(): AppSettingsSnapshot {
  return {
    distillHome: getDistillHome(),
    databasePath: getDistillDatabasePath(),
    codexHome: getCodexHome(),
    claudeHome: getClaudeHome(),
    opencodeDatabasePath: getOpenCodeDatabasePath(),
    opencodeConfigDir: getOpenCodeConfigDir(),
    opencodeStateDir: getOpenCodeStateDir(),
    sourceKinds,
    defaultLabels: getDefaultLabelNames(),
    backgroundSyncIntervalMinutes: BACKGROUND_SYNC_INTERVAL_MINUTES,
    envOverrides: {
      distillHome: Boolean(process.env.DISTILL_HOME),
      codexHome: Boolean(process.env.CODEX_HOME),
      claudeHome: Boolean(process.env.CLAUDE_HOME),
      opencodeDbPath: Boolean(process.env.OPENCODE_DB_PATH),
      opencodeConfigDir: Boolean(process.env.OPENCODE_CONFIG_DIR),
      opencodeStateDir: Boolean(process.env.OPENCODE_STATE_DIR)
    },
    sourceColors: getSourceColors()
  };
}
