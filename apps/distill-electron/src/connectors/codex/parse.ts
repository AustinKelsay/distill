import path from "node:path";
import { getCodexHome } from "../../distill/paths";
import { parseJsonlText, readJsonl } from "../../distill/jsonl";
import {
  DiscoveredCapture,
  NormalizedMessage,
  ParsedCapture,
  ParsedCaptureRecord
} from "../../shared/types";
import { CaptureSnapshot } from "../types";

function readCodexSessionIndex(): Map<string, { threadName?: string; updatedAt?: string }> {
  const indexPath = path.join(getCodexHome(), "session_index.jsonl");
  const map = new Map<string, { threadName?: string; updatedAt?: string }>();

  try {
    for (const row of readJsonl(indexPath)) {
      const id = typeof row.id === "string" ? row.id : undefined;
      if (!id) {
        continue;
      }

      map.set(id, {
        threadName: typeof row.thread_name === "string" ? row.thread_name : undefined,
        updatedAt: typeof row.updated_at === "string" ? row.updated_at : undefined
      });
    }
  } catch {
    return map;
  }

  return map;
}

function extractMessageText(content: unknown): string {
  if (!Array.isArray(content)) {
    return "";
  }

  return content
    .flatMap((item) => {
      if (!item || typeof item !== "object") {
        return [];
      }

      const text = (item as { text?: unknown }).text;
      return typeof text === "string" ? [text.trim()] : [];
    })
    .filter(Boolean)
    .join("\n\n")
    .trim();
}

function shouldSkipCodexMessage(role: string | undefined, text: string): boolean {
  if (!role || !text) {
    return true;
  }

  if (role === "developer") {
    return true;
  }

  return (
    text.startsWith("<environment_context>") ||
    text.startsWith("<local-command-caveat>") ||
    text.startsWith("<permissions instructions>") ||
    text.startsWith("<collaboration_mode>") ||
    text.startsWith("<skills_instructions>") ||
    text.startsWith("<image ") ||
    text.startsWith("# AGENTS.md instructions for ") ||
    text.startsWith("# CLAUDE.md instructions for ") ||
    text.startsWith("# Repository Guidelines") ||
    text.includes("\n<INSTRUCTIONS>\n")
  );
}

function pickCodexTitle(
  externalSessionId: string | undefined,
  sessionIndex: Map<string, { threadName?: string; updatedAt?: string }>,
  messages: NormalizedMessage[]
): string | undefined {
  const indexedTitle = externalSessionId ? sessionIndex.get(externalSessionId)?.threadName : undefined;
  if (indexedTitle?.trim()) {
    return indexedTitle.trim();
  }

  const firstUserMessage = messages.find((message) => message.role === "user");
  return firstUserMessage?.text.split("\n")[0]?.trim().slice(0, 160) || undefined;
}

export function parseCodexCapture(capture: DiscoveredCapture, snapshot: CaptureSnapshot): ParsedCapture {
  const rows = parseJsonlText(snapshot.rawText);
  const sessionIndex = readCodexSessionIndex();
  const rawRecords: ParsedCaptureRecord[] = [];
  const messages: NormalizedMessage[] = [];

  let startedAt: string | undefined;
  let updatedAt: string | undefined;
  let externalSessionId = capture.externalSessionId;
  let projectPath: string | undefined;
  let modelProvider: string | undefined;
  let cliVersion: string | undefined;
  let model: string | undefined;

  rows.forEach((row, index) => {
    const type = typeof row.type === "string" ? row.type : "unknown";
    const timestamp = typeof row.timestamp === "string" ? row.timestamp : undefined;
    const payload = row.payload && typeof row.payload === "object" ? (row.payload as Record<string, unknown>) : {};
    const payloadType = typeof payload.type === "string" ? payload.type : undefined;
    const role = typeof payload.role === "string" ? payload.role : undefined;
    const contentText = extractMessageText(payload.content);

    if (!updatedAt || (timestamp && timestamp > updatedAt)) {
      updatedAt = timestamp ?? updatedAt;
    }

    if (type === "session_meta") {
      externalSessionId =
        typeof payload.id === "string" ? payload.id : externalSessionId;
      startedAt = typeof payload.timestamp === "string" ? payload.timestamp : startedAt;
      projectPath = typeof payload.cwd === "string" ? payload.cwd : projectPath;
      modelProvider =
        typeof payload.model_provider === "string" ? payload.model_provider : modelProvider;
      cliVersion = typeof payload.cli_version === "string" ? payload.cli_version : cliVersion;
    }

    if (type === "turn_context" && !model) {
      model = typeof payload.model === "string" ? payload.model : model;
    }

    rawRecords.push({
      lineNo: index + 1,
      recordType: payloadType ? `${type}:${payloadType}` : type,
      recordTimestamp: timestamp,
      role,
      isMeta: !(type === "response_item" && payloadType === "message" && ["user", "assistant"].includes(role ?? "")),
      contentText: contentText || undefined,
      contentJson: row,
      metadata: {}
    });

    if (type === "response_item" && payloadType === "message" && ["user", "assistant"].includes(role ?? "")) {
      if (!shouldSkipCodexMessage(role, contentText)) {
        messages.push({
          sourceLineNo: index + 1,
          role: role as "user" | "assistant",
          text: contentText,
          createdAt: timestamp,
          messageKind: "text",
          metadata: {}
        });
      }
    }
  });

  const sessionMeta = externalSessionId ? sessionIndex.get(externalSessionId) : undefined;
  const resolvedExternalSessionId = externalSessionId ?? path.basename(capture.sourcePath);
  const externalSessionIdProvenance = externalSessionId
    ? { kind: "source" as const }
    : {
        kind: "synthetic" as const,
        strategy: "capture_path_basename"
      };

  return {
    session: {
      sourceKind: "codex",
      externalSessionId: resolvedExternalSessionId,
      title: pickCodexTitle(externalSessionId, sessionIndex, messages),
      projectPath,
      model,
      modelProvider,
      cliVersion,
      startedAt,
      updatedAt: sessionMeta?.updatedAt ?? updatedAt,
      metadata: {
        capturePath: capture.sourcePath,
        externalSessionIdProvenance
      }
    },
    messages,
    artifacts: [],
    rawRecords
  };
}
