/**
 * Deterministic major-state HTML assertions for the Distill renderer.
 */

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import type {
  DistillBridge,
  ExportProgress,
  FixtureJourneyPhase,
  HostError,
  LegacyImportReport,
  SessionListPage,
  SyncProgress,
  SyncRunResult,
} from "./types";

/**
 * Build a controllable bridge fake for major UI state evidence.
 */
function createBridge(options?: {
  sessionPage?: SessionListPage;
  pendingSessions?: Promise<SessionListPage>;
  syncResult?: SyncRunResult;
  pendingSync?: Promise<SyncRunResult>;
  migrationReport?: LegacyImportReport;
  pendingMigration?: Promise<LegacyImportReport>;
  migrationError?: HostError;
  exportError?: HostError;
  pendingExport?: Promise<import("./types").ExportResult>;
  error?: HostError;
}): DistillBridge {
  const listeners = new Set<(phase: FixtureJourneyPhase) => void>();
  const syncListeners = new Set<(progress: SyncProgress) => void>();
  const exportListeners = new Set<(progress: ExportProgress) => void>();
  return {
    async runFixtureJourney() {
      if (options?.error) throw options.error;
      throw { code: "unused", message: "unused in state suite" };
    },
    async health() {
      return {
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
      };
    },
    async importLegacy() {
      if (options?.pendingMigration) return options.pendingMigration;
      if (options?.migrationError) throw options.migrationError;
      return (
        options?.migrationReport ?? {
          ok: true,
          reused_prior_import: false,
          source_fingerprint: "fp",
          source_db_sha256: "sha",
          content_fingerprint: "cfp",
          counts: {
            sources: 1,
            captures: 1,
            captures_skipped: 0,
            attempts: 1,
            facts: 1,
            sessions: 1,
            messages: 1,
            artifacts: 0,
            tags: 0,
            tag_assignments: 0,
            labels: 0,
            label_assignments: 0,
            activity_events: 0,
            exports: 0,
            exports_skipped: 0,
          },
          skips: [],
        }
      );
    },
    async repair(_home, confirm) {
      if (!confirm)
        throw { code: "validation", message: "repair requires explicit confirmation" };
      return {
        actions: [],
        health_after: await this.health("/tmp"),
      };
    },
    async listSources() {
      return [];
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
        listener({ type: "run_started", sync_run_id: 3 }),
      );
      if (options?.pendingSync) return options.pendingSync;
      return (
        options?.syncResult ?? {
          run: {
            id: 3,
            status: "completed",
            cancel_requested: false,
            accepted_captures: 1,
            skipped_duplicates: 0,
            successful_attempts: 1,
            failed_attempts: 0,
            error_class: null,
            error_message: null,
            sources: [],
          },
          session_identities: [],
        }
      );
    },
    async syncStatus() {
      return {
        id: 3,
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
    async cancelSync() {
      return {
        id: 3,
        status: "cancelled",
        cancel_requested: true,
        accepted_captures: 0,
        skipped_duplicates: 0,
        successful_attempts: 0,
        failed_attempts: 0,
        error_class: "cancelled",
        error_message: "cancelled",
        sources: [],
      };
    },
    async listSessions() {
      if (options?.error) throw options.error;
      if (options?.pendingSessions) return options.pendingSessions;
      return options?.sessionPage ?? { items: [], next_cursor: null };
    },
    async sessionDetail() {
      return null;
    },
    async addSessionTag() {
      throw { code: "unused", message: "unused" };
    },
    async removeSessionTag() {
      throw { code: "unused", message: "unused" };
    },
    async toggleSessionLabel() {
      throw { code: "unused", message: "unused" };
    },
    async previewExport(_home, dataset) {
      if (options?.exportError) throw options.exportError;
      return {
        dataset,
        format_id: "distill-session-jsonl-v1",
        eligible: [{ source_kind: "fixture", external_session_id: "s1" }],
        omitted: [],
      };
    },
    async publishExport(_home, dataset) {
      if (options?.pendingExport) return options.pendingExport;
      if (options?.exportError) throw options.exportError;
      return {
        export_id: 1,
        dataset,
        format_id: "distill-session-jsonl-v1",
        status: "published",
        output_path: "/tmp/out.jsonl",
        sha256: "abc",
        byte_size: 12,
        record_count: 1,
        eligible: [],
        omitted: [],
        error_class: null,
        error_message: null,
      };
    },
    async cancelExport() {
      return true;
    },
    async listActivity() {
      return { items: [], next_cursor: null };
    },
    async listOperations() {
      return {
        operations_status: "ok",
        sync_runs: [],
        next_sync_cursor: null,
        exports: [],
        next_export_cursor: null,
      };
    },
    async captureAttempts() {
      return [];
    },
    async renormalizeCapture() {
      throw { code: "unused", message: "unused in state suite" };
    },
    onProgress(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    onSyncProgress(listener) {
      syncListeners.add(listener);
      return () => syncListeners.delete(listener);
    },
    onExportProgress(listener) {
      exportListeners.add(listener);
      return () => exportListeners.delete(listener);
    },
  };
}

const populatedPage: SessionListPage = {
  items: [
    {
      id: 4,
      source_kind: "fixture",
      external_session_id: "populated",
      title: "Populated session",
      project_path: "/demo",
      updated_at: "2026-01-01T00:00:00Z",
      preview: "preview",
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

function stateMarkers() {
  return {
    journey: screen.getByTestId("status").textContent,
    migration: screen.getByTestId("migration-status").textContent,
    session: screen.getByTestId("session-explorer-status").textContent,
    export: screen.getByTestId("export-status").textContent,
  };
}

describe("renderer major visual states", () => {
  it("starts in first-run idle with labeled landmarks", () => {
    render(<App bridge={createBridge()} />);
    expect(screen.getByTestId("status")).toHaveTextContent("idle");
    expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("idle");
    expect(screen.getByTestId("migration-status")).toHaveTextContent("idle");
    expect(screen.getByTestId("export-status")).toHaveTextContent("idle");
    expect(screen.getByRole("main")).toBeInTheDocument();
    expect(screen.getByRole("banner")).toBeInTheDocument();
    expect(stateMarkers()).toMatchInlineSnapshot(`
      {
        "export": "Export: idle",
        "journey": "idle",
        "migration": "Migration status: idle",
        "session": "Sessions: idle",
      }
    `);
  });

  it("distinguishes first-load loading from refreshing while rows stay visible", async () => {
    const user = userEvent.setup();
    let resolveFirst!: (page: SessionListPage) => void;
    let resolveSecond!: (page: SessionListPage) => void;
    let call = 0;
    const bridge = createBridge();
    bridge.listSessions = async () => {
      call += 1;
      if (call === 1) {
        return new Promise<SessionListPage>((resolve) => {
          resolveFirst = resolve;
        });
      }
      return new Promise<SessionListPage>((resolve) => {
        resolveSecond = resolve;
      });
    };

    render(<App bridge={bridge} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("loading");
    expect(screen.getByTestId("session-explorer")).toHaveAttribute("aria-busy", "true");
    const markers = [{ phase: "loading", ...stateMarkers() }];
    resolveFirst(populatedPage);
    await waitFor(() => {
      expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("ready");
    });
    expect(screen.getByTestId("session-list")).toHaveTextContent("Populated session");

    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("refreshing");
    expect(screen.getByTestId("session-list")).toHaveTextContent("Populated session");
    expect(screen.getByTestId("session-explorer")).toHaveAttribute("aria-busy", "true");
    markers.push({ phase: "refreshing", ...stateMarkers() });
    resolveSecond(populatedPage);
    await waitFor(() => {
      expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("ready");
    });
    markers.push({ phase: "populated", ...stateMarkers() });
    expect(markers).toMatchInlineSnapshot(`
      [
        {
          "export": "Export: idle",
          "journey": "idle",
          "migration": "Migration status: idle",
          "phase": "loading",
          "session": "Sessions: loading",
        },
        {
          "export": "Export: idle",
          "journey": "idle",
          "migration": "Migration status: idle",
          "phase": "refreshing",
          "session": "Sessions: refreshing",
        },
        {
          "export": "Export: idle",
          "journey": "idle",
          "migration": "Migration status: idle",
          "phase": "populated",
          "session": "Sessions: ready",
        },
      ]
    `);
  });

  it("covers empty, error, cancelled, warning, migration, and export states", async () => {
    const user = userEvent.setup();
    const markers: Array<Record<string, string | null>> = [];
    let resolveSessions!: (page: SessionListPage) => void;
    const pendingSessions = new Promise<SessionListPage>((resolve) => {
      resolveSessions = resolve;
    });

    const { rerender } = render(
      <App
        bridge={createBridge({
          pendingSessions,
          sessionPage: { items: [], next_cursor: null },
        })}
      />,
    );
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("loading");
    await user.click(screen.getByRole("button", { name: "Cancel session load" }));
    expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("cancelled");
    markers.push({ phase: "cancelled", ...stateMarkers() });
    resolveSessions({ items: [], next_cursor: null });

    rerender(
      <App bridge={createBridge({ sessionPage: { items: [], next_cursor: null } })} />,
    );
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await waitFor(() => {
      expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("empty");
    });
    markers.push({ phase: "empty", ...stateMarkers() });
    expect(screen.getByText("No sessions match this query.")).toBeInTheDocument();

    rerender(
      <App
        bridge={createBridge({
          error: { code: "query_failed", message: "boom" },
        })}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await waitFor(() => {
      expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("error");
    });
    markers.push({ phase: "error", ...stateMarkers() });
    expect(screen.getByTestId("session-explorer-error")).toHaveAttribute("role", "alert");

    rerender(
      <App
        bridge={createBridge({
          syncResult: {
            run: {
              id: 9,
              status: "warning",
              cancel_requested: false,
              accepted_captures: 0,
              skipped_duplicates: 0,
              successful_attempts: 0,
              failed_attempts: 1,
              error_class: null,
              error_message: null,
              warning_details: ["partial source failure"],
              sources: [],
            },
            session_identities: [],
          },
        })}
      />,
    );
    await user.type(
      screen.getByRole("textbox", { name: "Fixture root" }),
      "/tmp/fixture",
    );
    await user.click(screen.getByRole("button", { name: "Start Sync Run" }));
    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("warning");
      expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("warning");
    });
    markers.push({ phase: "warning", ...stateMarkers() });

    rerender(
      <App
        bridge={createBridge({
          migrationReport: {
            ok: false,
            reused_prior_import: false,
            source_fingerprint: "fp",
            source_db_sha256: "sha",
            content_fingerprint: "cfp",
            counts: {
              sources: 0,
              captures: 0,
              captures_skipped: 0,
              attempts: 0,
              facts: 0,
              sessions: 0,
              messages: 0,
              artifacts: 0,
              tags: 0,
              tag_assignments: 0,
              labels: 0,
              label_assignments: 0,
              activity_events: 0,
              exports: 0,
              exports_skipped: 0,
            },
            skips: [{ category: "export", reason: "unsupported" }],
          },
        })}
      />,
    );
    await user.type(
      screen.getByRole("textbox", { name: "Legacy Electron home" }),
      "/tmp/legacy",
    );
    await user.click(screen.getByTestId("migration-run"));
    await waitFor(() => {
      expect(screen.getByTestId("migration-status")).toHaveTextContent("warning");
    });
    expect(screen.getByTestId("migration-report")).toBeInTheDocument();
    markers.push({ phase: "migration-warning", ...stateMarkers() });

    await user.click(screen.getByRole("button", { name: "Preview export" }));
    await waitFor(() => {
      expect(screen.getByTestId("export-status")).toHaveTextContent("success");
    });
    expect(screen.getByTestId("export-preview")).toBeInTheDocument();
    markers.push({ phase: "export-preview", ...stateMarkers() });
    await user.click(screen.getByRole("button", { name: "Publish export" }));
    await waitFor(() => {
      expect(screen.getByTestId("export-result-status")).toHaveTextContent("published");
    });
    markers.push({ phase: "export-published", ...stateMarkers() });
    expect(markers).toMatchInlineSnapshot(`
      [
        {
          "export": "Export: idle",
          "journey": "idle",
          "migration": "Migration status: idle",
          "phase": "cancelled",
          "session": "Sessions: cancelled",
        },
        {
          "export": "Export: idle",
          "journey": "idle",
          "migration": "Migration status: idle",
          "phase": "empty",
          "session": "Sessions: empty",
        },
        {
          "export": "Export: idle",
          "journey": "idle",
          "migration": "Migration status: idle",
          "phase": "error",
          "session": "Sessions: error",
        },
        {
          "export": "Export: idle",
          "journey": "warning",
          "migration": "Migration status: idle",
          "phase": "warning",
          "session": "Sessions: warning",
        },
        {
          "export": "Export: idle",
          "journey": "warning",
          "migration": "Migration status: warning",
          "phase": "migration-warning",
          "session": "Sessions: warning",
        },
        {
          "export": "Export: success",
          "journey": "warning",
          "migration": "Migration status: warning",
          "phase": "export-preview",
          "session": "Sessions: warning",
        },
        {
          "export": "Export: success",
          "journey": "warning",
          "migration": "Migration status: warning",
          "phase": "export-published",
          "session": "Sessions: warning",
        },
      ]
    `);
  });

  it("renders Attempt history and failed renormalize warning states", async () => {
    const user = userEvent.setup();
    const bridge = createBridge({
      sessionPage: populatedPage,
    });
    bridge.sessionDetail = async () => ({
      summary: {
        id: 4,
        source_kind: "fixture",
        external_session_id: "populated",
        title: "Populated session",
        accepted_capture_count: 1,
        normalization_attempt_count: 1,
        successful_projection_generation: 1,
      },
      messages: [],
      artifacts: [],
      metadata_json: "{}",
    });
    bridge.listActivity = async () => ({
      items: [
        {
          id: 1,
          event_type: "capture_recorded",
          occurred_at: "2026-01-01T00:00:00Z",
          source_kind: "fixture",
          session_id: 4,
          capture_id: 9,
          attempt_id: null,
          payload_json: {},
        },
      ],
      next_cursor: null,
    });
    bridge.captureAttempts = async () => [
      {
        id: 1,
        capture_id: 9,
        parser_id: "fixture",
        parser_version: "1.0.0",
        outcome: "succeeded",
        error_class: null,
        error_message: null,
        projection_generation: 1,
        fact_count: 1,
      },
    ];
    bridge.renormalizeCapture = async () => ({
      capture_id: 9,
      attempt_id: 2,
      outcome: "failed",
      parser_id: "fixture",
      parser_version: "1.0.0",
    });

    render(<App bridge={bridge} />);
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await user.click(screen.getByRole("button", { name: /Populated session/ }));
    expect(screen.getByTestId("attempt-history-status")).toHaveTextContent("idle");
    await user.click(screen.getByTestId("load-attempt-history"));
    await waitFor(() =>
      expect(screen.getByTestId("attempt-history-status")).toHaveTextContent("ready"),
    );
    await user.click(screen.getByTestId("renormalize-capture"));
    await waitFor(() =>
      expect(screen.getByTestId("renormalize-status")).toHaveTextContent("warning"),
    );
    expect(screen.getByTestId("renormalize-report")).toHaveTextContent("failed");
  });
});
