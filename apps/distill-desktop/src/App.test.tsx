/**
 * Renderer seam: React first-run UI against one typed Distill bridge fake.
 */

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import type {
  ActivityListPage,
  CurationMutationResult,
  DistillBridge,
  ExportPreview,
  ExportProgress,
  ExportResult,
  FixtureJourneyPhase,
  FixtureJourneyResult,
  HealthReport,
  HostError,
  LegacyImportReport,
  OperationsPage,
  RepairReport,
  SessionDetail,
  SessionListItem,
  SessionListPage,
  SourceDetectResult,
  SyncRunResult,
  SyncRunSummary,
} from "./types";

/**
 * Build a successful Fixture journey fixture for the typed bridge fake.
 */
function sampleResult(): FixtureJourneyResult {
  return {
    source: {
      kind: "fixture",
      display_name: "Fixture",
      data_root: "/tmp/fixture",
      parser_id: "fixture",
      parser_version: "1.0.0",
    },
    sync: {
      accepted_captures: 1,
      skipped_duplicates: 0,
      successful_attempts: 1,
      failed_attempts: 0,
      capture_ids: [1],
      session_identities: [
        {
          source_kind: "fixture",
          external_session_id: "fixture-session-ui",
        },
      ],
    },
    session: {
      summary: {
        id: 1,
        source_kind: "fixture",
        external_session_id: "fixture-session-ui",
        title: "UI Fixture",
        accepted_capture_count: 1,
        normalization_attempt_count: 1,
        successful_projection_generation: 1,
      },
      messages: [
        {
          id: 1,
          ordinal: 0,
          role: "user",
          message_kind: "text",
          text: "Hello from UI fixture",
        },
      ],
      artifacts: [],
      metadata_json: "{}",
    },
    health: {
      ok: true,
      schema_status: "ok",
      content_status: "ok",
      fts_status: "ok",
      staging_status: "ok",
      orphan_status: "ok",
      incomplete_status: "ok",
      operations_status: "ok",
      issues: [],
      open_reconciliation: { removed_staging_partials: 0 },
    },
  };
}

/**
 * Create one typed bridge fake with controllable success/error behavior.
 */
function createFakeBridge(options?: {
  result?: FixtureJourneyResult;
  error?: HostError;
  phases?: FixtureJourneyPhase[];
  health?: HealthReport;
  repair?: RepairReport;
  pendingSync?: Promise<SyncRunResult>;
  syncResult?: SyncRunResult;
  cancelResult?: SyncRunSummary;
  onCancel?: (syncRunId: number) => void;
  sessionPage?: SessionListPage;
  sessionPages?: Record<string, SessionListPage>;
  sessionDetail?: SessionDetail | null;
  sessionDetails?: Record<string, SessionDetail | null>;
  pendingSessions?: Promise<SessionListPage>;
  onListSessions?: () => void;
  onSessionDetail?: () => void;
  curationResult?: CurationMutationResult;
  curationError?: HostError;
  exportPreview?: ExportPreview;
  exportResult?: ExportResult;
  exportError?: HostError;
  pendingExport?: Promise<ExportResult>;
  exportPhases?: ExportProgress[];
  exportCancelRequested?: boolean;
  activityPage?: ActivityListPage;
  activityPages?: Record<string, ActivityListPage>;
  activityError?: HostError;
  pendingActivity?: Promise<ActivityListPage>;
  operationsPage?: OperationsPage;
  operationsError?: HostError;
  pendingOperations?: Promise<OperationsPage>;
  migrationReport?: LegacyImportReport;
  migrationError?: HostError;
  pendingMigration?: Promise<LegacyImportReport>;
  attempts?: import("./types").AttemptSummary[];
  attemptError?: HostError;
  pendingAttempts?: Promise<import("./types").AttemptSummary[]>;
  renormalizeReport?: import("./types").RenormalizeReport;
  renormalizeError?: HostError;
  pendingRenormalize?: Promise<import("./types").RenormalizeReport>;
  detectResults?: SourceDetectResult[];
  detectError?: HostError;
  pendingDetect?: Promise<SourceDetectResult[]>;
}): DistillBridge {
  const listeners = new Set<(phase: FixtureJourneyPhase) => void>();
  const syncListeners = new Set<(progress: import("./types").SyncProgress) => void>();
  const exportListeners = new Set<(progress: ExportProgress) => void>();
  return {
    async runFixtureJourney() {
      for (const phase of options?.phases ?? [
        "detecting_source",
        "syncing_captures",
        "loading_session",
        "checking_health",
      ]) {
        listeners.forEach((listener) => listener(phase));
      }
      if (options?.error) throw options.error;
      return options?.result ?? sampleResult();
    },
    async health() {
      if (options?.error) throw options.error;
      return options?.health ?? sampleResult().health;
    },
    async importLegacy() {
      if (options?.pendingMigration) return options.pendingMigration;
      if (options?.migrationError) throw options.migrationError;
      return (
        options?.migrationReport ?? {
          ok: true,
          reused_prior_import: false,
          source_fingerprint: "abc",
          source_db_sha256: "def",
          content_fingerprint: "ghi",
          counts: {
            sources: 1,
            captures: 1,
            captures_skipped: 0,
            attempts: 1,
            facts: 1,
            sessions: 1,
            messages: 1,
            artifacts: 0,
            tags: 1,
            tag_assignments: 1,
            labels: 1,
            label_assignments: 1,
            activity_events: 1,
            exports: 0,
            exports_skipped: 0,
          },
          skips: [],
        }
      );
    },
    async repair(_home, confirm) {
      if (!confirm) {
        throw { code: "validation", message: "repair requires explicit confirmation" };
      }
      if (options?.error) throw options.error;
      return (
        options?.repair ?? {
          actions: [{ name: "removed_staging_partials", count: 0 }],
          health_after: sampleResult().health,
        }
      );
    },
    async listSources() {
      return [
        {
          kind: "fixture",
          enabled: true,
          configured_root: "/tmp/fixture",
          display_name: "Fixture",
          data_root: null,
        },
      ];
    },
    async detectSources() {
      if (options?.pendingDetect) return options.pendingDetect;
      if (options?.detectError) throw options.detectError;
      if (options?.detectResults) return options.detectResults;
      return [
        {
          kind: "fixture",
          status: "ok",
          executable: null,
          effective_data_root: "/tmp/fixture",
          display_name: "Fixture",
          error_class: null,
          error_message: null,
        },
      ];
    },
    async setSourcePreference(_home, kind, enabled, configuredRoot) {
      return {
        kind,
        enabled,
        configured_root: configuredRoot ?? null,
        display_name: kind,
        data_root: null,
      };
    },
    async startSync() {
      syncListeners.forEach((listener) =>
        listener({ type: "run_started", sync_run_id: 1 }),
      );
      if (options?.pendingSync) return options.pendingSync;
      return (
        options?.syncResult ?? {
          run: {
            id: 1,
            status: "completed",
            cancel_requested: false,
            accepted_captures: 1,
            skipped_duplicates: 0,
            successful_attempts: 1,
            failed_attempts: 0,
            error_class: null,
            error_message: null,
            sources: [
              {
                source_kind: "fixture",
                status: "completed",
                accepted_captures: 1,
                skipped_duplicates: 0,
                successful_attempts: 1,
                failed_attempts: 0,
                error_class: null,
                error_message: null,
              },
            ],
          },
          session_identities: [
            { source_kind: "fixture", external_session_id: "fixture-session-ui" },
          ],
        }
      );
    },
    async syncStatus() {
      return {
        id: 1,
        status: "completed",
        cancel_requested: false,
        accepted_captures: 1,
        skipped_duplicates: 0,
        successful_attempts: 1,
        failed_attempts: 0,
        error_class: null,
        error_message: null,
        sources: [],
      };
    },
    async cancelSync(_home, syncRunId) {
      options?.onCancel?.(syncRunId);
      return (
        options?.cancelResult ?? {
          id: 1,
          status: "cancelled",
          cancel_requested: true,
          accepted_captures: 0,
          skipped_duplicates: 0,
          successful_attempts: 0,
          failed_attempts: 0,
          error_class: "cancelled",
          error_message: "sync run cancelled at a safe checkpoint",
          sources: [],
        }
      );
    },
    async listSessions(_home, request) {
      options?.onListSessions?.();
      if (options?.error) throw options.error;
      if (options?.pendingSessions) return options.pendingSessions;
      if (options?.sessionPages && request.cursor) {
        return options.sessionPages[request.cursor] ?? { items: [], next_cursor: null };
      }
      return options?.sessionPage ?? { items: [], next_cursor: null };
    },
    async sessionDetail(_home, request) {
      options?.onSessionDetail?.();
      if (options?.error) throw options.error;
      if (options?.sessionDetails) {
        const cursor = request.message_cursor ?? request.artifact_cursor;
        if (cursor) return options.sessionDetails[cursor] ?? null;
      }
      return options?.sessionDetail ?? null;
    },
    async addSessionTag(_home, request) {
      if (options?.curationError) throw options.curationError;
      return (
        options?.curationResult ?? {
          changed: true,
          identity: {
            source_kind: request.source_kind,
            external_session_id: request.external_session_id,
          },
          tags: [
            {
              id: 1,
              name: request.name.trim().toLowerCase(),
              kind: "manual",
              origin: "manual",
            },
          ],
          labels: [],
          workflow_state: "neutral",
        }
      );
    },
    async removeSessionTag(_home, request) {
      if (options?.curationError) throw options.curationError;
      return (
        options?.curationResult ?? {
          changed: true,
          identity: {
            source_kind: request.source_kind,
            external_session_id: request.external_session_id,
          },
          tags: [],
          labels: [],
          workflow_state: "neutral",
        }
      );
    },
    async toggleSessionLabel(_home, request) {
      if (options?.curationError) throw options.curationError;
      return (
        options?.curationResult ?? {
          changed: true,
          identity: {
            source_kind: request.source_kind,
            external_session_id: request.external_session_id,
          },
          tags: [],
          labels: [
            {
              id: 1,
              name: request.name.trim().toLowerCase(),
              scope: "session",
              origin: "manual",
            },
          ],
          workflow_state:
            request.name.trim().toLowerCase() === "train" ? "train_ready" : "favorite",
        }
      );
    },
    async previewExport(_home, dataset) {
      if (options?.exportError) throw options.exportError;
      return (
        options?.exportPreview ?? {
          dataset,
          format_id: "distill-session-jsonl-v1",
          eligible: [
            { source_kind: "fixture", external_session_id: "fixture-session-ui" },
          ],
          omitted: [],
        }
      );
    },
    async publishExport(_home, dataset) {
      for (const progress of options?.exportPhases ?? [
        { type: "preparing" as const, export_id: 1 },
        { type: "published" as const, export_id: 1 },
      ]) {
        exportListeners.forEach((listener) => listener(progress));
      }
      if (options?.pendingExport) return options.pendingExport;
      if (options?.exportError) throw options.exportError;
      return (
        options?.exportResult ?? {
          export_id: 1,
          dataset,
          format_id: "distill-session-jsonl-v1",
          status: "published",
          output_path: "/tmp/home/exports/train-sessions.jsonl",
          sha256: "abc",
          byte_size: 12,
          record_count: 1,
          eligible: [
            { source_kind: "fixture", external_session_id: "fixture-session-ui" },
          ],
          omitted: [],
          error_class: null,
          error_message: null,
        }
      );
    },
    async cancelExport() {
      return options?.exportCancelRequested ?? true;
    },
    async listActivity(_home, request) {
      if (options?.pendingActivity) return options.pendingActivity;
      if (options?.activityError) throw options.activityError;
      if (options?.activityPages && request.cursor) {
        return options.activityPages[request.cursor] ?? { items: [], next_cursor: null };
      }
      return (
        options?.activityPage ?? {
          items: [
            {
              id: 2,
              event_type: "projection_replaced",
              occurred_at: "2026-01-01T00:00:01Z",
              source_kind: "fixture",
              session_id: 1,
              capture_id: 1,
              attempt_id: 1,
              payload_json: { projection_generation: 1 },
            },
            {
              id: 1,
              event_type: "capture_recorded",
              occurred_at: "2026-01-01T00:00:00Z",
              source_kind: "fixture",
              session_id: 1,
              capture_id: 1,
              attempt_id: null,
              payload_json: {},
            },
          ],
          next_cursor: null,
        }
      );
    },
    async listOperations() {
      if (options?.pendingOperations) return options.pendingOperations;
      if (options?.operationsError) throw options.operationsError;
      return (
        options?.operationsPage ?? {
          operations_status: "ok",
          sync_runs: [
            {
              id: 1,
              status: "completed",
              cancel_requested: false,
              accepted_captures: 1,
              skipped_duplicates: 0,
              successful_attempts: 1,
              failed_attempts: 0,
              error_class: null,
              error_message: null,
              warning_details: [],
              sources: [],
            },
          ],
          next_sync_cursor: null,
          exports: [
            {
              id: 1,
              dataset: "train",
              format_id: "distill-session-jsonl-v1",
              status: "published",
              created_at: "2026-01-01T00:00:00Z",
              updated_at: "2026-01-01T00:00:00Z",
              sha256: "abc",
              byte_size: 12,
              record_count: 1,
              error_class: null,
              error_message: null,
            },
          ],
          next_export_cursor: null,
        }
      );
    },
    async captureAttempts() {
      if (options?.pendingAttempts) return options.pendingAttempts;
      if (options?.attemptError) throw options.attemptError;
      return (
        options?.attempts ?? [
          {
            id: 1,
            capture_id: 1,
            parser_id: "fixture",
            parser_version: "1.0.0",
            outcome: "succeeded",
            error_class: null,
            error_message: null,
            projection_generation: 1,
            fact_count: 2,
          },
        ]
      );
    },
    async renormalizeCapture() {
      if (options?.pendingRenormalize) return options.pendingRenormalize;
      if (options?.renormalizeError) throw options.renormalizeError;
      return (
        options?.renormalizeReport ?? {
          capture_id: 1,
          attempt_id: 2,
          outcome: "succeeded",
          parser_id: "fixture",
          parser_version: "1.0.0",
        }
      );
    },
    onProgress(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    onSyncProgress(listener) {
      syncListeners.add(listener);
      return () => {
        syncListeners.delete(listener);
      };
    },
    onExportProgress(listener) {
      exportListeners.add(listener);
      return () => {
        exportListeners.delete(listener);
      };
    },
  };
}

describe("first-run Fixture UI", () => {
  it("renders idle input and success source/sync/session/health panels", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge();
    render(<App bridge={bridge} />);

    expect(screen.getByTestId("status")).toHaveTextContent("idle");
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.type(
      screen.getByRole("textbox", { name: "Fixture root" }),
      "/tmp/fixture",
    );
    await user.click(screen.getByRole("button", { name: /run fixture journey/i }));

    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("success");
    });
    expect(screen.getByTestId("source-panel")).toHaveTextContent("fixture");
    expect(screen.getByTestId("sync-panel")).toHaveTextContent("Accepted captures");
    expect(screen.getByTestId("sync-panel")).toHaveTextContent("1");
    expect(screen.getByTestId("session-panel")).toHaveTextContent(
      "fixture:fixture-session-ui",
    );
    expect(screen.getByTestId("session-panel")).toHaveTextContent(
      "Accepted Capture count",
    );
    expect(screen.getByTestId("session-panel")).toHaveTextContent(
      "Normalization Attempt count",
    );
    expect(screen.getByTestId("session-panel")).toHaveTextContent(
      "Successful projection generation",
    );
    expect(screen.getByTestId("session-panel")).toHaveTextContent(
      "Hello from UI fixture",
    );
    expect(screen.getByTestId("health-panel")).toHaveTextContent("true");
  });

  it("detects Sources through the bridge and renders per-Source status", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge();
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByTestId("detect-sources"));

    await waitFor(() => {
      expect(screen.getByTestId("detect-status")).toHaveTextContent("ready");
    });
    expect(screen.getByTestId("detect-sources")).toHaveAccessibleName(
      "Detect Sources — Status: ready",
    );
    expect(screen.getByTestId("detect-result-fixture")).toHaveTextContent("fixture: ok");
    expect(screen.getByTestId("source-detection-panel")).toHaveAttribute(
      "aria-busy",
      "false",
    );
  });

  it("keeps detection visibly loading until the bridge resolves", async () => {
    const user = userEvent.setup();
    let resolveDetect!: (results: SourceDetectResult[]) => void;
    const pendingDetect = new Promise<SourceDetectResult[]>((resolve) => {
      resolveDetect = resolve;
    });
    render(<App bridge={createFakeBridge({ pendingDetect })} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByTestId("detect-sources"));
    expect(screen.getByTestId("detect-status")).toHaveTextContent("loading");
    expect(screen.getByTestId("source-detection-panel")).toHaveAttribute(
      "aria-busy",
      "true",
    );

    resolveDetect([
      {
        kind: "fixture",
        status: "ok",
        executable: null,
        effective_data_root: "/tmp/fixture",
        display_name: "Fixture",
        error_class: null,
        error_message: null,
      },
    ]);
    await waitFor(() => {
      expect(screen.getByTestId("detect-status")).toHaveTextContent("ready");
    });
  });

  it("renders empty, warning, and error detection states", async () => {
    const user = userEvent.setup();
    const fillHome = async () => {
      await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
      await user.click(screen.getByTestId("detect-sources"));
    };

    const { unmount: unmountEmpty } = render(
      <App bridge={createFakeBridge({ detectResults: [] })} />,
    );
    await fillHome();
    await waitFor(() => {
      expect(screen.getByTestId("detect-status")).toHaveTextContent("empty");
    });
    unmountEmpty();

    const { unmount: unmountWarning } = render(
      <App
        bridge={createFakeBridge({
          detectResults: [
            {
              kind: "fixture",
              status: "ok",
              executable: null,
              effective_data_root: "/tmp/fixture",
              display_name: "Fixture",
              error_class: null,
              error_message: null,
            },
            {
              kind: "codex",
              status: "unavailable",
              executable: "/private/provider/codex",
              effective_data_root: "/private/provider/root",
              display_name: "Codex",
              error_class: "executable_not_found",
              error_message: "source executable is unavailable",
            },
          ],
        })}
      />,
    );
    await fillHome();
    await waitFor(() => {
      expect(screen.getByTestId("detect-status")).toHaveTextContent("warning");
    });
    expect(screen.getByTestId("detect-sources")).toHaveAccessibleName(
      "Detect Sources — Status: warning",
    );
    expect(screen.getByTestId("detect-results")).toHaveTextContent(
      "codex: unavailable (executable_not_found)",
    );
    expect(screen.getByTestId("detect-results")).not.toHaveTextContent(
      "/private/provider",
    );
    unmountWarning();

    render(
      <App
        bridge={createFakeBridge({
          detectError: { code: "runtime", message: "source detection failed" },
        })}
      />,
    );
    await fillHome();
    await waitFor(() => {
      expect(screen.getByTestId("detect-status")).toHaveTextContent("error");
    });
    expect(screen.getByTestId("detect-error")).toHaveTextContent(
      "runtime: source detection failed",
    );
  });

  it("loads paged sessions, detail slices, and preserves selection on refresh", async () => {
    const user = userEvent.setup();
    const sessionPage: SessionListPage = {
      items: [
        {
          id: 8,
          source_kind: "fixture",
          external_session_id: "session-explorer",
          title: "Explorer session",
          project_path: "/workspace/demo",
          updated_at: "2026-01-01T00:00:00Z",
          preview: "Preview",
          message_count: 2,
          accepted_capture_count: 1,
          normalization_attempt_count: 1,
          successful_projection_generation: 1,
          labels: [],
          tags: [],
          workflow_state: "neutral",
        },
      ],
      next_cursor: null,
    };
    const bridge = createFakeBridge({
      sessionPage,
      sessionDetail: {
        ...sampleResult().session!,
        project_path: "/workspace/demo",
        projection_summary: "Summary",
        raw_capture_count: 1,
        labels: [],
        tags: [],
        workflow_state: "neutral",
      },
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await waitFor(() => {
      expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("ready");
    });
    const sessionButton = screen.getByRole("button", { name: /Explorer session/ });
    await user.click(sessionButton);
    await waitFor(() => {
      expect(screen.getByTestId("session-detail-panel")).toHaveTextContent("Summary");
    });
    expect(sessionButton).toHaveAttribute("aria-pressed", "true");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /Explorer session/ })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
    });
  });

  it("appends session and transcript pages without losing selection or context", async () => {
    const user = userEvent.setup();
    const first: SessionListItem = {
      id: 11,
      source_kind: "fixture",
      external_session_id: "first",
      title: "First session",
      project_path: null,
      updated_at: null,
      preview: "first",
      message_count: 2,
      accepted_capture_count: 1,
      normalization_attempt_count: 1,
      successful_projection_generation: 1,
      labels: [],
      tags: [],
      workflow_state: "neutral",
    };
    const second: SessionListItem = {
      ...first,
      id: 12,
      external_session_id: "second",
      title: "Second session",
    };
    const initialDetail: SessionDetail = {
      ...sampleResult().session!,
      summary: {
        ...sampleResult().session!.summary,
        external_session_id: "first",
        title: "First session",
      },
      messages: [
        { id: 21, ordinal: 0, role: "user", message_kind: "text", text: "first message" },
      ],
      next_message_cursor: "message-1",
    };
    const continuationDetail: SessionDetail = {
      ...initialDetail,
      messages: [
        {
          id: 22,
          ordinal: 1,
          role: "assistant",
          message_kind: "text",
          text: "second message",
        },
      ],
      next_message_cursor: null,
    };
    const bridge = createFakeBridge({
      sessionPage: { items: [first], next_cursor: "page-1" },
      sessionPages: { "page-1": { items: [second], next_cursor: null } },
      sessionDetail: initialDetail,
      sessionDetails: { "message-1": continuationDetail },
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /First session/ })).toBeTruthy(),
    );
    expect(screen.queryByTestId("session-detail-panel")).toBeNull();
    await user.click(screen.getByRole("button", { name: /First session/ }));
    await waitFor(() =>
      expect(screen.getByTestId("session-detail-panel")).toHaveTextContent(
        "first message",
      ),
    );

    await user.click(screen.getByRole("button", { name: "Load more sessions" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Second session/ })).toBeTruthy(),
    );
    expect(screen.getByRole("button", { name: /First session/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByTestId("session-detail-panel")).toHaveTextContent("first message");

    await user.click(screen.getByRole("button", { name: "Load more transcript" }));
    await waitFor(() =>
      expect(screen.getByTestId("session-detail-panel")).toHaveTextContent(
        "second message",
      ),
    );
    expect(screen.getByTestId("session-detail-panel")).toHaveTextContent("first message");
  });

  it("shows explicit empty, error, cancelled, and warning session states", async () => {
    const user = userEvent.setup();
    const empty = createFakeBridge();
    const { unmount } = render(<App bridge={empty} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await waitFor(() => {
      expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("empty");
    });
    unmount();

    const failing = createFakeBridge({
      error: { code: "query", message: "query failed" },
    });
    const failingRender = render(<App bridge={failing} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await waitFor(() => {
      expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("error");
    });
    expect(screen.getByTestId("session-explorer-error")).toHaveTextContent(
      "query failed",
    );
    failingRender.unmount();

    let resolveSessions!: (page: SessionListPage) => void;
    const pendingSessions = new Promise<SessionListPage>((resolve) => {
      resolveSessions = resolve;
    });
    const pending = createFakeBridge({ pendingSessions });
    const pendingRender = render(<App bridge={pending} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("loading");
    await user.click(screen.getByRole("button", { name: "Cancel session load" }));
    expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("cancelled");
    resolveSessions({ items: [], next_cursor: null });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("cancelled");
    pendingRender.unmount();

    const warning = createFakeBridge({
      syncResult: {
        run: {
          id: 2,
          status: "warning",
          cancel_requested: false,
          accepted_captures: 1,
          skipped_duplicates: 0,
          successful_attempts: 1,
          failed_attempts: 1,
          error_class: null,
          error_message: null,
          warning_details: ["partial"],
          sources: [],
        },
        session_identities: [],
      },
    });
    render(<App bridge={warning} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Start Sync Run" }));
    await waitFor(() => {
      expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("warning");
    });
  });

  it("renders typed error state from the bridge", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge({
      error: { code: "validation", message: "fixture root must not be empty" },
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.type(
      screen.getByRole("textbox", { name: "Fixture root" }),
      "/tmp/fixture",
    );
    await user.click(screen.getByRole("button", { name: /run fixture journey/i }));

    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("error");
    });
    expect(screen.getByTestId("error-panel")).toHaveTextContent("validation");
    expect(screen.getByTestId("error-panel")).toHaveTextContent(
      "fixture root must not be empty",
    );
  });

  it("shows running status while the bridge journey is in flight", async () => {
    const user = userEvent.setup();
    let resolveJourney!: (value: FixtureJourneyResult) => void;
    const pending = new Promise<FixtureJourneyResult>((resolve) => {
      resolveJourney = resolve;
    });
    const bridge: DistillBridge = {
      runFixtureJourney: () => pending,
      health: async () => sampleResult().health,
      importLegacy: async () => {
        throw new Error("not used");
      },
      repair: async () => ({
        actions: [],
        health_after: sampleResult().health,
      }),
      listSources: async () => [],
      detectSources: async () => [],
      setSourcePreference: async (_home, kind, enabled, configuredRoot) => ({
        kind,
        enabled,
        configured_root: configuredRoot ?? null,
        display_name: kind,
        data_root: null,
      }),
      startSync: async () => {
        throw new Error("not used");
      },
      syncStatus: async () => {
        throw new Error("not used");
      },
      cancelSync: async () => {
        throw new Error("not used");
      },
      listSessions: async () => ({ items: [], next_cursor: null }),
      sessionDetail: async () => null,
      addSessionTag: async () => {
        throw new Error("not used");
      },
      removeSessionTag: async () => {
        throw new Error("not used");
      },
      toggleSessionLabel: async () => {
        throw new Error("not used");
      },
      previewExport: async () => {
        throw new Error("not used");
      },
      publishExport: async () => {
        throw new Error("not used");
      },
      cancelExport: async () => {
        throw new Error("not used");
      },
      listActivity: async () => ({ items: [], next_cursor: null }),
      listOperations: async () => ({
        operations_status: "ok",
        sync_runs: [],
        next_sync_cursor: null,
        exports: [],
        next_export_cursor: null,
      }),
      captureAttempts: async () => [],
      renormalizeCapture: async () => {
        throw new Error("not used");
      },
      onProgress: () => () => undefined,
      onSyncProgress: () => () => undefined,
      onExportProgress: () => () => undefined,
    };
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.type(
      screen.getByRole("textbox", { name: "Fixture root" }),
      "/tmp/fixture",
    );
    await user.click(screen.getByRole("button", { name: /run fixture journey/i }));

    expect(screen.getByTestId("status")).toHaveTextContent("running");
    resolveJourney(sampleResult());
    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("success");
    });
  });

  it("requires confirmation before repair and renders typed repair actions", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge({
      repair: {
        actions: [{ name: "removed_orphan_blobs", count: 1 }],
        health_after: {
          ...sampleResult().health,
          ok: true,
        },
      },
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    expect(screen.queryByTestId("repair-confirm-dialog")).not.toBeNull();
    expect(screen.getByTestId("repair-confirm-dialog")).not.toHaveAttribute("open");
    await user.click(screen.getByRole("button", { name: /repair library/i }));
    expect(
      screen.getByRole("dialog", { name: /confirm destructive repair/i }),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: /confirm repair/i }));

    await waitFor(() => {
      expect(screen.getByTestId("repair-panel")).toHaveTextContent(
        "removed_orphan_blobs",
      );
    });
    expect(screen.getByTestId("repair-panel")).toHaveTextContent("1");
  });

  it("starts a Sync Run through the bridge and renders completed status", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge();
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.type(
      screen.getByRole("textbox", { name: "Fixture root" }),
      "/tmp/fixture",
    );
    await user.click(screen.getByRole("button", { name: /start sync run/i }));

    await waitFor(() => {
      expect(screen.getByTestId("sync-run-panel")).toHaveTextContent("completed");
    });
    expect(screen.getByTestId("status")).toHaveTextContent("success");
    expect(screen.getByTestId("sources-list")).toHaveTextContent("fixture: enabled");
  });

  it("renders warning details and per-source outcomes", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge({
      syncResult: {
        run: {
          id: 2,
          status: "warning",
          cancel_requested: false,
          accepted_captures: 1,
          skipped_duplicates: 0,
          successful_attempts: 1,
          failed_attempts: 0,
          error_class: null,
          error_message: null,
          warning_details: ["codex: source adapter is not registered in this build"],
          sources: [
            {
              source_kind: "fixture",
              status: "completed",
              accepted_captures: 1,
              skipped_duplicates: 0,
              successful_attempts: 1,
              failed_attempts: 0,
              error_class: null,
              error_message: null,
            },
            {
              source_kind: "codex",
              status: "failed",
              accepted_captures: 0,
              skipped_duplicates: 0,
              successful_attempts: 0,
              failed_attempts: 0,
              error_class: "adapter_not_registered",
              error_message: "source adapter is not registered in this build",
            },
          ],
        },
        session_identities: [],
      },
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.type(
      screen.getByRole("textbox", { name: "Fixture root" }),
      "/tmp/fixture",
    );
    await user.click(screen.getByRole("button", { name: /start sync run/i }));

    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("warning");
    });
    expect(screen.getByTestId("sync-warning-details")).toHaveTextContent("codex:");
    expect(screen.getByTestId("sync-run-panel")).toHaveTextContent("fixture: completed");
    expect(screen.getByTestId("sync-run-panel")).toHaveTextContent("codex: failed");
  });

  it("keeps cancellation tied to the active run while cancellation is pending", async () => {
    const user = userEvent.setup();
    let resolveSync!: (value: SyncRunResult) => void;
    const pendingSync = new Promise<SyncRunResult>((resolve) => {
      resolveSync = resolve;
    });
    let cancelledId: number | null = null;
    const bridge = createFakeBridge({
      pendingSync,
      onCancel: (syncRunId) => {
        cancelledId = syncRunId;
      },
      cancelResult: {
        id: 1,
        status: "running",
        cancel_requested: true,
        accepted_captures: 0,
        skipped_duplicates: 0,
        successful_attempts: 0,
        failed_attempts: 0,
        error_class: null,
        error_message: null,
        sources: [],
      },
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.type(
      screen.getByRole("textbox", { name: "Fixture root" }),
      "/tmp/fixture",
    );
    await user.click(screen.getByRole("button", { name: /start sync run/i }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /cancel sync/i })).not.toBeDisabled();
    });

    await user.click(screen.getByRole("button", { name: /cancel sync/i }));
    expect(cancelledId).toBe(1);
    expect(screen.getByTestId("status")).toHaveTextContent("running");

    resolveSync({
      run: {
        id: 1,
        status: "completed",
        cancel_requested: true,
        accepted_captures: 1,
        skipped_duplicates: 0,
        successful_attempts: 1,
        failed_attempts: 0,
        error_class: null,
        error_message: null,
        sources: [],
      },
      session_identities: [],
    });
    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("success");
    });
  });

  it("applies curation snapshots immediately and shows manual origins", async () => {
    const user = userEvent.setup();
    let listCalls = 0;
    let detailCalls = 0;
    const sessionPage: SessionListPage = {
      items: [
        {
          id: 8,
          source_kind: "fixture",
          external_session_id: "session-explorer",
          title: "Explorer session",
          project_path: "/workspace/demo",
          updated_at: "2026-01-01T00:00:00Z",
          preview: "Preview",
          message_count: 2,
          accepted_capture_count: 1,
          normalization_attempt_count: 1,
          successful_projection_generation: 1,
          labels: [],
          tags: [],
          workflow_state: "neutral",
        },
      ],
      next_cursor: null,
    };
    const bridge = createFakeBridge({
      sessionPage,
      sessionDetail: {
        ...sampleResult().session!,
        summary: {
          ...sampleResult().session!.summary,
          external_session_id: "session-explorer",
          title: "Explorer session",
        },
        labels: [],
        tags: [],
        workflow_state: "neutral",
      },
      curationResult: {
        changed: true,
        identity: {
          source_kind: "fixture",
          external_session_id: "session-explorer",
        },
        tags: [{ id: 3, name: "research", kind: "manual", origin: "manual" }],
        labels: [{ id: 2, name: "train", scope: "session", origin: "manual" }],
        workflow_state: "train_ready",
      },
      onListSessions: () => {
        listCalls += 1;
      },
      onSessionDetail: () => {
        detailCalls += 1;
      },
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await user.click(screen.getByRole("button", { name: /Explorer session/ }));
    await waitFor(() => {
      expect(screen.getByTestId("session-detail-panel")).toBeTruthy();
    });

    await user.type(screen.getByLabelText("Add tag"), "research");
    await user.click(screen.getByRole("button", { name: "Add tag" }));
    await waitFor(() => {
      expect(screen.getByTestId("session-tags")).toHaveTextContent("research (manual)");
    });
    expect(screen.getByTestId("session-labels")).toHaveTextContent("train (manual)");
    expect(screen.getByRole("button", { name: /Explorer session/ })).toHaveTextContent(
      "train_ready",
    );
    const listCallsAfterLoad = listCalls;
    const detailCallsAfterLoad = detailCalls;
    await user.click(screen.getByRole("button", { name: "train" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "train" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
    });
    expect(listCalls).toBe(listCallsAfterLoad);
    expect(detailCalls).toBe(detailCallsAfterLoad);
  });

  it("surfaces a typed curation mutation error without clearing the detail", async () => {
    const user = userEvent.setup();
    const sessionPage: SessionListPage = {
      items: [
        {
          id: 8,
          source_kind: "fixture",
          external_session_id: "session-explorer",
          title: "Explorer session",
          project_path: null,
          updated_at: null,
          preview: null,
          message_count: 1,
          accepted_capture_count: 1,
          normalization_attempt_count: 1,
          successful_projection_generation: 1,
          labels: [],
          tags: [],
          workflow_state: "neutral",
        },
      ],
      next_cursor: null,
    };
    const bridge = createFakeBridge({
      sessionPage,
      sessionDetail: {
        ...sampleResult().session!,
        summary: {
          ...sampleResult().session!.summary,
          external_session_id: "session-explorer",
          title: "Explorer session",
        },
        labels: [],
        tags: [],
        workflow_state: "neutral",
      },
      curationError: { code: "curation", message: "mutation failed" },
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await user.click(screen.getByRole("button", { name: /Explorer session/ }));
    await waitFor(() => {
      expect(screen.getByTestId("session-detail-panel")).toBeTruthy();
    });

    await user.click(screen.getByRole("button", { name: "train" }));
    await waitFor(() => {
      expect(screen.getByTestId("session-curation-error")).toHaveTextContent(
        "curation: mutation failed",
      );
    });
    expect(screen.getByTestId("session-detail-panel")).toHaveTextContent(
      "Explorer session",
    );
  });

  it("previews then publishes export with explicit lifecycle states", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge({
      exportPreview: {
        dataset: "train",
        format_id: "distill-session-jsonl-v1",
        eligible: [{ source_kind: "fixture", external_session_id: "fixture-session-ui" }],
        omitted: [],
      },
      exportResult: {
        export_id: 9,
        dataset: "train",
        format_id: "distill-session-jsonl-v1",
        status: "published",
        output_path: "/tmp/home/exports/train.jsonl",
        sha256: "deadbeef",
        byte_size: 42,
        record_count: 1,
        eligible: [{ source_kind: "fixture", external_session_id: "fixture-session-ui" }],
        omitted: [],
        error_class: null,
        error_message: null,
      },
    });
    render(<App bridge={bridge} />);

    expect(screen.getByTestId("export-status")).toHaveTextContent("idle");
    expect(screen.getByRole("button", { name: "Publish export" })).toBeDisabled();

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Preview export" }));

    await waitFor(() => {
      expect(screen.getByTestId("export-status")).toHaveTextContent("success");
    });
    expect(screen.getByTestId("export-preview")).toHaveTextContent("train");
    expect(screen.getByTestId("export-eligible-count")).toHaveTextContent("1");
    expect(screen.getByRole("button", { name: "Publish export" })).toBeEnabled();

    await user.click(screen.getByRole("button", { name: "Publish export" }));
    await waitFor(() => {
      expect(screen.getByTestId("export-result-status")).toHaveTextContent("published");
    });
    expect(screen.getByTestId("export-status")).toHaveTextContent("success");
    expect(screen.getByTestId("export-progress")).toHaveTextContent("published");
  });

  it("surfaces export error state without inventing eligibility", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge({
      exportError: {
        code: "validation",
        message: "dataset must be one of: train, holdout",
      },
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Preview export" }));

    await waitFor(() => {
      expect(screen.getByTestId("export-status")).toHaveTextContent("error");
    });
    expect(screen.getByTestId("export-error")).toHaveTextContent(
      "validation: dataset must be one of: train, holdout",
    );
    expect(screen.queryByTestId("export-preview")).toBeNull();
  });

  it("requests export cancellation and renders the cancelled terminal state", async () => {
    const user = userEvent.setup();
    let resolveExport!: (result: ExportResult) => void;
    const pendingExport = new Promise<ExportResult>((resolve) => {
      resolveExport = resolve;
    });
    const bridge = createFakeBridge({
      pendingExport,
      exportPreview: {
        dataset: "train",
        format_id: "distill-session-jsonl-v1",
        eligible: [{ source_kind: "fixture", external_session_id: "fixture-session-ui" }],
        omitted: [],
      },
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Preview export" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Publish export" })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Publish export" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Cancel export" })).toBeVisible(),
    );
    await user.click(screen.getByRole("button", { name: "Cancel export" }));

    resolveExport({
      export_id: 1,
      dataset: "train",
      format_id: "distill-session-jsonl-v1",
      status: "cancelled",
      output_path: null,
      sha256: null,
      byte_size: null,
      record_count: 0,
      eligible: [],
      omitted: [],
      error_class: null,
      error_message: null,
    });
    await waitFor(() => {
      expect(screen.getByTestId("export-status")).toHaveTextContent("cancelled");
      expect(screen.getByTestId("export-result-status")).toHaveTextContent("cancelled");
    });
  });

  it("loads Activity and Operations panels with explicit states and no ambient fetch", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge({
      activityPage: {
        items: [
          {
            id: 3,
            event_type: "sync_completed",
            occurred_at: "2026-01-01T00:00:02Z",
            source_kind: null,
            session_id: null,
            capture_id: null,
            attempt_id: null,
            payload_json: { status: "completed" },
          },
        ],
        next_cursor: "page-2",
      },
      activityPages: {
        "page-2": {
          items: [
            {
              id: 1,
              event_type: "capture_recorded",
              occurred_at: "2026-01-01T00:00:00Z",
              source_kind: "fixture",
              session_id: 1,
              capture_id: 1,
              attempt_id: null,
              payload_json: {},
            },
          ],
          next_cursor: null,
        },
      },
      operationsPage: {
        operations_status: "ok",
        sync_runs: [
          {
            id: 2,
            status: "warning",
            cancel_requested: false,
            accepted_captures: 1,
            skipped_duplicates: 0,
            successful_attempts: 1,
            failed_attempts: 1,
            error_class: null,
            error_message: null,
            warning_details: ["sibling source unavailable"],
            sources: [],
          },
          {
            id: 1,
            status: "cancelled",
            cancel_requested: true,
            accepted_captures: 0,
            skipped_duplicates: 0,
            successful_attempts: 0,
            failed_attempts: 0,
            error_class: "cancelled",
            error_message: "sync run cancelled at a safe checkpoint",
            warning_details: [],
            sources: [],
          },
        ],
        next_sync_cursor: null,
        exports: [
          {
            id: 1,
            dataset: "train",
            format_id: "distill-session-jsonl-v1",
            status: "failed_publish",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-01T00:00:00Z",
            sha256: null,
            byte_size: null,
            record_count: 0,
            error_class: "export_failed",
            error_message: "export failed",
          },
        ],
        next_export_cursor: null,
      },
    });
    render(<App bridge={bridge} />);

    expect(screen.getByTestId("activity-status")).toHaveTextContent("idle");
    expect(screen.getByTestId("operations-status")).toHaveTextContent("idle");
    expect(screen.queryByTestId("activity-list")).not.toBeInTheDocument();
    expect(screen.queryByTestId("operations-sync-list")).not.toBeInTheDocument();

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load Activity" }));
    await waitFor(() => {
      expect(screen.getByTestId("activity-status")).toHaveTextContent("ready");
    });
    expect(screen.getByTestId("activity-list")).toHaveTextContent("sync_completed");
    await user.click(screen.getByRole("button", { name: "Load more Activity" }));
    await waitFor(() => {
      expect(screen.getByTestId("activity-list")).toHaveTextContent("capture_recorded");
    });

    await user.click(screen.getByRole("button", { name: "Load Operations" }));
    await waitFor(() => {
      expect(screen.getByTestId("operations-status")).toHaveTextContent("warning");
    });
    expect(screen.getByTestId("operations-lease-status")).toHaveTextContent("ok");
    expect(screen.getByTestId("operations-sync-list")).toHaveTextContent("warning");
    expect(screen.getByTestId("operations-sync-list")).toHaveTextContent("cancelled");
    expect(screen.getByTestId("operations-export-list")).toHaveTextContent(
      "failed_publish",
    );
  });

  it("renders Activity error state", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge({
      activityError: { code: "runtime", message: "activity failed" },
    });
    render(<App bridge={bridge} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load Activity" }));
    await waitFor(() => {
      expect(screen.getByTestId("activity-status")).toHaveTextContent("error");
    });
    expect(screen.getByTestId("activity-error")).toHaveTextContent("activity failed");
  });

  it("cancels in-flight Activity and Operations loads explicitly", async () => {
    const user = userEvent.setup();
    const pendingActivity = new Promise<ActivityListPage>(() => {});
    const pendingOperations = new Promise<OperationsPage>(() => {});
    const bridge = createFakeBridge({ pendingActivity, pendingOperations });
    render(<App bridge={bridge} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");

    await user.click(screen.getByRole("button", { name: "Load Activity" }));
    expect(screen.getByTestId("activity-status")).toHaveTextContent("loading");
    await user.click(screen.getByRole("button", { name: "Cancel Activity load" }));
    expect(screen.getByTestId("activity-status")).toHaveTextContent("cancelled");
    expect(screen.getByTestId("activity-error")).toHaveTextContent(
      "Activity load cancelled",
    );

    await user.click(screen.getByRole("button", { name: "Load Operations" }));
    expect(screen.getByTestId("operations-status")).toHaveTextContent("loading");
    await user.click(screen.getByRole("button", { name: "Cancel Operations load" }));
    expect(screen.getByTestId("operations-status")).toHaveTextContent("cancelled");
    expect(screen.getByTestId("operations-error")).toHaveTextContent(
      "Operations load cancelled",
    );
  });

  it("renders Activity and Operations empty states", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge({
      activityPage: { items: [], next_cursor: null },
      operationsPage: {
        operations_status: "ok",
        sync_runs: [],
        next_sync_cursor: null,
        exports: [],
        next_export_cursor: null,
      },
    });
    render(<App bridge={bridge} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load Activity" }));
    await waitFor(() => {
      expect(screen.getByTestId("activity-status")).toHaveTextContent("empty");
    });
    await user.click(screen.getByRole("button", { name: "Load Operations" }));
    await waitFor(() => {
      expect(screen.getByTestId("operations-status")).toHaveTextContent("empty");
    });
  });

  it("imports legacy homes through an explicit migration action", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge({
      migrationReport: {
        ok: true,
        reused_prior_import: false,
        source_fingerprint: "fp1",
        source_db_sha256: "db1",
        content_fingerprint: "c1",
        counts: {
          sources: 1,
          captures: 1,
          captures_skipped: 1,
          attempts: 1,
          facts: 1,
          sessions: 1,
          messages: 1,
          artifacts: 1,
          tags: 1,
          tag_assignments: 1,
          labels: 1,
          label_assignments: 1,
          activity_events: 1,
          exports: 1,
          exports_skipped: 0,
        },
        skips: [{ category: "capture_content", reason: "missing_or_unsafe_blob" }],
      },
    });
    render(<App bridge={bridge} />);
    expect(screen.getByTestId("migration-status")).toHaveTextContent("idle");
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.type(
      screen.getByRole("textbox", { name: "Legacy Electron home" }),
      "/tmp/legacy",
    );
    await user.click(screen.getByTestId("migration-run"));
    await waitFor(() => {
      expect(screen.getByTestId("migration-status")).toHaveTextContent("warning");
    });
    expect(screen.getByTestId("migration-report")).toHaveTextContent("fp1");
    expect(screen.getByTestId("migration-report")).toHaveTextContent("skips: 1");
  });

  it("renders migration error state without ambient fetch", async () => {
    const user = userEvent.setup();
    const failing = createFakeBridge({
      migrationError: { code: "invalid_argument", message: "paths must differ" },
    });
    render(<App bridge={failing} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.type(
      screen.getByRole("textbox", { name: "Legacy Electron home" }),
      "/tmp/legacy",
    );
    await user.click(screen.getByTestId("migration-run"));
    await waitFor(() => {
      expect(screen.getByTestId("migration-status")).toHaveTextContent("error");
    });
    expect(screen.getByTestId("migration-error")).toHaveTextContent("paths must differ");
  });

  it("cancels an in-flight migration panel request explicitly", async () => {
    const user = userEvent.setup();
    const pendingMigration = new Promise<LegacyImportReport>(() => {});
    const pending = createFakeBridge({ pendingMigration });
    render(<App bridge={pending} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.type(
      screen.getByRole("textbox", { name: "Legacy Electron home" }),
      "/tmp/legacy",
    );
    await user.click(screen.getByTestId("migration-run"));
    expect(screen.getByTestId("migration-status")).toHaveTextContent("loading");
    await user.click(screen.getByTestId("migration-cancel"));
    expect(screen.getByTestId("migration-status")).toHaveTextContent("cancelled");
  });

  it("loads Attempt history and renormalize states through the bridge only (TRC-005)", async () => {
    const user = userEvent.setup();
    const attemptsAfter = [
      {
        id: 1,
        capture_id: 1,
        parser_id: "fixture",
        parser_version: "1.0.0",
        outcome: "succeeded",
        error_class: null,
        error_message: null,
        projection_generation: 1,
        fact_count: 2,
      },
      {
        id: 2,
        capture_id: 1,
        parser_id: "fixture",
        parser_version: "1.0.0",
        outcome: "succeeded",
        error_class: null,
        error_message: null,
        projection_generation: 2,
        fact_count: 2,
      },
    ];
    let attemptCalls = 0;
    const bridge = createFakeBridge({
      sessionPage: {
        items: [
          {
            id: 1,
            source_kind: "fixture",
            external_session_id: "fixture-session-ui",
            title: "UI Fixture",
            project_path: null,
            updated_at: null,
            preview: "hello",
            message_count: 1,
            accepted_capture_count: 1,
            normalization_attempt_count: 1,
            successful_projection_generation: 1,
            labels: [],
            tags: [],
            workflow_state: "neutral",
          },
        ],
        next_cursor: null,
      },
      sessionDetail: {
        summary: {
          id: 1,
          source_kind: "fixture",
          external_session_id: "fixture-session-ui",
          title: "UI Fixture",
          accepted_capture_count: 1,
          normalization_attempt_count: 1,
          successful_projection_generation: 1,
        },
        messages: [
          { id: 1, ordinal: 0, role: "user", message_kind: "text", text: "hello" },
        ],
        artifacts: [],
        metadata_json: "{}",
      },
      attempts: [
        {
          id: 1,
          capture_id: 1,
          parser_id: "fixture",
          parser_version: "1.0.0",
          outcome: "succeeded",
          error_class: null,
          error_message: null,
          projection_generation: 1,
          fact_count: 2,
        },
      ],
      renormalizeReport: {
        capture_id: 1,
        attempt_id: 2,
        outcome: "succeeded",
        parser_id: "fixture",
        parser_version: "1.0.0",
      },
    });
    const originalAttempts = bridge.captureAttempts.bind(bridge);
    bridge.captureAttempts = async (home, captureId) => {
      attemptCalls += 1;
      if (attemptCalls === 1) return originalAttempts(home, captureId);
      return attemptsAfter;
    };

    render(<App bridge={bridge} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await user.click(screen.getByRole("button", { name: /UI Fixture/ }));
    expect(screen.getByTestId("attempt-history-panel")).toBeInTheDocument();
    expect(screen.getByTestId("attempt-history-status")).toHaveTextContent("idle");

    await user.click(screen.getByTestId("load-attempt-history"));
    await waitFor(() =>
      expect(screen.getByTestId("attempt-history-status")).toHaveTextContent("ready"),
    );
    expect(screen.getByTestId("attempt-history-list")).toHaveTextContent("fixture/1.0.0");
    expect(screen.getByTestId("renormalize-capture")).not.toBeDisabled();

    await user.click(screen.getByTestId("renormalize-capture"));
    await waitFor(() =>
      expect(screen.getByTestId("renormalize-status")).toHaveTextContent("ready"),
    );
    expect(screen.getByTestId("renormalize-report")).toHaveTextContent("attempt 2");
    expect(screen.getByTestId("attempt-history-list")).toHaveTextContent("#2");
  });

  it("renders Attempt history error without ambient authority", async () => {
    const user = userEvent.setup();
    const failing = createFakeBridge({
      sessionPage: {
        items: [
          {
            id: 1,
            source_kind: "fixture",
            external_session_id: "fixture-session-ui",
            title: "UI Fixture",
            project_path: null,
            updated_at: null,
            preview: "hello",
            message_count: 1,
            accepted_capture_count: 1,
            normalization_attempt_count: 1,
            successful_projection_generation: 1,
            labels: [],
            tags: [],
            workflow_state: "neutral",
          },
        ],
        next_cursor: null,
      },
      sessionDetail: {
        summary: {
          id: 1,
          source_kind: "fixture",
          external_session_id: "fixture-session-ui",
          title: "UI Fixture",
          accepted_capture_count: 1,
          normalization_attempt_count: 1,
          successful_projection_generation: 1,
        },
        messages: [],
        artifacts: [],
        metadata_json: "{}",
      },
      attemptError: { code: "not_found", message: "capture missing" },
    });
    render(<App bridge={failing} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await user.click(screen.getByRole("button", { name: /UI Fixture/ }));
    await user.click(screen.getByTestId("load-attempt-history"));
    await waitFor(() =>
      expect(screen.getByTestId("attempt-history-error")).toHaveTextContent("not_found"),
    );
    expect(screen.getByTestId("attempt-history-status")).toHaveTextContent("error");
  });

  it("ignores Attempt results when the selected Session changes mid-flight", async () => {
    const user = userEvent.setup();
    let resolveAttempts!: (value: import("./types").AttemptSummary[]) => void;
    const pendingAttempts = new Promise<import("./types").AttemptSummary[]>((resolve) => {
      resolveAttempts = resolve;
    });
    const bridge = createFakeBridge({
      sessionPage: {
        items: [
          {
            id: 1,
            source_kind: "fixture",
            external_session_id: "fixture-session-a",
            title: "Session A",
            project_path: null,
            updated_at: null,
            preview: "a",
            message_count: 1,
            accepted_capture_count: 1,
            normalization_attempt_count: 1,
            successful_projection_generation: 1,
            labels: [],
            tags: [],
            workflow_state: "neutral",
          },
          {
            id: 2,
            source_kind: "fixture",
            external_session_id: "fixture-session-b",
            title: "Session B",
            project_path: null,
            updated_at: null,
            preview: "b",
            message_count: 1,
            accepted_capture_count: 1,
            normalization_attempt_count: 1,
            successful_projection_generation: 1,
            labels: [],
            tags: [],
            workflow_state: "neutral",
          },
        ],
        next_cursor: null,
      },
    });
    bridge.sessionDetail = async (_home, request) => ({
      summary: {
        id: request.external_session_id.endsWith("a") ? 1 : 2,
        source_kind: request.source_kind,
        external_session_id: request.external_session_id,
        title: request.external_session_id.endsWith("a") ? "Session A" : "Session B",
        accepted_capture_count: 1,
        normalization_attempt_count: 1,
        successful_projection_generation: 1,
      },
      messages: [],
      artifacts: [],
      metadata_json: "{}",
    });
    bridge.captureAttempts = async () => pendingAttempts;

    render(<App bridge={bridge} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await user.click(screen.getByRole("button", { name: /Session A/ }));
    await user.click(screen.getByTestId("load-attempt-history"));
    await waitFor(() =>
      expect(screen.getByTestId("attempt-history-status")).toHaveTextContent("loading"),
    );

    await user.click(screen.getByRole("button", { name: /Session B/ }));
    await waitFor(() => expect(screen.getByText("Session B")).toBeInTheDocument());
    resolveAttempts([
      {
        id: 1,
        capture_id: 1,
        parser_id: "fixture",
        parser_version: "1.0.0",
        outcome: "succeeded",
        error_class: null,
        error_message: null,
        projection_generation: 1,
        fact_count: 1,
      },
    ]);
    await waitFor(() =>
      expect(screen.getByTestId("attempt-history-status")).toHaveTextContent("idle"),
    );
    expect(screen.getByTestId("renormalize-capture")).toBeDisabled();
  });
});
