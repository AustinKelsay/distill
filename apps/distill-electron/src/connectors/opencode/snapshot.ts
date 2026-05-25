import { getTextSha256 } from "../../distill/fs";
import { DiscoveredCapture } from "../../shared/types";
import { CaptureSnapshot } from "../types";
import { runOpenCodeCommandToFile } from "./common";

function extractJsonExport(output: string): string {
  const index = output.indexOf("{");
  if (index < 0) {
    throw new Error("OpenCode export did not return JSON");
  }

  const jsonText = output.slice(index).trim();
  JSON.parse(jsonText);
  return jsonText;
}

export function snapshotOpenCodeCapture(capture: DiscoveredCapture): CaptureSnapshot {
  const sessionId = capture.externalSessionId;
  if (!sessionId) {
    throw new Error("OpenCode capture is missing an external session ID");
  }

  const rawText = extractJsonExport(runOpenCodeCommandToFile(["export", sessionId]));

  return {
    rawText,
    rawSha256: getTextSha256(rawText),
    sourceModifiedAt: capture.sourceModifiedAt,
    sourceSizeBytes: Buffer.byteLength(rawText)
  };
}
