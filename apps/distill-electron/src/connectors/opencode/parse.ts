import {
  DiscoveredCapture,
  NormalizedArtifact,
  NormalizedMessage,
  ParsedCapture,
  ParsedCaptureRecord
} from "../../shared/types";
import { CaptureSnapshot } from "../types";
import { openCodeTimestampToIso } from "./common";

type OpenCodeMessageInfo = Record<string, unknown> & {
  id?: string;
  role?: string;
  parentID?: string;
};

type OpenCodePart = Record<string, unknown> & {
  id?: string;
  type?: string;
};

type OpenCodeExportMessage = {
  info?: OpenCodeMessageInfo;
  parts?: OpenCodePart[];
};

type OpenCodeExport = {
  info?: Record<string, unknown>;
  messages?: OpenCodeExportMessage[];
};

function trimText(text: string, maxLength = 400): string {
  const normalized = text.replace(/\s+/g, " ").trim();
  return normalized.length > maxLength ? `${normalized.slice(0, maxLength - 1).trimEnd()}…` : normalized;
}

function textValue(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function readModelInfo(model: unknown, provider: unknown): { model?: string; provider?: string } | undefined {
  const normalizedModel = textValue(model);
  const normalizedProvider = textValue(provider);

  if (!normalizedModel && !normalizedProvider) {
    return undefined;
  }

  return {
    model: normalizedModel,
    provider: normalizedProvider
  };
}

function firstUserText(messages: OpenCodeExportMessage[]): string | undefined {
  for (const message of messages) {
    if (message.info?.role !== "user") {
      continue;
    }

    for (const part of message.parts ?? []) {
      if (part.type !== "text") {
        continue;
      }

      const text = textValue(part.text);
      if (text) {
        return text;
      }
    }
  }

  return undefined;
}

function isGeneratedTitle(title: string | undefined): boolean {
  return Boolean(title && /^New session - \d{4}-\d{2}-\d{2}T/.test(title));
}

function buildToolMessageText(part: OpenCodePart): string {
  const toolName = textValue(part.tool) ?? "unknown";
  const state = part.state && typeof part.state === "object" ? (part.state as Record<string, unknown>) : {};
  const status = textValue(state.status) ?? "pending";

  if (status === "completed") {
    const title = textValue(state.title);
    const output = textValue(state.output);
    const body = [title, output ? trimText(output) : undefined].filter(Boolean).join("\n");
    return body ? `[tool:completed] ${toolName}\n${body}` : `[tool:completed] ${toolName}`;
  }

  if (status === "error") {
    const errorText = textValue(state.error);
    return errorText ? `[tool:error] ${toolName}\n${trimText(errorText)}` : `[tool:error] ${toolName}`;
  }

  return `[tool:${status}] ${toolName}`;
}

function buildMetaMessageText(part: OpenCodePart, messageRole: string): string | undefined {
  if (part.type === "text") {
    return textValue(part.text);
  }

  if (part.type === "reasoning") {
    return textValue(part.text);
  }

  if (part.type === "step-start") {
    return "[step-start]";
  }

  if (part.type === "step-finish") {
    const tokens = part.tokens && typeof part.tokens === "object" ? (part.tokens as Record<string, unknown>) : {};
    const input = typeof tokens.input === "number" ? tokens.input : 0;
    const output = typeof tokens.output === "number" ? tokens.output : 0;
    const reason = textValue(part.reason) ?? "unknown";
    return `[step-finish] reason=${reason} input=${input} output=${output}`;
  }

  if (part.type === "tool") {
    return buildToolMessageText(part);
  }

  if (part.type === "file") {
    const fileName = textValue(part.filename);
    const source =
      part.source && typeof part.source === "object" ? (part.source as Record<string, unknown>) : undefined;
    const sourcePath = source ? textValue(source.path) : undefined;
    return `[file] ${fileName ?? sourcePath ?? textValue(part.url) ?? "attachment"}`;
  }

  if (part.type === "subtask") {
    return `[subtask] ${textValue(part.description) ?? textValue(part.prompt) ?? "subtask"}`;
  }

  if (part.type === "agent") {
    return `[agent] ${textValue(part.name) ?? "agent"}`;
  }

  if (part.type === "patch") {
    const files = Array.isArray(part.files) ? part.files.length : 0;
    return `[patch] ${files} files`;
  }

  if (part.type === "snapshot") {
    return "[snapshot]";
  }

  if (part.type === "retry") {
    const attempt = typeof part.attempt === "number" ? part.attempt : 0;
    const errorText = part.error && typeof part.error === "object"
      ? textValue((part.error as Record<string, unknown>).message)
      : undefined;
    return `[retry] attempt ${attempt}${errorText ? ` ${trimText(errorText, 120)}` : ""}`;
  }

  if (part.type === "compaction") {
    return `[compaction] ${part.auto === true ? "auto" : "manual"}`;
  }

  const type = textValue(part.type) ?? "unknown";
  return `[${type}] ${messageRole}`;
}

function partTimestamp(part: OpenCodePart, info: OpenCodeMessageInfo | undefined): string | undefined {
  const time = part.time && typeof part.time === "object" ? (part.time as Record<string, unknown>) : undefined;
  const start = time?.start;
  if (typeof start === "number") {
    return openCodeTimestampToIso(start);
  }

  const messageTime = info?.time && typeof info.time === "object" ? (info.time as Record<string, unknown>) : undefined;
  return openCodeTimestampToIso(messageTime?.created);
}

function normalizePartRole(part: OpenCodePart, messageRole: string): NormalizedMessage["role"] {
  if (part.type === "tool") {
    return "tool";
  }

  if (messageRole === "user" || messageRole === "assistant" || messageRole === "system") {
    return messageRole;
  }

  return "assistant";
}

function normalizeModelInfo(messages: OpenCodeExportMessage[]): { model?: string; modelProvider?: string } {
  let firstUserModel: { model?: string; provider?: string } | undefined;
  let lastAssistantModel: { model?: string; provider?: string } | undefined;

  for (const message of messages) {
    const info = message.info;
    if (!info) {
      continue;
    }

    if (info.role === "user" && !firstUserModel) {
      const modelInfo = info.model && typeof info.model === "object" ? (info.model as Record<string, unknown>) : {};
      firstUserModel = readModelInfo(modelInfo.modelID, modelInfo.providerID);
    }

    if (info.role === "assistant") {
      const assistantModel = readModelInfo(info.modelID, info.providerID);
      if (assistantModel) {
        lastAssistantModel = assistantModel;
      }
    }
  }

  return {
    model: lastAssistantModel?.model ?? firstUserModel?.model,
    modelProvider: lastAssistantModel?.provider ?? firstUserModel?.provider
  };
}

function pushToolArtifacts(
  artifacts: NormalizedArtifact[],
  sourceLineNo: number,
  partId: string | undefined,
  part: OpenCodePart
): void {
  const state = part.state && typeof part.state === "object" ? (part.state as Record<string, unknown>) : {};
  const payload = {
    name: textValue(part.tool) ?? "unknown",
    callId: textValue(part.callID),
    state,
    metadata: part.metadata && typeof part.metadata === "object" ? part.metadata : undefined
  };

  artifacts.push({
    sourceLineNo,
    externalMessageId: partId,
    kind: "tool_call",
    payload
  });

  const status = textValue(state.status);
  if (status === "completed" || status === "error") {
    artifacts.push({
      sourceLineNo,
      externalMessageId: partId,
      kind: "tool_result",
      payload: {
        name: payload.name,
        status,
        output: state.output,
        error: state.error,
        title: state.title,
        attachments: state.attachments
      }
    });

    const attachments = Array.isArray(state.attachments)
      ? (state.attachments as Array<Record<string, unknown>>)
      : [];

    for (const attachment of attachments) {
      artifacts.push({
        sourceLineNo,
        externalMessageId: partId,
        kind: "file",
        mimeType: textValue(attachment.mime),
        payload: attachment
      });
    }
  }
}

export function parseOpenCodeCapture(capture: DiscoveredCapture, snapshot: CaptureSnapshot): ParsedCapture {
  const exportPayload = JSON.parse(snapshot.rawText) as OpenCodeExport;
  const sessionInfo = exportPayload.info && typeof exportPayload.info === "object"
    ? exportPayload.info
    : {};
  const exportMessages = Array.isArray(exportPayload.messages) ? exportPayload.messages : [];
  const rawRecords: ParsedCaptureRecord[] = [];
  const messages: NormalizedMessage[] = [];
  const artifacts: NormalizedArtifact[] = [];
  const { model, modelProvider } = normalizeModelInfo(exportMessages);
  const generatedTitle = textValue(sessionInfo.title);
  const fallbackTitle = firstUserText(exportMessages)?.split("\n")[0]?.trim().slice(0, 160);
  const title = !generatedTitle || isGeneratedTitle(generatedTitle) ? fallbackTitle : generatedTitle;

  let lineNo = 0;

  for (const exportMessage of exportMessages) {
    const info = exportMessage.info;
    const messageRole = textValue(info?.role) ?? "assistant";
    const parentId = textValue(info?.parentID);

    for (const part of exportMessage.parts ?? []) {
      const partType = textValue(part.type) ?? "unknown";
      const partId = textValue(part.id);
      const text = buildMetaMessageText(part, messageRole);
      lineNo += 1;

      rawRecords.push({
        lineNo,
        recordType: `message:${messageRole}:${partType}`,
        recordTimestamp: partTimestamp(part, info),
        providerMessageId: textValue(info?.id),
        parentProviderMessageId: parentId,
        role: normalizePartRole(part, messageRole),
        isMeta: partType !== "text",
        contentText: text,
        contentJson: {
          info: exportMessage.info ?? {},
          part
        },
        metadata: {}
      });

      if (text) {
        messages.push({
          sourceLineNo: lineNo,
          externalMessageId: partId,
          parentExternalMessageId: parentId,
          role: normalizePartRole(part, messageRole),
          text,
          createdAt: partTimestamp(part, info),
          messageKind: partType === "text" ? "text" : "meta",
          metadata: {
            partType
          }
        });
      }

      if (partType === "tool") {
        pushToolArtifacts(artifacts, lineNo, partId, part);
      } else if (partType === "file") {
        artifacts.push({
          sourceLineNo: lineNo,
          externalMessageId: partId,
          kind: "file",
          mimeType: textValue(part.mime),
          payload: part
        });
      } else if (!["text", "reasoning", "step-start", "step-finish"].includes(partType)) {
        artifacts.push({
          sourceLineNo: lineNo,
          externalMessageId: partId,
          kind: "raw_json",
          payload: part
        });
      }
    }
  }

  const externalSessionId = textValue(sessionInfo.id) ?? capture.externalSessionId;
  const resolvedExternalSessionId = externalSessionId ?? capture.sourcePath;
  const externalSessionIdProvenance = externalSessionId
    ? { kind: "source" as const }
    : {
        kind: "synthetic" as const,
        strategy: "capture_source_path"
      };

  return {
    session: {
      sourceKind: "opencode",
      externalSessionId: resolvedExternalSessionId,
      title,
      projectPath: textValue(sessionInfo.directory),
      sourceUrl: textValue(capture.metadata.shareUrl),
      model,
      modelProvider,
      cliVersion: textValue(sessionInfo.version),
      startedAt: openCodeTimestampToIso(
        sessionInfo.time && typeof sessionInfo.time === "object"
          ? (sessionInfo.time as Record<string, unknown>).created
          : undefined
      ),
      updatedAt: openCodeTimestampToIso(
        sessionInfo.time && typeof sessionInfo.time === "object"
          ? (sessionInfo.time as Record<string, unknown>).updated
          : capture.metadata.timeUpdated
      ) ?? capture.sourceModifiedAt,
      metadata: {
        slug: textValue(sessionInfo.slug),
        projectID: textValue(sessionInfo.projectID),
        archiveTimestamp: capture.metadata.timeArchived ?? null,
        exportSource: "opencode export",
        originalTitle: !title || title === generatedTitle ? undefined : generatedTitle,
        externalSessionIdProvenance
      }
    },
    messages,
    artifacts,
    rawRecords
  };
}
