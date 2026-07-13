/**
 * Keyboard, focus-return, dialog, and pointer-only contracts for the Distill renderer.
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import type {
  DistillBridge,
  ExportProgress,
  FixtureJourneyPhase,
  HostError,
  SessionDetail,
  SessionListPage,
  SyncProgress,
  SyncRunResult,
  SyncRunSummary,
} from "./types";

/**
 * Build a minimal typed bridge fake for accessibility contracts.
 */
function createBridge(options?: {
  pendingSessions?: Promise<SessionListPage>;
  sessionPage?: SessionListPage;
  sessionDetail?: SessionDetail | null;
  pendingSync?: Promise<SyncRunResult>;
  cancelResult?: SyncRunSummary;
  pendingExport?: Promise<import("./types").ExportResult>;
  pendingMigration?: Promise<import("./types").LegacyImportReport>;
  pendingActivity?: Promise<import("./types").ActivityListPage>;
  pendingOperations?: Promise<import("./types").OperationsPage>;
  error?: HostError;
}): DistillBridge {
  const listeners = new Set<(phase: FixtureJourneyPhase) => void>();
  const syncListeners = new Set<(progress: SyncProgress) => void>();
  const exportListeners = new Set<(progress: ExportProgress) => void>();
  const sessionPage: SessionListPage = options?.sessionPage ?? {
    items: [
      {
        id: 1,
        source_kind: "fixture",
        external_session_id: "a11y-session",
        title: "A11y session",
        project_path: null,
        updated_at: null,
        preview: null,
        message_count: 1,
        accepted_capture_count: 1,
        normalization_attempt_count: 1,
        successful_projection_generation: 1,
        labels: [],
        tags: [{ id: 9, name: "alpha", kind: "manual", origin: "manual" }],
        workflow_state: "neutral",
      },
    ],
    next_cursor: null,
  };
  const sessionDetail: SessionDetail = options?.sessionDetail ?? {
    summary: {
      id: 1,
      source_kind: "fixture",
      external_session_id: "a11y-session",
      title: "A11y session",
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
        text: "hello",
      },
    ],
    artifacts: [],
    metadata_json: "{}",
    tags: [{ id: 9, name: "alpha", kind: "manual", origin: "manual" }],
    labels: [],
    workflow_state: "neutral",
  };
  return {
    async runFixtureJourney() {
      throw { code: "unused", message: "unused" };
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
      return {
        ok: true,
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
        skips: [],
      };
    },
    async repair(_home, confirm) {
      if (!confirm)
        throw { code: "validation", message: "repair requires explicit confirmation" };
      return {
        actions: [{ name: "removed_staging_partials", count: 0 }],
        health_after: await this.health("/tmp"),
      };
    },
    async listSources() {
      return [];
    },
    async detectSources() {
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
        listener({ type: "run_started", sync_run_id: 7 }),
      );
      if (options?.pendingSync) return options.pendingSync;
      return {
        run: {
          id: 7,
          status: "completed",
          cancel_requested: false,
          accepted_captures: 0,
          skipped_duplicates: 0,
          successful_attempts: 0,
          failed_attempts: 0,
          error_class: null,
          error_message: null,
          sources: [],
        },
        session_identities: [],
      };
    },
    async syncStatus() {
      return {
        id: 7,
        status: "running",
        cancel_requested: false,
        accepted_captures: 0,
        skipped_duplicates: 0,
        successful_attempts: 0,
        failed_attempts: 0,
        error_class: null,
        error_message: null,
        sources: [],
      };
    },
    async cancelSync() {
      return (
        options?.cancelResult ?? {
          id: 7,
          status: "cancelled",
          cancel_requested: true,
          accepted_captures: 0,
          skipped_duplicates: 0,
          successful_attempts: 0,
          failed_attempts: 0,
          error_class: "cancelled",
          error_message: "cancelled",
          sources: [],
        }
      );
    },
    async listSessions() {
      if (options?.error) throw options.error;
      if (options?.pendingSessions) return options.pendingSessions;
      return sessionPage;
    },
    async sessionDetail() {
      return sessionDetail;
    },
    async addSessionTag(_home, request) {
      return {
        changed: true,
        identity: {
          source_kind: request.source_kind,
          external_session_id: request.external_session_id,
        },
        tags: [{ id: 1, name: request.name, kind: "manual", origin: "manual" }],
        labels: [],
        workflow_state: "neutral",
      };
    },
    async removeSessionTag(_home, request) {
      return {
        changed: true,
        identity: {
          source_kind: request.source_kind,
          external_session_id: request.external_session_id,
        },
        tags: [],
        labels: [],
        workflow_state: "neutral",
      };
    },
    async toggleSessionLabel(_home, request) {
      return {
        changed: true,
        identity: {
          source_kind: request.source_kind,
          external_session_id: request.external_session_id,
        },
        tags: sessionDetail.tags ?? [],
        labels: [
          {
            id: 1,
            name: request.name,
            scope: "session",
            origin: "manual",
          },
        ],
        workflow_state: "train_ready",
      };
    },
    async previewExport(_home, dataset) {
      return {
        dataset,
        format_id: "distill-session-jsonl-v1",
        eligible: [],
        omitted: [],
      };
    },
    async publishExport(_home, dataset) {
      if (options?.pendingExport) return options.pendingExport;
      return {
        export_id: 1,
        dataset,
        format_id: "distill-session-jsonl-v1",
        status: "published",
        output_path: "/tmp/out.jsonl",
        sha256: "abc",
        byte_size: 1,
        record_count: 0,
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
      if (options?.pendingActivity) return options.pendingActivity;
      return { items: [], next_cursor: null };
    },
    async listOperations() {
      if (options?.pendingOperations) return options.pendingOperations;
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
      throw { code: "unused", message: "unused in a11y suite" };
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

const appSource = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), "App.tsx"),
  "utf8",
).replace(/\s+/g, " ");

/**
 * Static source audit for handlers and tab stops attached to non-widget JSX.
 * React delegates events, so rendered DOM `onclick` inspection is not evidence.
 */
function nonWidgetInteractiveProps(): string[] {
  const nonWidgets =
    "(?:div|span|p|li|article|section|header|main|footer|h[1-6]|pre|ul|ol|dl|dt|dd)";
  return [
    ...appSource.matchAll(new RegExp(`<${nonWidgets}\\b[^>]*\\bonClick\\s*=`, "g")),
    ...appSource.matchAll(new RegExp(`<${nonWidgets}\\b[^>]*\\btabIndex\\s*=`, "g")),
  ].map((match) => match[0]);
}

describe("renderer accessibility contracts", () => {
  it("submits session search with Enter and keeps actions on native controls", async () => {
    const user = userEvent.setup();
    const bridge = createBridge();
    render(<App bridge={bridge} />);

    expect(nonWidgetInteractiveProps()).toEqual([]);
    expect(screen.getByRole("main")).toBeInTheDocument();

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    const search = screen.getByRole("textbox", { name: "Search sessions" });
    await user.click(search);
    await user.keyboard("{Enter}");

    await waitFor(() => {
      expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("ready");
    });
    expect(screen.getByRole("button", { name: /A11y session/ })).toBeInTheDocument();
  });

  it("loads the selected workflow lane and session through keyboard controls", async () => {
    const user = userEvent.setup();
    render(<App bridge={createBridge()} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    const lane = screen.getByRole("combobox", { name: "Workflow lane" });
    await user.selectOptions(lane, "favorites");
    lane.focus();
    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("ready");
    });

    const session = screen.getByRole("button", { name: /A11y session/ });
    session.focus();
    await user.keyboard(" ");
    await waitFor(() => {
      expect(screen.getByTestId("session-detail-panel")).toBeInTheDocument();
    });
    expect(session).toHaveAttribute("aria-pressed", "true");
  });

  it("opens a modal repair dialog with Escape cancel and focus return", async () => {
    const user = userEvent.setup();
    const bridge = createBridge();
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    const repair = screen.getByRole("button", { name: /repair library/i });
    await user.click(repair);

    const dialog = screen.getByRole("dialog", { name: /confirm destructive repair/i });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveAccessibleDescription(/staging partials/i);
    expect(screen.getByRole("button", { name: /cancel repair/i })).toHaveFocus();

    await user.keyboard("{Tab}");
    expect(screen.getByRole("button", { name: /confirm repair/i })).toHaveFocus();
    await user.keyboard("{Tab}");
    expect(screen.getByRole("button", { name: /cancel repair/i })).toHaveFocus();
    await user.keyboard("{Escape}");
    await waitFor(() => {
      expect(repair).toHaveFocus();
    });
    expect(dialog).not.toHaveAttribute("open");
  });

  it("returns focus to Load sessions after cancelling a pending load", async () => {
    const user = userEvent.setup();
    let resolveSessions!: (page: SessionListPage) => void;
    const pendingSessions = new Promise<SessionListPage>((resolve) => {
      resolveSessions = resolve;
    });
    const bridge = createBridge({ pendingSessions });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    const load = screen.getByRole("button", { name: "Load sessions" });
    await user.click(load);
    expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("loading");
    await user.click(screen.getByRole("button", { name: "Cancel session load" }));
    expect(screen.getByTestId("session-explorer-status")).toHaveTextContent("cancelled");
    await waitFor(() => {
      expect(load).toHaveFocus();
    });
    resolveSessions({ items: [], next_cursor: null });
  });

  it("returns focus after cancelling Sync Run and export publication", async () => {
    const user = userEvent.setup();
    let resolveSync!: (result: SyncRunResult) => void;
    const pendingSync = new Promise<SyncRunResult>((resolve) => {
      resolveSync = resolve;
    });
    let resolveExport!: (result: import("./types").ExportResult) => void;
    const pendingExport = new Promise<import("./types").ExportResult>((resolve) => {
      resolveExport = resolve;
    });
    render(<App bridge={createBridge({ pendingSync, pendingExport })} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    const startSync = screen.getByRole("button", { name: "Start Sync Run" });
    await user.click(startSync);
    const cancelSync = await screen.findByRole("button", { name: "Cancel Sync" });
    cancelSync.focus();
    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(startSync).toHaveFocus();
    });

    await user.click(screen.getByRole("button", { name: "Preview export" }));
    await waitFor(() => {
      expect(screen.getByTestId("export-status")).toHaveTextContent("success");
    });
    const publish = screen.getByRole("button", { name: "Publish export" });
    await user.click(publish);
    const cancelExport = await screen.findByRole("button", { name: "Cancel export" });
    cancelExport.focus();
    await user.keyboard(" ");
    await waitFor(() => {
      expect(screen.getByTestId("export-status")).toHaveTextContent("cancelled");
      expect(publish).toHaveFocus();
    });

    resolveSync({
      run: {
        id: 7,
        status: "cancelled",
        cancel_requested: true,
        accepted_captures: 0,
        skipped_duplicates: 0,
        successful_attempts: 0,
        failed_attempts: 0,
        error_class: "cancelled",
        error_message: "cancelled",
        sources: [],
      },
      session_identities: [],
    });
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
      error_class: "cancelled",
      error_message: "cancelled",
    });
  });

  it("exposes semantic groups, live status, busy panels, and tag remove names", async () => {
    const user = userEvent.setup();
    const bridge = createBridge();
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.click(screen.getByRole("button", { name: "Load sessions" }));
    await waitFor(() => {
      expect(screen.getByTestId("session-list")).toBeInTheDocument();
    });
    await user.click(screen.getByRole("button", { name: /A11y session/ }));
    await waitFor(() => {
      expect(screen.getByTestId("session-detail-panel")).toBeInTheDocument();
    });

    expect(screen.getByRole("group", { name: "Session labels" })).toBeInTheDocument();
    expect(
      screen.getByRole("group", { name: "Toggle session labels" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Session tags" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove tag alpha" })).toBeInTheDocument();
    expect(screen.getByTestId("session-explorer")).toHaveAttribute("aria-busy", "false");
    expect(screen.getByTestId("status").closest("[aria-live]")).toHaveAttribute(
      "aria-live",
      "polite",
    );

    const detail = screen.getByTestId("session-detail-panel");
    const train = within(detail).getByRole("button", { name: "train" });
    train.focus();
    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(within(detail).getByRole("button", { name: "train" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
    });

    const removeTag = within(detail).getByRole("button", { name: "Remove tag alpha" });
    removeTag.focus();
    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(
        within(detail).queryByRole("button", { name: "Remove tag alpha" }),
      ).not.toBeInTheDocument();
    });
  });

  it("keeps labeled controls available when text is scaled to 200%", () => {
    const previous = document.documentElement.style.fontSize;
    document.documentElement.style.fontSize = "200%";
    try {
      render(<App bridge={createBridge()} />);
      expect(screen.getByRole("textbox", { name: "Distill home" })).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "Run Fixture journey" }),
      ).toBeInTheDocument();
      expect(screen.getByRole("combobox", { name: "Workflow lane" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Publish export" })).toBeInTheDocument();
    } finally {
      document.documentElement.style.fontSize = previous;
    }
  });
});
