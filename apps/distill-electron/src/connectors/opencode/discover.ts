import { DiscoveredCapture } from "../../shared/types";
import { listOpenCodeSessions, openCodeTimestampToIso, OpenCodeDiscoveryError } from "./common";

export function discoverOpenCodeCaptures(): DiscoveredCapture[] {
  try {
    return listOpenCodeSessions().map((session) => ({
      sourceKind: "opencode",
      captureKind: "session_export",
      sourcePath: `opencode://session/${session.id}`,
      externalSessionId: session.id,
      sourceModifiedAt: openCodeTimestampToIso(session.time_updated),
      metadata: {
        title: session.title ?? null,
        directory: session.directory ?? null,
        version: session.version ?? null,
        timeCreated: session.time_created ?? null,
        timeUpdated: session.time_updated ?? null,
        timeArchived: session.time_archived ?? null,
        shareUrl: session.share_url ?? null
      }
    }));
  } catch (error) {
    if (error instanceof OpenCodeDiscoveryError) {
      throw new OpenCodeDiscoveryError(
        error.code,
        `OpenCode session discovery failed: ${error.message}`,
        {
          cause: error,
          stdout: error.stdout,
          stderr: error.stderr
        }
      );
    }

    throw error;
  }
}
