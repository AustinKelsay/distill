import fs from "node:fs";
import { getTextSha256 } from "../../distill/fs";
import { DiscoveredCapture } from "../../shared/types";
import { CaptureSnapshot } from "../types";

export function snapshotClaudeCodeCapture(capture: DiscoveredCapture): CaptureSnapshot {
  const rawText = fs.readFileSync(capture.sourcePath, "utf8");

  return {
    rawText,
    rawSha256: getTextSha256(rawText),
    sourceModifiedAt: capture.sourceModifiedAt,
    sourceSizeBytes: Buffer.byteLength(rawText)
  };
}
