/**
 * Focused multi-Source Sync preference seam for the thin React caller.
 */

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import type {
  DistillBridge,
  ExportProgress,
  FixtureJourneyPhase,
  SourcePreference,
  SyncProgress,
  SyncRunResult,
} from "./types";

type PreferenceCall = {
  home: string;
  kind: string;
  enabled: boolean;
  configuredRoot: string | null;
};

/**
 * Build a recording DistillBridge fake for multi-Source Sync assertions.
 * @param options - controllable Sync result and listed preferences
 */
function createMultisourceBridge(options?: {
  syncResult?: SyncRunResult;
  listSources?: SourcePreference[];
}): {
  bridge: DistillBridge;
  preferenceCalls: PreferenceCall[];
  startSyncCalls: Array<{ home: string; sourceKinds?: string[] }>;
} {
  const preferenceCalls: PreferenceCall[] = [];
  const startSyncCalls: Array<{ home: string; sourceKinds?: string[] }> = [];
  const listeners = new Set<(phase: FixtureJourneyPhase) => void>();
  const syncListeners = new Set<(progress: SyncProgress) => void>();
  const exportListeners = new Set<(progress: ExportProgress) => void>();
  const persisted = new Map<string, SourcePreference>();

  const bridge: DistillBridge = {
    async runFixtureJourney() {
      throw { code: "unused", message: "fixture journey unused in multisource suite" };
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
      throw { code: "unused", message: "unused" };
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
      if (options?.listSources) return options.listSources;
      return [...persisted.values()];
    },
    async setSourcePreference(home, kind, enabled, configuredRoot) {
      const preference: SourcePreference = {
        kind,
        enabled,
        configured_root: configuredRoot ?? null,
        display_name: kind,
        data_root: null,
      };
      preferenceCalls.push({
        home,
        kind,
        enabled,
        configuredRoot: configuredRoot ?? null,
      });
      persisted.set(kind, preference);
      return preference;
    },
    async startSync(home, sourceKinds) {
      startSyncCalls.push({ home, sourceKinds });
      syncListeners.forEach((listener) =>
        listener({ type: "run_started", sync_run_id: 44 }),
      );
      return (
        options?.syncResult ?? {
          run: {
            id: 44,
            status: "warning",
            cancel_requested: false,
            accepted_captures: 1,
            skipped_duplicates: 0,
            successful_attempts: 1,
            failed_attempts: 0,
            error_class: null,
            error_message: null,
            warning_details: ["codex: source unavailable"],
            sources: [
              {
                source_kind: "codex",
                status: "failed",
                accepted_captures: 0,
                skipped_duplicates: 0,
                successful_attempts: 0,
                failed_attempts: 0,
                error_class: "source_unavailable",
                error_message: "source unavailable",
              },
              {
                source_kind: "opencode",
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
            { source_kind: "opencode", external_session_id: "opencode-session" },
          ],
        }
      );
    },
    async syncStatus() {
      return {
        id: 44,
        status: "warning",
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
      throw { code: "unused", message: "unused" };
    },
    async listSessions() {
      return { items: [], next_cursor: null };
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
    async previewExport() {
      throw { code: "unused", message: "unused" };
    },
    async publishExport() {
      throw { code: "unused", message: "unused" };
    },
    async cancelExport() {
      return false;
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

  return { bridge, preferenceCalls, startSyncCalls };
}

describe("App multi-source Sync preferences", () => {
  it("persists selected codex and opencode roots and renders safe warning details", async () => {
    const safeWarning = "codex: source unavailable (path redacted)";
    const forbiddenSourcePayload = "secret-token-from-provider";
    const user = userEvent.setup();
    const { bridge, preferenceCalls, startSyncCalls } = createMultisourceBridge({
      syncResult: {
        run: {
          id: 44,
          status: "warning",
          cancel_requested: false,
          accepted_captures: 1,
          skipped_duplicates: 0,
          successful_attempts: 1,
          failed_attempts: 0,
          error_class: null,
          error_message: null,
          warning_details: [safeWarning],
          sources: [],
        },
        session_identities: [],
      },
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByTestId("source-enabled-fixture"));
    await user.click(screen.getByTestId("source-enabled-codex"));
    await user.type(screen.getByTestId("source-root-codex"), "/tmp/codex-root");
    await user.click(screen.getByTestId("source-enabled-opencode"));
    await user.type(screen.getByTestId("source-root-opencode"), "/tmp/opencode-root");
    await user.click(screen.getByRole("button", { name: /start sync run/i }));

    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("warning");
    });

    expect(preferenceCalls).toEqual([
      {
        home: "/tmp/home",
        kind: "fixture",
        enabled: false,
        configuredRoot: null,
      },
      {
        home: "/tmp/home",
        kind: "codex",
        enabled: true,
        configuredRoot: "/tmp/codex-root",
      },
      {
        home: "/tmp/home",
        kind: "claude_code",
        enabled: false,
        configuredRoot: null,
      },
      {
        home: "/tmp/home",
        kind: "opencode",
        enabled: true,
        configuredRoot: "/tmp/opencode-root",
      },
      {
        home: "/tmp/home",
        kind: "droid",
        enabled: false,
        configuredRoot: null,
      },
    ]);
    expect(startSyncCalls).toEqual([
      { home: "/tmp/home", sourceKinds: ["codex", "opencode"] },
    ]);

    const warningDetails = screen.getByTestId("sync-warning-details");
    expect(warningDetails).toHaveTextContent(safeWarning);
    expect(warningDetails.textContent).not.toContain(forbiddenSourcePayload);
    expect(screen.queryByTestId("source-panel")).not.toBeInTheDocument();
  });

  it("hydrates an existing enabled provider root before persisting untouched drafts", async () => {
    const user = userEvent.setup();
    const { bridge, preferenceCalls, startSyncCalls } = createMultisourceBridge({
      listSources: [
        {
          kind: "codex",
          enabled: true,
          configured_root: "/tmp/existing-codex-root",
          display_name: "Codex",
          data_root: "/tmp/existing-codex-root",
        },
      ],
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: /start sync run/i }));

    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("warning");
    });

    expect(preferenceCalls.find((call) => call.kind === "codex")).toEqual({
      home: "/tmp/home",
      kind: "codex",
      enabled: true,
      configuredRoot: "/tmp/existing-codex-root",
    });
    expect(startSyncCalls[0]?.sourceKinds).toContain("codex");
  });
});
