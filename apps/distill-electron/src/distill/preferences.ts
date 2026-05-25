import { openDistillDatabase } from "./db";
import { SourceColors } from "../shared/types";

export const DEFAULT_SOURCE_COLORS: SourceColors = {
  codex: "#3dbf9a",
  claude_code: "#d4944a",
  opencode: "#a88cd4"
};

export function getUserPreference(key: string): string | undefined {
  const { db, close } = openDistillDatabase();
  try {
    const row = db.prepare("SELECT value FROM user_preferences WHERE key = ?").get(key) as
      | { value: string }
      | undefined;
    return row?.value;
  } finally {
    close();
  }
}

export function setUserPreference(key: string, value: string): void {
  const { db, close } = openDistillDatabase();
  try {
    db.prepare(
      `INSERT INTO user_preferences (key, value, updated_at)
       VALUES (?, ?, CURRENT_TIMESTAMP)
       ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP`
    ).run(key, value);
  } finally {
    close();
  }
}

export function getSourceColors(): SourceColors {
  const raw = getUserPreference("source_colors");
  if (!raw) return { ...DEFAULT_SOURCE_COLORS };
  try {
    const parsed = JSON.parse(raw) as Record<string, string>;
    return { ...DEFAULT_SOURCE_COLORS, ...parsed };
  } catch {
    return { ...DEFAULT_SOURCE_COLORS };
  }
}

export function setSourceColor(sourceKind: string, color: string): SourceColors {
  const current = getSourceColors();
  current[sourceKind] = color;
  setUserPreference("source_colors", JSON.stringify(current));
  return current;
}
