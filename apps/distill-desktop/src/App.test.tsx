/**
 * Renderer seam: React first-run UI against one typed Distill bridge fake.
 */

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import type {
  CurationMutationResult,
  DistillBridge,
  ExportPreview,
  ExportProgress,
  ExportResult,
  FixtureJourneyPhase,
  FixtureJourneyResult,
  HealthReport,
  HostError,
  RepairReport,
  SessionDetail,
  SessionListItem,
  SessionListPage,
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
      repair: async () => ({
        actions: [],
        health_after: sampleResult().health,
      }),
      listSources: async () => [],
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
    expect(screen.getByRole("button", { name: /repair library/i })).toBeDisabled();
    await user.click(screen.getByLabelText(/confirm destructive repair/i));
    await user.click(screen.getByRole("button", { name: /repair library/i }));

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
});
