import fs from "node:fs";

export function readJsonl(filePath: string): Array<Record<string, unknown>> {
  const text = fs.readFileSync(filePath, "utf8");
  return parseJsonlText(text);
}

export function parseJsonlText(text: string): Array<Record<string, unknown>> {
  if (!text.trim()) {
    return [];
  }

  return text
    .split(/\r?\n/)
    .filter((line) => line.trim().length > 0)
    .map((line) => JSON.parse(line) as Record<string, unknown>);
}
