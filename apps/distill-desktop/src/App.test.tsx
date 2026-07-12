/**
 * Renderer seam: React first-run UI against one typed Distill bridge fake.
 */

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import type {
  DistillBridge,
  FixtureJourneyPhase,
  FixtureJourneyResult,
  HealthReport,
  HostError,
  RepairReport,
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
}): DistillBridge {
  const listeners = new Set<(phase: FixtureJourneyPhase) => void>();
  const syncListeners = new Set<(progress: import("./types").SyncProgress) => void>();
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
      onProgress: () => () => undefined,
      onSyncProgress: () => () => undefined,
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
});
