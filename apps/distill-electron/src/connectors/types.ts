import { DiscoveredCapture, DiscoveredSource, ParsedCapture, SourceKind } from "../shared/types";

export type CaptureSnapshot = {
  rawText: string;
  rawSha256: string;
  sourceModifiedAt?: string;
  sourceSizeBytes?: number;
};

export type SourceConnector = {
  kind: SourceKind;
  detect: () => DiscoveredSource;
  discoverCaptures: () => DiscoveredCapture[];
  snapshotCapture: (capture: DiscoveredCapture) => CaptureSnapshot;
  parseCapture: (capture: DiscoveredCapture, snapshot: CaptureSnapshot) => ParsedCapture;
};
