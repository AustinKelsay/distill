import fs from "node:fs";
import path from "node:path";
import { getDroidHome } from "../../distill-electron/paths";
import { parseJsonlText } from "../../distill-electron/jsonl";
import {
  DiscoveredCapture,
  NormalizedArtifact,
  NormalizedMessage,
  ParsedCapture,
  ParsedCaptureRecord
} from "../../shared/types";
import { CaptureSnapshot } from "../types";

type DroidIndexEntry = {
  title?: string;
  cwd?: string;
  mtime?: number;
  messagesCount?: number;
  tags?: unknown[];
};

function textValue(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function numberValue(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function readDroidSessionsIndex(): Map<string, DroidIndexEntry> {
  const indexPath = path.join(getDroidHome(), "sessions-index.json");
  const sessionsIndex = new Map<string, DroidIndexEntry>();

  try {
    const raw = JSON.parse(fs.readFileSync(indexPath, "utf8")) as unknown;
    if (!raw || typeof raw !== "object") {
      return sessionsIndex;
    }

    const entries = (raw as { entries?: unknown }).entries;
    if (!Array.isArray(entries)) {
      return sessionsIndex;
    }

    for (const row of entries) {
      if (!row || typeof row !== "object") {
        continue;
      }

      const entry = row as {
        sessionId?: unknown;
        title?: unknown;
        cwd?: unknown;
        mtime?: unknown;
        messagesCount?: unknown;
        tags?: unknown;
      };
      const sessionId = textValue(entry.sessionId);
      if (!sessionId) {
        continue;
      }

      sessionsIndex.set(sessionId, {
        title: textValue(entry.title),
        cwd: textValue(entry.cwd),
        mtime: numberValue(entry.mtime),
        messagesCount: numberValue(entry.messagesCount),
        tags: Array.isArray(entry.tags) ? entry.tags : undefined
      });
    }
  } catch {
    return sessionsIndex;
  }

  return sessionsIndex;
}

function isSystemReminder(text: string): boolean {
  return text.startsWith("<system-reminder>");
}

function blockPreview(blockType: string, block: Record<string, unknown>): string | undefined {
  if (blockType === "text") {
    return textValue(block.text);
  }

  if (blockType === "tool_use") {
    return `[tool_use] ${textValue(block.name) ?? "tool"}`;
  }

  if (blockType === "tool_result") {
    const isError = block.is_error === true;
    return isError ? "[tool_result:error]" : "[tool_result]";
  }

  if (blockType === "thinking") {
    const text = textValue(block.thinking);
    return text ? `[thinking] ${text}` : "[thinking]";
  }

  return `[${blockType}]`;
}

function firstUserTitle(messages: NormalizedMessage[]): string | undefined {
  const firstUserMessage = messages.find((message) => message.role === "user");
  return firstUserMessage?.text.split("\n")[0]?.trim().slice(0, 160) || undefined;
}

export function parseDroidCapture(capture: DiscoveredCapture, snapshot: CaptureSnapshot): ParsedCapture {
  const rows = parseJsonlText(snapshot.rawText);
  const sessionsIndex = readDroidSessionsIndex();
  const rawRecords: ParsedCaptureRecord[] = [];
  const messages: NormalizedMessage[] = [];
  const artifacts: NormalizedArtifact[] = [];

  let lineNo = 0;
  let startedAt: string | undefined;
  let updatedAt: string | undefined;
  let sessionId = capture.externalSessionId;
  let sessionTitle: string | undefined;
  let projectPath: string | undefined;
  let owner: string | undefined;
  let sessionVersion: number | undefined;

  for (const row of rows) {
    const rowType = textValue(row.type) ?? "unknown";

    if (rowType === "session_start") {
      sessionId = textValue(row.id) ?? sessionId;
      sessionTitle = textValue(row.title) ?? textValue(row.sessionTitle) ?? sessionTitle;
      projectPath = textValue(row.cwd) ?? projectPath;
      owner = textValue(row.owner) ?? owner;
      sessionVersion = numberValue(row.version) ?? sessionVersion;

      lineNo += 1;
      rawRecords.push({
        lineNo,
        recordType: rowType,
        providerMessageId: textValue(row.id),
        role: "system",
        isMeta: true,
        contentText: sessionTitle,
        contentJson: row,
        metadata: {}
      });
      continue;
    }

    if (rowType !== "message") {
      lineNo += 1;
      rawRecords.push({
        lineNo,
        recordType: rowType,
        isMeta: true,
        contentJson: row,
        metadata: {}
      });
      continue;
    }

    const timestamp = textValue(row.timestamp);
    if (!startedAt || (timestamp && timestamp < startedAt)) {
      startedAt = timestamp ?? startedAt;
    }
    if (!updatedAt || (timestamp && timestamp > updatedAt)) {
      updatedAt = timestamp ?? updatedAt;
    }

    const message =
      row.message && typeof row.message === "object" ? (row.message as Record<string, unknown>) : {};
    const role = message.role === "user" || message.role === "assistant" ? message.role : "assistant";
    const visibility = textValue(message.visibility);
    const providerMessageId = textValue(row.id);
    const parentProviderMessageId = textValue(row.parentId);
    const blocks = Array.isArray(message.content)
      ? message.content.filter(
        (block): block is Record<string, unknown> => Boolean(block) && typeof block === "object"
      )
      : [];

    if (blocks.length === 0) {
      lineNo += 1;
      rawRecords.push({
        lineNo,
        recordType: "message:empty",
        recordTimestamp: timestamp,
        providerMessageId,
        parentProviderMessageId,
        role,
        isMeta: true,
        contentJson: row,
        metadata: {
          visibility
        }
      });
      continue;
    }

    for (const block of blocks) {
      const blockType = textValue(block.type) ?? "unknown";
      const text = blockPreview(blockType, block);
      const shouldSuppressText = role === "user" && blockType === "text" && text ? isSystemReminder(text) : false;
      const isUserOnly = visibility === "user_only";
      const isMeta = blockType !== "text" || shouldSuppressText || isUserOnly;
      lineNo += 1;

      rawRecords.push({
        lineNo,
        recordType: `message:${blockType}`,
        recordTimestamp: timestamp,
        providerMessageId,
        parentProviderMessageId,
        role,
        isMeta,
        contentText: text,
        contentJson: {
          message: row.message ?? {},
          block
        },
        metadata: {
          visibility
        }
      });

      if (blockType === "text" && text && !shouldSuppressText) {
        messages.push({
          sourceLineNo: lineNo,
          externalMessageId: providerMessageId,
          parentExternalMessageId: parentProviderMessageId,
          role,
          text,
          createdAt: timestamp,
          messageKind: isUserOnly ? "meta" : "text",
          metadata: {
            visibility
          }
        });
      } else if (blockType === "tool_use") {
        artifacts.push({
          sourceLineNo: lineNo,
          externalMessageId: providerMessageId,
          kind: "tool_call",
          payload: block
        });
      } else if (blockType === "tool_result") {
        artifacts.push({
          sourceLineNo: lineNo,
          externalMessageId: providerMessageId,
          kind: "tool_result",
          payload: block
        });
      } else if (blockType === "thinking") {
        artifacts.push({
          sourceLineNo: lineNo,
          externalMessageId: providerMessageId,
          kind: "raw_json",
          payload: block
        });
      }
    }
  }

  const sourceExternalSessionId = sessionId;
  const resolvedExternalSessionId = sourceExternalSessionId ?? path.basename(capture.sourcePath, ".jsonl");
  const indexed = sessionsIndex.get(resolvedExternalSessionId);
  const indexedUpdatedAt = indexed?.mtime ? new Date(indexed.mtime).toISOString() : undefined;
  const externalSessionIdProvenance = sourceExternalSessionId
    ? { kind: "source" as const }
    : {
        kind: "synthetic" as const,
        strategy: "capture_path_basename"
      };

  return {
    session: {
      sourceKind: "droid",
      externalSessionId: resolvedExternalSessionId,
      title: sessionTitle ?? indexed?.title ?? firstUserTitle(messages),
      projectPath: projectPath ?? indexed?.cwd,
      startedAt,
      updatedAt: updatedAt ?? indexedUpdatedAt ?? capture.sourceModifiedAt,
      metadata: {
        capturePath: capture.sourcePath,
        owner,
        sessionVersion,
        indexedMessagesCount: indexed?.messagesCount,
        indexedTags: indexed?.tags,
        externalSessionIdProvenance
      }
    },
    messages,
    artifacts,
    rawRecords
  };
}
