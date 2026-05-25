import path from "node:path";

export function getHomeDirectory(): string {
  const home = process.env.HOME;
  if (!home) {
    throw new Error("HOME is not set");
  }

  return home;
}

export function getDistillHome(): string {
  return process.env.DISTILL_HOME ?? path.join(getHomeDirectory(), ".distill");
}

export function getDistillDatabasePath(): string {
  return path.join(getDistillHome(), "distill.db");
}

export function getCodexHome(): string {
  return process.env.CODEX_HOME ?? path.join(getHomeDirectory(), ".codex");
}

export function getClaudeHome(): string {
  return process.env.CLAUDE_HOME ?? path.join(getHomeDirectory(), ".claude");
}

export function getOpenCodeConfigDir(): string {
  return process.env.OPENCODE_CONFIG_DIR ?? path.join(getHomeDirectory(), ".config", "opencode");
}

export function getOpenCodeStateDir(): string {
  return process.env.OPENCODE_STATE_DIR ?? path.join(getHomeDirectory(), ".local", "state", "opencode");
}

export function getOpenCodeDefaultDatabasePath(): string {
  return process.env.OPENCODE_DB_PATH ?? path.join(getHomeDirectory(), ".local", "share", "opencode", "opencode.db");
}
