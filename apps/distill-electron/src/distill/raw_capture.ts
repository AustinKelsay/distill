import fs from "node:fs";
import path from "node:path";
import { CaptureSnapshot } from "../connectors/types";
import { CaptureContentRef, DiscoveredCapture } from "../shared/types";
import { ensureDirectory } from "./fs";
import { getDistillHome } from "./paths";

const INLINE_CAPTURE_MAX_BYTES = 64 * 1024;

function captureMediaType(capture: DiscoveredCapture): string {
  if (capture.sourceKind === "opencode") {
    return "application/json; charset=utf-8";
  }

  return "application/x-ndjson; charset=utf-8";
}

function captureExtension(mediaType: string): string {
  if (mediaType.startsWith("application/json")) {
    return ".json";
  }

  if (mediaType.startsWith("application/x-ndjson")) {
    return ".jsonl";
  }

  return ".txt";
}

export function getInlineCaptureMaxBytes(): number {
  return INLINE_CAPTURE_MAX_BYTES;
}

export function resolveCaptureBlobPath(blobPath: string): string {
  if (path.isAbsolute(blobPath)) {
    throw new Error(`Capture blobPath must be relative to Distill blobs: ${blobPath}`);
  }

  const blobsRoot = path.resolve(getDistillHome(), "blobs");
  const resolvedPath = path.resolve(blobsRoot, path.normalize(blobPath));
  const relativePath = path.relative(blobsRoot, resolvedPath);
  if (!relativePath || relativePath.startsWith("..") || path.isAbsolute(relativePath)) {
    throw new Error(`Capture blobPath escapes the Distill blob root: ${blobPath}`);
  }

  return resolvedPath;
}

export function persistCaptureContent(
  capture: DiscoveredCapture,
  snapshot: CaptureSnapshot
): CaptureContentRef {
  const mediaType = captureMediaType(capture);
  const byteSize = snapshot.sourceSizeBytes ?? Buffer.byteLength(snapshot.rawText);

  if (byteSize <= INLINE_CAPTURE_MAX_BYTES) {
    return {
      kind: "inline",
      mediaType,
      text: snapshot.rawText,
      sha256: snapshot.rawSha256,
      byteSize
    };
  }

  const extension = captureExtension(mediaType);
  const blobPath = path.join(
    "captures",
    snapshot.rawSha256.slice(0, 2),
    `${snapshot.rawSha256}${extension}`
  );
  const absolutePath = resolveCaptureBlobPath(blobPath);
  ensureDirectory(path.dirname(absolutePath));

  if (!fs.existsSync(absolutePath)) {
    fs.writeFileSync(absolutePath, snapshot.rawText);
  }

  return {
    kind: "blob",
    mediaType,
    blobPath,
    sha256: snapshot.rawSha256,
    byteSize
  };
}

export function readCaptureContentText(contentRef: CaptureContentRef): string {
  if (contentRef.kind === "inline") {
    return contentRef.text;
  }

  return fs.readFileSync(resolveCaptureBlobPath(contentRef.blobPath), "utf8");
}
