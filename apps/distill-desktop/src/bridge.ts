/**
 * Production Tauri bridge. Uses only the explicit invoke/event APIs.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  DistillBridge,
  FixtureJourneyInput,
  FixtureJourneyPhase,
  FixtureJourneyResult,
  HealthReport,
  RepairReport,
  SourcePreference,
  SyncProgress,
  SyncRunResult,
  SyncRunSummary,
} from "./types";

const PROGRESS_EVENT = "fixture-journey-progress";
const SYNC_PROGRESS_EVENT = "sync-progress";

/**
 * Create the real Tauri Distill bridge.
 */
export function createTauriBridge(): DistillBridge {
  return {
    async runFixtureJourney(input: FixtureJourneyInput): Promise<FixtureJourneyResult> {
      return invoke<FixtureJourneyResult>("run_fixture_journey_command", {
        home: input.home,
        fixtureRoot: input.fixtureRoot,
      });
    },
    async health(home: string): Promise<HealthReport> {
      return invoke<HealthReport>("health_command", { home });
    },
    async repair(home: string, confirm: boolean): Promise<RepairReport> {
      return invoke<RepairReport>("repair_command", { home, confirm });
    },
    async listSources(home: string): Promise<SourcePreference[]> {
      return invoke<SourcePreference[]>("list_sources_command", { home });
    },
    async setSourcePreference(
      home: string,
      kind: string,
      enabled: boolean,
      configuredRoot?: string | null,
    ): Promise<SourcePreference> {
      return invoke<SourcePreference>("set_source_preference_command", {
        home,
        kind,
        enabled,
        configuredRoot: configuredRoot ?? null,
      });
    },
    async startSync(home: string, sourceKinds?: string[]): Promise<SyncRunResult> {
      return invoke<SyncRunResult>("sync_start_command", {
        home,
        sourceKinds: sourceKinds ?? [],
      });
    },
    async syncStatus(home: string, syncRunId?: number | null): Promise<SyncRunSummary> {
      return invoke<SyncRunSummary>("sync_status_command", {
        home,
        syncRunId: syncRunId ?? null,
      });
    },
    async cancelSync(home: string, syncRunId: number): Promise<SyncRunSummary> {
      return invoke<SyncRunSummary>("sync_cancel_command", { home, syncRunId });
    },
    onProgress(listener: (phase: FixtureJourneyPhase) => void) {
      return subscribe(PROGRESS_EVENT, listener);
    },
    onSyncProgress(listener: (progress: SyncProgress) => void) {
      return subscribe(SYNC_PROGRESS_EVENT, listener);
    },
  };
}

/**
 * Subscribe to a typed Tauri event with race-safe cleanup.
 */
function subscribe<T>(eventName: string, listener: (payload: T) => void): () => void {
  let unlisten: (() => void) | undefined;
  let disposed = false;
  void listen<T>(eventName, (event) => {
    listener(event.payload);
  }).then((fn) => {
    if (disposed) {
      fn();
    } else {
      unlisten = fn;
    }
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}
