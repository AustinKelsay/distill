/**
 * Production Tauri bridge. Uses only the explicit invoke/event APIs.
 *
 * v1 privacy/capability boundary (issue #32): the renderer must never gain
 * ambient filesystem, process, SQL, or shell access. Sensitive is an
 * export-only policy label; Distill provides no application encryption,
 * per-session delete, retention purge, or secure-forget in v1.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  ActivityListPage,
  ActivityListRequest,
  CurationMutationResult,
  DistillBridge,
  ExportDataset,
  ExportPreview,
  ExportProgress,
  ExportResult,
  FixtureJourneyInput,
  FixtureJourneyPhase,
  FixtureJourneyResult,
  HealthReport,
  LegacyImportReport,
  OperationsPage,
  OperationsRequest,
  RepairReport,
  SessionCurationRequest,
  SessionDetail,
  SessionDetailRequest,
  SessionListPage,
  SessionListRequest,
  SourcePreference,
  SyncProgress,
  SyncRunResult,
  SyncRunSummary,
} from "./types";

const PROGRESS_EVENT = "fixture-journey-progress";
const SYNC_PROGRESS_EVENT = "sync-progress";
const EXPORT_PROGRESS_EVENT = "export-progress";

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
    async importLegacy(home: string, sourceHome: string): Promise<LegacyImportReport> {
      return invoke<LegacyImportReport>("import_legacy_command", {
        home,
        sourceHome,
      });
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
    async listSessions(
      home: string,
      request: SessionListRequest,
    ): Promise<SessionListPage> {
      return invoke<SessionListPage>("sessions_list_command", { home, request });
    },
    async sessionDetail(
      home: string,
      request: SessionDetailRequest,
    ): Promise<SessionDetail | null> {
      return invoke<SessionDetail | null>("session_detail_command", { home, request });
    },
    async addSessionTag(
      home: string,
      request: SessionCurationRequest,
    ): Promise<CurationMutationResult> {
      return invoke<CurationMutationResult>("add_session_tag_command", { home, request });
    },
    async removeSessionTag(
      home: string,
      request: SessionCurationRequest,
    ): Promise<CurationMutationResult> {
      return invoke<CurationMutationResult>("remove_session_tag_command", {
        home,
        request,
      });
    },
    async toggleSessionLabel(
      home: string,
      request: SessionCurationRequest,
    ): Promise<CurationMutationResult> {
      return invoke<CurationMutationResult>("toggle_session_label_command", {
        home,
        request,
      });
    },
    async previewExport(home: string, dataset: ExportDataset): Promise<ExportPreview> {
      return invoke<ExportPreview>("export_preview_command", { home, dataset });
    },
    async publishExport(home: string, dataset: ExportDataset): Promise<ExportResult> {
      return invoke<ExportResult>("export_publish_command", { home, dataset });
    },
    async cancelExport(home: string, dataset: ExportDataset): Promise<boolean> {
      return invoke<boolean>("export_cancel_command", { home, dataset });
    },
    async listActivity(
      home: string,
      request: ActivityListRequest,
    ): Promise<ActivityListPage> {
      return invoke<ActivityListPage>("activity_list_command", { home, request });
    },
    async listOperations(
      home: string,
      request: OperationsRequest,
    ): Promise<OperationsPage> {
      return invoke<OperationsPage>("operations_list_command", { home, request });
    },
    onProgress(listener: (phase: FixtureJourneyPhase) => void) {
      return subscribe(PROGRESS_EVENT, listener);
    },
    onSyncProgress(listener: (progress: SyncProgress) => void) {
      return subscribe(SYNC_PROGRESS_EVENT, listener);
    },
    onExportProgress(listener: (progress: ExportProgress) => void) {
      return subscribe(EXPORT_PROGRESS_EVENT, listener);
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
