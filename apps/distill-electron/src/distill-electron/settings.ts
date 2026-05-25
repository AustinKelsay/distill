import { sourceConnectors } from "../connectors";
import { getOpenCodeDatabasePath } from "../connectors/opencode/common";
import { getDefaultLabelNames } from "./curation";
import {
  getClaudeHome,
  getCodexHome,
  getDroidHome,
  getDistillElectronDatabasePath,
  getDistillElectronHome,
  getOpenCodeConfigDir,
  getOpenCodeStateDir
} from "./paths";
import { getSourceColors } from "./preferences";
import { AppSettingsSnapshot } from "../shared/types";

export const BACKGROUND_SYNC_INTERVAL_MINUTES = 2;
const sourceKinds = sourceConnectors.map((connector) => connector.kind);

export function getAppSettingsSnapshot(): AppSettingsSnapshot {
  return {
    distillElectronHome: getDistillElectronHome(),
    databasePath: getDistillElectronDatabasePath(),
    codexHome: getCodexHome(),
    claudeHome: getClaudeHome(),
    droidHome: getDroidHome(),
    opencodeDatabasePath: getOpenCodeDatabasePath(),
    opencodeConfigDir: getOpenCodeConfigDir(),
    opencodeStateDir: getOpenCodeStateDir(),
    sourceKinds,
    defaultLabels: getDefaultLabelNames(),
    backgroundSyncIntervalMinutes: BACKGROUND_SYNC_INTERVAL_MINUTES,
    envOverrides: {
      distillElectronHome: Boolean(process.env.DISTILL_ELECTRON_HOME),
      codexHome: Boolean(process.env.CODEX_HOME),
      claudeHome: Boolean(process.env.CLAUDE_HOME),
      droidHome: Boolean(process.env.DROID_HOME),
      opencodeDbPath: Boolean(process.env.OPENCODE_DB_PATH),
      opencodeConfigDir: Boolean(process.env.OPENCODE_CONFIG_DIR),
      opencodeStateDir: Boolean(process.env.OPENCODE_STATE_DIR)
    },
    sourceColors: getSourceColors()
  };
}
