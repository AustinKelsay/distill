import path from "node:path";
import { getClaudeHome } from "../../distill/paths";
import { parseJsonlText, readJsonl } from "../../distill/jsonl";
import {
  DiscoveredCapture,
  NormalizedArtifact,
  NormalizedMessage,
  ParsedCapture,
  ParsedCaptureRecord
} from "../../shared/types";
import { CaptureSnapshot } from "../types";

function readClaudeHistoryIndex(): Map<string, string> {
  const historyPath = path.join(getClaudeHome(), "history.jsonl");
  const map = new Map<string, string>();

  try {
    for (const row of readJsonl(historyPath)) {
      const sessionId = typeof row.sessionId === "string" ? row.sessionId : undefined;
      const display = typeof row.display === "string" ? row.display : undefined;
      if (sessionId && display && !map.has(sessionId)) {
        map.set(sessionId, display);
      }
    }
  } catch {
    return map;
  }

  return map;
}

function normalizeContentBlocks(content: unknown): Array<Record<string, unknown>> {
  if (typeof content === "string") {
    return [{ type: "text", text: content }];
  }

  if (Array.isArray(content)) {
    return content.filter((item): item is Record<string, unknown> => Boolean(item) && typeof item === "object");
  }

  return [];
}

function extractTextBlocks(blocks: Array<Record<string, unknown>>): string {
  return blocks
    .flatMap((block) => {
      const type = typeof block.type === "string" ? block.type : undefined;
      const text = typeof block.text === "string" ? block.text : undefined;
      return type === "text" && text ? [text.trim()] : [];
    })
    .filter(Boolean)
    .join("\n\n")
    .trim();
}

function isSuppressedClaudeText(text: string): boolean {
  return (
    text.startsWith("<local-command-caveat>") ||
    text.startsWith("<command-name>") ||
    text.startsWith("<local-command-stdout>") ||
    text.startsWith("[Image: original ")
  );
}

function pickClaudeTitle(
  sessionId: string | undefined,
  historyIndex: Map<string, string>,
  messages: NormalizedMessage[]
): string | undefined {
  if (sessionId) {
    const historyTitle = historyIndex.get(sessionId);
    if (historyTitle && !isSuppressedClaudeText(historyTitle)) {
      return historyTitle;
    }
  }

  const firstUserMessage = messages.find((message) => message.role === "user");
  if (!firstUserMessage) {
    return undefined;
  }

  return firstUserMessage.text.split("\n")[0]?.trim().slice(0, 160) || undefined;
}

export function parseClaudeCodeCapture(capture: DiscoveredCapture, snapshot: CaptureSnapshot): ParsedCapture {
  const rows = parseJsonlText(snapshot.rawText);
  const historyIndex = readClaudeHistoryIndex();
  const rawRecords: ParsedCaptureRecord[] = [];
  const messages: NormalizedMessage[] = [];
  const artifacts: NormalizedArtifact[] = [];

  let sessionId = capture.externalSessionId;
  let startedAt: string | undefined;
  let updatedAt: string | undefined;
  let projectPath =
    typeof capture.metadata.projectFolder === "string" ? capture.metadata.projectFolder : undefined;
  let gitBranch: string | undefined;

  rows.forEach((row, index) => {
    const type = typeof row.type === "string" ? row.type : "unknown";
    const timestamp = typeof row.timestamp === "string" ? row.timestamp : undefined;
    const uuid = typeof row.uuid === "string" ? row.uuid : undefined;
    const parentUuid = typeof row.parentUuid === "string" ? row.parentUuid : undefined;
    const recordSessionId = typeof row.sessionId === "string" ? row.sessionId : undefined;
    const cwd = typeof row.cwd === "string" ? row.cwd : undefined;
    const branch = typeof row.gitBranch === "string" ? row.gitBranch : undefined;
    const isMeta = row.isMeta === true;
    const message =
      row.message && typeof row.message === "object" ? (row.message as Record<string, unknown>) : undefined;
    const role = typeof message?.role === "string" ? message.role : undefined;
    const blocks = normalizeContentBlocks(message?.content);
    const contentText = extractTextBlocks(blocks);

    sessionId = recordSessionId ?? sessionId;
    projectPath = cwd ?? projectPath;
    gitBranch = branch ?? gitBranch;

    if (!startedAt || (timestamp && timestamp < startedAt)) {
      startedAt = timestamp ?? startedAt;
    }

    if (!updatedAt || (timestamp && timestamp > updatedAt)) {
      updatedAt = timestamp ?? updatedAt;
    }

    rawRecords.push({
      lineNo: index + 1,
      recordType: type,
      recordTimestamp: timestamp,
      providerMessageId: uuid,
      parentProviderMessageId: parentUuid,
      role,
      isMeta,
      contentText: contentText || undefined,
      contentJson: row,
      metadata: {}
    });

    if ((type === "user" || type === "assistant") && !isMeta && role && contentText && !isSuppressedClaudeText(contentText)) {
      messages.push({
        sourceLineNo: index + 1,
        externalMessageId: uuid,
        parentExternalMessageId: parentUuid,
        role: role as "user" | "assistant",
        text: contentText,
        createdAt: timestamp,
        messageKind: "text",
        metadata: {}
      });
    }

    for (const block of blocks) {
      const blockType = typeof block.type === "string" ? block.type : undefined;
      if (blockType === "image") {
        artifacts.push({
          sourceLineNo: index + 1,
          externalMessageId: uuid,
          kind: "image",
          mimeType:
            block.source && typeof block.source === "object" && typeof (block.source as { media_type?: unknown }).media_type === "string"
              ? ((block.source as { media_type: string }).media_type)
              : undefined,
          payload: block
        });
      }

      if (blockType === "tool_use") {
        artifacts.push({
          sourceLineNo: index + 1,
          externalMessageId: uuid,
          kind: "tool_call",
          payload: block
        });
      }

      if (blockType === "tool_result") {
        artifacts.push({
          sourceLineNo: index + 1,
          externalMessageId: uuid,
          kind: "tool_result",
          payload: block
        });
      }
    }
  });

  const resolvedSessionId = sessionId ?? path.basename(capture.sourcePath, ".jsonl");
  const externalSessionIdProvenance = sessionId
    ? { kind: "source" as const }
    : {
        kind: "synthetic" as const,
        strategy: "capture_path_basename"
      };

  return {
    session: {
      sourceKind: "claude_code",
      externalSessionId: resolvedSessionId,
      title: pickClaudeTitle(resolvedSessionId, historyIndex, messages),
      projectPath,
      gitBranch,
      startedAt,
      updatedAt,
      metadata: {
        capturePath: capture.sourcePath,
        externalSessionIdProvenance
      }
    },
    messages,
    artifacts,
    rawRecords
  };
}
