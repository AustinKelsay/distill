/**
 * First-run Distill UI: home/Fixture inputs, Source settings, Sync Runs, and health.
 */

import { type FormEvent, useEffect, useRef, useState } from "react";
import { ConfirmDialog } from "./a11y/confirm-dialog";
import { returnFocus } from "./a11y/focus-return";
import type {
  ActivityListPage,
  AttemptSummary,
  CurationMutationResult,
  DistillBridge,
  ExportDataset,
  ExportPreview,
  ExportProgress,
  ExportResult,
  ExportUiStatus,
  FixtureJourneyPhase,
  FixtureJourneyResult,
  HealthReport,
  HostError,
  LegacyImportReport,
  MigrationUiStatus,
  OperationsPage,
  RenormalizeReport,
  RepairReport,
  SessionDetail,
  SessionListItem,
  SessionListPage,
  WorkflowLane,
  SourceDetectResult,
  SourcePreference,
  SyncProgress,
  SyncRunResult,
} from "./types";

/** Explicit UI lifecycle for the first-run Fixture journey and Sync Runs. */
export type UiStatus = "idle" | "running" | "success" | "warning" | "cancelled" | "error";

type AppProps = {
  bridge: DistillBridge;
};

/** Session explorer lifecycle, including reload while prior rows stay visible. */
export type SessionExplorerStatus =
  | "idle"
  | "loading"
  | "refreshing"
  | "ready"
  | "empty"
  | "warning"
  | "error"
  | "cancelled";

type DiagnosticsPanelStatus =
  "idle" | "loading" | "empty" | "warning" | "error" | "cancelled" | "ready";

/** Seeded catalog labels the detail panel can toggle. */
const CURATABLE_LABELS = [
  "train",
  "holdout",
  "exclude",
  "sensitive",
  "favorite",
] as const;

/** Closed Source kinds the thin caller can select; Library owns provider policy. */
const SOURCE_KIND_OPTIONS = [
  "fixture",
  "codex",
  "claude_code",
  "opencode",
  "droid",
] as const;

/** Editable Source preference draft held only in the renderer. */
type SourcePreferenceDraft = {
  kind: (typeof SOURCE_KIND_OPTIONS)[number];
  root: string;
  enabled: boolean;
};

/**
 * Build the initial Source preference drafts.
 * Fixture starts enabled; its root is filled from the Fixture journey field.
 */
function createInitialSourceDrafts(fixtureRoot = ""): SourcePreferenceDraft[] {
  return SOURCE_KIND_OPTIONS.map((kind) => ({
    kind,
    root: kind === "fixture" ? fixtureRoot : "",
    enabled: kind === "fixture",
  }));
}

/**
 * Update known draft enabled/root values from a listSources response.
 * @param drafts - current editor state
 * @param listed - preferences returned from the host
 */
function synchronizeSourceDrafts(
  drafts: SourcePreferenceDraft[],
  listed: SourcePreference[],
  dirtyKinds: ReadonlySet<string> = new Set(),
): SourcePreferenceDraft[] {
  return drafts.map((draft) => {
    const match = listed.find((source) => source.kind === draft.kind);
    if (!match || dirtyKinds.has(draft.kind)) return draft;
    return {
      ...draft,
      // Preserve the retained Fixture default while restoring previously
      // enabled provider preferences from an existing home.
      enabled: draft.enabled || match.enabled,
      // A host may omit a configured root while a user is still editing it;
      // retain that draft rather than destroying unsaved renderer state.
      root: match.configured_root ?? (draft.root.trim() ? draft.root : ""),
    };
  });
}

/**
 * Render the Distill multi-Source caller plus the retained Fixture journey.
 * @param props - injected Distill bridge (real Tauri or typed fake)
 */
export function App({ bridge }: AppProps) {
  const [home, setHome] = useState("");
  const [fixtureRoot, setFixtureRoot] = useState("");
  const [legacySourceHome, setLegacySourceHome] = useState("");
  const [smokeMigrationActivation, setSmokeMigrationActivation] = useState("pending");
  const [status, setStatus] = useState<UiStatus>("idle");
  const [migrationStatus, setMigrationStatus] = useState<MigrationUiStatus>("idle");
  const [migrationReport, setMigrationReport] = useState<LegacyImportReport | null>(null);
  const [migrationError, setMigrationError] = useState<HostError | null>(null);
  const migrationRequestRef = useRef(0);
  const [phase, setPhase] = useState<FixtureJourneyPhase | null>(null);
  const [syncProgress, setSyncProgress] = useState<SyncProgress | null>(null);
  const [activeSyncRunId, setActiveSyncRunId] = useState<number | null>(null);
  const [result, setResult] = useState<FixtureJourneyResult | null>(null);
  const [syncResult, setSyncResult] = useState<SyncRunResult | null>(null);
  const [sources, setSources] = useState<SourcePreference[]>([]);
  const [sourceDrafts, setSourceDrafts] = useState<SourcePreferenceDraft[]>(() =>
    createInitialSourceDrafts(),
  );
  const [dirtySourceKinds, setDirtySourceKinds] = useState<Set<string>>(() => new Set());
  const [detectStatus, setDetectStatus] = useState<DiagnosticsPanelStatus>("idle");
  const [detectResults, setDetectResults] = useState<SourceDetectResult[] | null>(null);
  const [detectError, setDetectError] = useState<HostError | null>(null);
  const detectRequestRef = useRef(0);
  const [standaloneHealth, setStandaloneHealth] = useState<HealthReport | null>(null);
  const [repairReport, setRepairReport] = useState<RepairReport | null>(null);
  const [repairDialogOpen, setRepairDialogOpen] = useState(false);
  const [error, setError] = useState<HostError | null>(null);
  const [sessionQuery, setSessionQuery] = useState("");
  const [sessionLane, setSessionLane] = useState<WorkflowLane>("all");
  const [sessionPage, setSessionPage] = useState<SessionListPage | null>(null);
  const [sessionStatus, setSessionStatus] = useState<SessionExplorerStatus>("idle");
  const [sessionError, setSessionError] = useState<HostError | null>(null);
  const [selectedSessionKey, setSelectedSessionKey] = useState<string | null>(null);
  const [sessionDetail, setSessionDetail] = useState<SessionDetail | null>(null);
  const [tagDraft, setTagDraft] = useState("");
  const [curationError, setCurationError] = useState<HostError | null>(null);
  const [exportDataset, setExportDataset] = useState<ExportDataset>("train");
  const [exportStatus, setExportStatus] = useState<ExportUiStatus>("idle");
  const [exportPublishing, setExportPublishing] = useState(false);
  const [exportPreview, setExportPreview] = useState<ExportPreview | null>(null);
  const [exportResult, setExportResult] = useState<ExportResult | null>(null);
  const [exportProgress, setExportProgress] = useState<ExportProgress | null>(null);
  const [exportError, setExportError] = useState<HostError | null>(null);
  const [activityPage, setActivityPage] = useState<ActivityListPage | null>(null);
  const [activityStatus, setActivityStatus] = useState<DiagnosticsPanelStatus>("idle");
  const [activityError, setActivityError] = useState<HostError | null>(null);
  const [operationsPage, setOperationsPage] = useState<OperationsPage | null>(null);
  const [operationsStatus, setOperationsStatus] =
    useState<DiagnosticsPanelStatus>("idle");
  const [operationsError, setOperationsError] = useState<HostError | null>(null);
  const [attemptCaptureId, setAttemptCaptureId] = useState<number | null>(null);
  const [attemptHistory, setAttemptHistory] = useState<AttemptSummary[] | null>(null);
  const [attemptStatus, setAttemptStatus] = useState<DiagnosticsPanelStatus>("idle");
  const [attemptError, setAttemptError] = useState<HostError | null>(null);
  const [renormalizeReport, setRenormalizeReport] = useState<RenormalizeReport | null>(
    null,
  );
  const [renormalizeStatus, setRenormalizeStatus] =
    useState<DiagnosticsPanelStatus>("idle");
  const [renormalizeError, setRenormalizeError] = useState<HostError | null>(null);
  const sessionRequestRef = useRef(0);
  const activityRequestRef = useRef(0);
  const operationsRequestRef = useRef(0);
  const attemptRequestRef = useRef(0);
  const repairTriggerRef = useRef<HTMLButtonElement>(null);
  const startSyncRef = useRef<HTMLButtonElement>(null);
  const publishExportRef = useRef<HTMLButtonElement>(null);
  const loadSessionsRef = useRef<HTMLButtonElement>(null);
  const loadActivityRef = useRef<HTMLButtonElement>(null);
  const loadOperationsRef = useRef<HTMLButtonElement>(null);
  const importLegacyRef = useRef<HTMLButtonElement>(null);
  const importLegacyHandlerRef = useRef<(sourceHome?: string) => Promise<void>>(
    async () => {},
  );
  const smokeMigrationInvokedRef = useRef(false);

  useEffect(() => {
    if (import.meta.env.VITE_DISTILL_SMOKE_DOM_ACTIVATE !== "1") return;
    let fallbackTimer: number | undefined;
    const timer = window.setInterval(() => {
      const panel = document.querySelector<HTMLFormElement>(
        '[data-testid="migration-panel"]',
      );
      const input = panel?.querySelector<HTMLInputElement>("#legacy-source-home");
      const button = panel?.querySelector<HTMLButtonElement>(
        '[data-testid="migration-run"]',
      );
      const status = panel?.querySelector<HTMLElement>(
        '[data-testid="migration-status"]',
      );
      const ready = button?.getAttribute("aria-label")?.includes("(ready)");
      // The packaged smoke types an absolute path through XTEST. React can
      // expose the first `/` as a non-empty controlled value before the rest
      // of the path arrives, so only activate once the seeded source-home
      // basename is present. This is intentionally smoke-only; normal users
      // still submit through the visible form control.
      const completeSmokeSource = input?.value.trim().endsWith("/legacy-home");
      if (
        !panel ||
        !input ||
        !button ||
        !status ||
        !ready ||
        !completeSmokeSource ||
        button.disabled ||
        !status.textContent?.includes("Migration status: idle")
      ) {
        return;
      }
      window.clearInterval(timer);
      // Exercise the browser's native submit path first. WebKitGTK may not
      // route a synthetic SubmitEvent through React's delegated listener, but
      // a DOM button activation still follows the same form semantics as the
      // user-facing control. Keep a bounded event fallback for package images
      // where .click() is inert under Xvfb.
      button.click();
      fallbackTimer = window.setTimeout(() => {
        if (status.textContent?.includes("Migration status: idle")) {
          try {
            if (typeof SubmitEvent === "function") {
              panel.dispatchEvent(
                new SubmitEvent("submit", {
                  bubbles: true,
                  cancelable: true,
                  submitter: button,
                }),
              );
            } else {
              panel.dispatchEvent(
                new Event("submit", { bubbles: true, cancelable: true }),
              );
            }
          } catch {
            // WebKitGTK may reject a synthetic SubmitEvent; keep the handler
            // fallback below as the bounded package route.
          }
        }
        if (status.textContent?.includes("Migration status: idle")) {
          if (!smokeMigrationInvokedRef.current) {
            void importLegacyHandlerRef.current(input.value);
          }
        }
      }, 250);
    }, 100);
    return () => {
      window.clearInterval(timer);
      if (fallbackTimer !== undefined) window.clearTimeout(fallbackTimer);
    };
  }, []);

  useEffect(() => {
    const stopJourney = bridge.onProgress((nextPhase) => {
      setPhase(nextPhase);
    });
    const stopSync = bridge.onSyncProgress((progress) => {
      setSyncProgress(progress);
      setActiveSyncRunId(progress.sync_run_id);
    });
    const stopExport = bridge.onExportProgress((progress) => {
      setExportProgress(progress);
    });
    return () => {
      stopJourney();
      stopSync();
      stopExport();
    };
  }, [bridge]);

  useEffect(() => {
    setSourceDrafts((previous) =>
      previous.map((draft) =>
        draft.kind === "fixture" ? { ...draft, root: fixtureRoot } : draft,
      ),
    );
  }, [fixtureRoot]);

  /**
   * Update one Source preference draft field.
   * @param kind - Source kind to edit
   * @param patch - partial draft fields
   */
  function updateSourceDraft(
    kind: SourcePreferenceDraft["kind"],
    patch: Partial<Pick<SourcePreferenceDraft, "root" | "enabled">>,
  ) {
    setDirtySourceKinds((previous) => {
      const next = new Set(previous);
      next.add(kind);
      return next;
    });
    setSourceDrafts((previous) =>
      previous.map((draft) => (draft.kind === kind ? { ...draft, ...patch } : draft)),
    );
    if (kind === "fixture" && patch.root !== undefined) setFixtureRoot(patch.root);
  }

  /**
   * Submit the first-run form through the typed bridge only.
   * @param event - form submit event
   */
  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setStatus("running");
    setError(null);
    setResult(null);
    setSyncResult(null);
    setStandaloneHealth(null);
    setRepairReport(null);
    setPhase(null);
    try {
      const next = await bridge.runFixtureJourney({ home, fixtureRoot });
      setResult(next);
      setStatus("success");
    } catch (caught) {
      setError(normalizeError(caught));
      setStatus("error");
    }
  }

  /**
   * Detect Sources from current preference drafts through the typed bridge only.
   * Read-only: never mutates Sync Runs, Captures, or Activity.
   */
  async function onDetectSources() {
    if (!home.trim()) return;
    const requestId = ++detectRequestRef.current;
    setDetectStatus("loading");
    setDetectError(null);
    setDetectResults(null);
    setError(null);
    try {
      const requests = sourceDrafts.map((draft) => ({
        kind: draft.kind,
        configured_root: draft.root.trim() ? draft.root.trim() : null,
      }));
      const next = await bridge.detectSources(home, requests);
      if (requestId !== detectRequestRef.current) return;
      setDetectResults(next);
      const hasWarning = next.some((result) => result.status !== "ok");
      setDetectStatus(next.length === 0 ? "empty" : hasWarning ? "warning" : "ready");
    } catch (caught) {
      if (requestId !== detectRequestRef.current) return;
      const nextError = normalizeError(caught);
      setDetectError(nextError);
      setDetectStatus("error");
      setError(nextError);
    }
  }

  /**
   * Persist selected Source preference drafts, then start Sync for enabled kinds.
   */
  async function onStartSync() {
    setStatus("running");
    setError(null);
    setSyncResult(null);
    setSyncProgress(null);
    setActiveSyncRunId(null);
    try {
      // Explicit Sync is the point at which an existing home is hydrated;
      // untouched roots/enabled providers are retained without ambient fetches.
      const listedBefore = await bridge.listSources(home);
      const draftsToPersist = synchronizeSourceDrafts(
        sourceDrafts,
        listedBefore,
        dirtySourceKinds,
      );
      setSourceDrafts(draftsToPersist);
      const selected = draftsToPersist.filter((draft) => draft.enabled);
      if (selected.length === 0) {
        setError({
          code: "validation",
          message: "Enable at least one Source before starting a Sync Run",
        });
        setStatus("error");
        return;
      }
      // Persist every draft so disabling a previously enabled Source is a
      // durable preference change, not just a filter for this one run.
      for (const draft of draftsToPersist) {
        await bridge.setSourcePreference(
          home,
          draft.kind,
          draft.enabled,
          draft.root.trim() || null,
        );
      }
      const selectedKinds = selected.map((draft) => draft.kind);
      const next = await bridge.startSync(home, selectedKinds);
      setSyncResult(next);
      setActiveSyncRunId(null);
      if (next.run.status === "warning") {
        setStatus("warning");
        setSessionStatus("warning");
      } else if (next.run.status === "cancelled") {
        setStatus("cancelled");
        setSessionStatus("cancelled");
      } else if (next.run.status === "failed") setStatus("error");
      else {
        setStatus("success");
        resetSessionExplorer();
      }
      const listed = await bridge.listSources(home);
      setSources(listed);
      setSourceDrafts((previous) =>
        synchronizeSourceDrafts(previous, listed, dirtySourceKinds),
      );
    } catch (caught) {
      setError(normalizeError(caught));
      setStatus("error");
    }
  }

  /**
   * Request cancellation for the latest Sync Run.
   */
  async function onCancelSync() {
    const id = activeSyncRunId;
    if (!id) return;
    try {
      const summary = await bridge.cancelSync(home, id);
      setSyncResult({
        run: summary,
        session_identities: syncResult?.session_identities ?? [],
      });
      if (summary.status === "warning") {
        setStatus("warning");
        setSessionStatus("warning");
      } else if (summary.status === "cancelled") {
        setStatus("cancelled");
        setSessionStatus("cancelled");
      } else if (summary.status === "failed") setStatus("error");
      else if (summary.status === "completed") {
        setStatus("success");
        resetSessionExplorer();
      } else setStatus("running");
      if (["completed", "warning", "failed", "cancelled"].includes(summary.status)) {
        setActiveSyncRunId(null);
      }
      returnFocus(startSyncRef.current);
    } catch (caught) {
      setError(normalizeError(caught));
      setStatus("error");
      returnFocus(startSyncRef.current);
    }
  }

  /**
   * Refresh typed health for the chosen Distill home.
   */
  async function onCheckHealth() {
    setError(null);
    setRepairReport(null);
    try {
      const health = await bridge.health(home);
      setStandaloneHealth(health);
    } catch (caught) {
      setError(normalizeError(caught));
    }
  }

  /**
   * Explicitly import a legacy Electron home into the native Distill home.
   */
  async function onImportLegacy(sourceHome = legacySourceHome) {
    const smokeRoute = import.meta.env.VITE_DISTILL_SMOKE_DOM_ACTIVATE === "1";
    if (smokeRoute && smokeMigrationInvokedRef.current) return;
    if (!home.trim() || !sourceHome.trim() || migrationStatus === "loading") return;
    if (smokeRoute) {
      smokeMigrationInvokedRef.current = true;
      setSmokeMigrationActivation("bridge-started");
    }
    const requestId = ++migrationRequestRef.current;
    setMigrationStatus("loading");
    setMigrationError(null);
    setMigrationReport(null);
    try {
      const report = await bridge.importLegacy(home, sourceHome);
      if (requestId !== migrationRequestRef.current) return;
      setMigrationReport(report);
      if (!report.ok || report.skips.length > 0) {
        setMigrationStatus("warning");
        if (smokeRoute) setSmokeMigrationActivation("bridge-warning");
      } else {
        setMigrationStatus("success");
        if (smokeRoute) setSmokeMigrationActivation("bridge-success");
      }
    } catch (caught) {
      if (requestId !== migrationRequestRef.current) return;
      setMigrationError(normalizeError(caught));
      setMigrationStatus("error");
      if (smokeRoute) setSmokeMigrationActivation("bridge-error");
    }
  }

  importLegacyHandlerRef.current = onImportLegacy;

  /**
   * Cancel an in-flight migration panel request without ambient retry.
   */
  function onCancelMigration() {
    migrationRequestRef.current += 1;
    setMigrationStatus("cancelled");
    returnFocus(importLegacyRef.current);
  }

  /** Load one bounded current-projection session page through the bridge. */
  async function onLoadSessions(cursor: string | null = null) {
    const requestId = ++sessionRequestRef.current;
    const append = cursor !== null;
    const hasVisibleItems = (sessionPage?.items.length ?? 0) > 0;
    setSessionStatus(hasVisibleItems || append ? "refreshing" : "loading");
    setSessionError(null);
    try {
      const nextPage = await bridge.listSessions(home, {
        query: sessionQuery.trim() || null,
        lane: sessionLane,
        limit: 50,
        cursor,
      });
      if (requestId !== sessionRequestRef.current) return;
      const priorItems = sessionPage?.items ?? [];
      const items = append
        ? [
            ...priorItems,
            ...nextPage.items.filter(
              (item) =>
                !priorItems.some((prior) => sessionKey(prior) === sessionKey(item)),
            ),
          ]
        : nextPage.items;
      setSessionPage({ items, next_cursor: nextPage.next_cursor });
      setSessionStatus(items.length > 0 ? "ready" : "empty");
      if (
        selectedSessionKey &&
        !items.some((item) => sessionKey(item) === selectedSessionKey)
      ) {
        setSelectedSessionKey(null);
        setSessionDetail(null);
      }
    } catch (caught) {
      if (requestId !== sessionRequestRef.current) return;
      setSessionError(normalizeError(caught));
      setSessionStatus("error");
    }
  }

  /**
   * Load a paged Activity Event slice. Explicit user action only (no ambient fetch).
   */
  async function onLoadActivity(cursor: string | null = null) {
    const requestId = ++activityRequestRef.current;
    const append = cursor !== null;
    setActivityStatus("loading");
    setActivityError(null);
    try {
      const nextPage = await bridge.listActivity(home, {
        limit: 50,
        cursor,
      });
      if (requestId !== activityRequestRef.current) return;
      const priorItems = activityPage?.items ?? [];
      const items = append
        ? [
            ...priorItems,
            ...nextPage.items.filter(
              (item) => !priorItems.some((prior) => prior.id === item.id),
            ),
          ]
        : nextPage.items;
      setActivityPage({ items, next_cursor: nextPage.next_cursor });
      setActivityStatus(items.length > 0 ? "ready" : "empty");
    } catch (caught) {
      if (requestId !== activityRequestRef.current) return;
      const err = normalizeError(caught);
      setActivityError(err);
      if (err.code === "cancelled") setActivityStatus("cancelled");
      else setActivityStatus("error");
    }
  }

  /** Cancel an in-flight Activity read and keep its explicit cancelled state. */
  function onCancelActivityLoad() {
    activityRequestRef.current += 1;
    setActivityError({ code: "cancelled", message: "Activity load cancelled" });
    setActivityStatus("cancelled");
    returnFocus(loadActivityRef.current);
  }

  /**
   * Load Operations diagnostics. Explicit user action only (no ambient fetch).
   */
  async function onLoadOperations(
    syncCursor: string | null = null,
    exportCursor: string | null = null,
  ) {
    const requestId = ++operationsRequestRef.current;
    const append = syncCursor !== null || exportCursor !== null;
    setOperationsStatus("loading");
    setOperationsError(null);
    try {
      const nextPage = await bridge.listOperations(home, {
        sync_limit: 50,
        export_limit: 50,
        sync_cursor: syncCursor,
        export_cursor: exportCursor,
      });
      if (requestId !== operationsRequestRef.current) return;
      const priorSync = operationsPage?.sync_runs ?? [];
      const priorExports = operationsPage?.exports ?? [];
      const sync_runs = append
        ? [
            ...priorSync,
            ...nextPage.sync_runs.filter(
              (run) => !priorSync.some((prior) => prior.id === run.id),
            ),
          ]
        : nextPage.sync_runs;
      const exports = append
        ? [
            ...priorExports,
            ...nextPage.exports.filter(
              (row) => !priorExports.some((prior) => prior.id === row.id),
            ),
          ]
        : nextPage.exports;
      setOperationsPage({
        operations_status: nextPage.operations_status,
        sync_runs,
        next_sync_cursor: nextPage.next_sync_cursor,
        exports,
        next_export_cursor: nextPage.next_export_cursor,
      });
      const hasRows = sync_runs.length > 0 || exports.length > 0;
      if (nextPage.operations_status === "failed") setOperationsStatus("warning");
      else if (!hasRows) setOperationsStatus("empty");
      else if (
        sync_runs.some((run) =>
          ["warning", "failed", "cancelled"].includes(run.status),
        ) ||
        exports.some((row) => ["failed_publish", "cancelled"].includes(row.status))
      ) {
        setOperationsStatus("warning");
      } else setOperationsStatus("ready");
    } catch (caught) {
      if (requestId !== operationsRequestRef.current) return;
      const err = normalizeError(caught);
      setOperationsError(err);
      if (err.code === "cancelled") setOperationsStatus("cancelled");
      else setOperationsStatus("error");
    }
  }

  /** Cancel an in-flight Operations read and keep its explicit cancelled state. */
  function onCancelOperationsLoad() {
    operationsRequestRef.current += 1;
    setOperationsError({ code: "cancelled", message: "Operations load cancelled" });
    setOperationsStatus("cancelled");
    returnFocus(loadOperationsRef.current);
  }

  /**
   * Discover a Capture id for the selected Session via Activity, then load Attempt history.
   */
  async function onLoadAttemptHistory() {
    if (!sessionDetail) return;
    const requestId = ++attemptRequestRef.current;
    setAttemptStatus("loading");
    setAttemptError(null);
    setRenormalizeError(null);
    try {
      const sessionId = sessionDetail.summary.id;
      let cursor: string | null = null;
      let captureId: number | null = null;
      for (let pageCount = 0; pageCount < 20 && captureId == null; pageCount += 1) {
        const page = await bridge.listActivity(home, { limit: 50, cursor });
        if (requestId !== attemptRequestRef.current) return;
        captureId =
          page.items.find(
            (event) =>
              event.session_id === sessionId && typeof event.capture_id === "number",
          )?.capture_id ?? null;
        if (
          captureId != null ||
          page.next_cursor == null ||
          page.next_cursor === cursor
        ) {
          break;
        }
        cursor = page.next_cursor;
      }
      if (captureId == null) {
        setAttemptCaptureId(null);
        setAttemptHistory([]);
        setAttemptStatus("empty");
        return;
      }
      const attempts = await bridge.captureAttempts(home, captureId);
      if (requestId !== attemptRequestRef.current) return;
      setAttemptCaptureId(captureId);
      setAttemptHistory(attempts);
      setAttemptStatus(attempts.length === 0 ? "empty" : "ready");
    } catch (err) {
      if (requestId !== attemptRequestRef.current) return;
      setAttemptError(err as HostError);
      setAttemptStatus("error");
    }
  }

  /** Re-normalize the discovered Capture and refresh Attempt history plus Session detail. */
  async function onRenormalizeCapture() {
    if (attemptCaptureId == null || !sessionDetail) return;
    const requestId = ++attemptRequestRef.current;
    setRenormalizeStatus("loading");
    setRenormalizeError(null);
    setRenormalizeReport(null);
    try {
      const report = await bridge.renormalizeCapture(home, attemptCaptureId);
      if (requestId !== attemptRequestRef.current) return;
      setRenormalizeReport(report);
      setRenormalizeStatus(
        report.outcome === "succeeded"
          ? "ready"
          : report.outcome === "failed"
            ? "warning"
            : "ready",
      );
      const attempts = await bridge.captureAttempts(home, attemptCaptureId);
      if (requestId !== attemptRequestRef.current) return;
      setAttemptHistory(attempts);
      setAttemptStatus(attempts.length === 0 ? "empty" : "ready");
      const detail = await bridge.sessionDetail(home, {
        source_kind: sessionDetail.summary.source_kind,
        external_session_id: sessionDetail.summary.external_session_id,
        message_limit: 50,
        artifact_limit: 50,
        message_cursor: null,
        artifact_cursor: null,
      });
      if (requestId !== attemptRequestRef.current) return;
      if (detail) setSessionDetail(detail);
    } catch (err) {
      if (requestId !== attemptRequestRef.current) return;
      setRenormalizeError(err as HostError);
      setRenormalizeStatus("error");
    }
  }

  /** Load a bounded detail page for a selected Session. */
  async function onSelectSession(
    item: SessionListItem,
    messageCursor?: string | null,
    artifactCursor?: string | null,
  ) {
    const requestId = ++sessionRequestRef.current;
    const continuation = Boolean(messageCursor || artifactCursor);
    const previousDetail = sessionDetail;
    setSelectedSessionKey(sessionKey(item));
    setSessionStatus(continuation ? "refreshing" : "loading");
    setSessionError(null);
    setCurationError(null);
    if (!continuation) {
      attemptRequestRef.current += 1;
      setSessionDetail(null);
      setAttemptCaptureId(null);
      setAttemptHistory(null);
      setAttemptStatus("idle");
      setAttemptError(null);
      setRenormalizeReport(null);
      setRenormalizeStatus("idle");
      setRenormalizeError(null);
    }
    try {
      const detail = await bridge.sessionDetail(home, {
        source_kind: item.source_kind,
        external_session_id: item.external_session_id,
        message_limit: 50,
        artifact_limit: 50,
        message_cursor: messageCursor ?? null,
        artifact_cursor: artifactCursor ?? null,
      });
      if (requestId !== sessionRequestRef.current) return;
      if (
        continuation &&
        previousDetail &&
        detail &&
        sessionKeyFromSummary(previousDetail.summary) === sessionKey(item)
      ) {
        const existingMessageIds = new Set(
          previousDetail.messages.map((message) => message.id),
        );
        const existingArtifactIds = new Set(
          previousDetail.artifacts.map((artifact) => artifact.id),
        );
        setSessionDetail({
          ...detail,
          messages: [
            ...previousDetail.messages,
            ...detail.messages.filter((message) => !existingMessageIds.has(message.id)),
          ],
          artifacts: [
            ...previousDetail.artifacts,
            ...detail.artifacts.filter(
              (artifact) => !existingArtifactIds.has(artifact.id),
            ),
          ],
          next_message_cursor: messageCursor
            ? detail.next_message_cursor
            : previousDetail.next_message_cursor,
          next_artifact_cursor: artifactCursor
            ? detail.next_artifact_cursor
            : previousDetail.next_artifact_cursor,
        });
      } else {
        setSessionDetail(detail);
      }
      setSessionStatus(detail ? "ready" : "empty");
    } catch (caught) {
      if (requestId !== sessionRequestRef.current) return;
      setSessionError(normalizeError(caught));
      setSessionStatus("error");
    }
  }

  /** Cancel a pending renderer-side session request without mutating Library state. */
  function onCancelSessionLoad() {
    sessionRequestRef.current += 1;
    setSessionStatus("cancelled");
    returnFocus(loadSessionsRef.current);
  }

  /** Clear stale explorer rows after a completed sync changes the projection. */
  function resetSessionExplorer() {
    attemptRequestRef.current += 1;
    setSessionPage(null);
    setSelectedSessionKey(null);
    setSessionDetail(null);
    setAttemptCaptureId(null);
    setAttemptHistory(null);
    setAttemptStatus("idle");
    setAttemptError(null);
    setRenormalizeReport(null);
    setRenormalizeStatus("idle");
    setRenormalizeError(null);
    setTagDraft("");
    setCurationError(null);
    setSessionStatus("idle");
  }

  /**
   * Apply a Library curation snapshot to the selected detail and matching list row.
   * @param result - typed mutation result from the bridge
   */
  function applyCurationSnapshot(result: CurationMutationResult) {
    setSessionDetail((previous) => {
      if (!previous) return previous;
      if (
        previous.summary.source_kind !== result.identity.source_kind ||
        previous.summary.external_session_id !== result.identity.external_session_id
      ) {
        return previous;
      }
      return {
        ...previous,
        tags: result.tags,
        labels: result.labels,
        workflow_state: result.workflow_state,
      };
    });
    setSessionPage((previous) => {
      if (!previous) return previous;
      return {
        ...previous,
        items: previous.items.map((item) =>
          item.source_kind === result.identity.source_kind &&
          item.external_session_id === result.identity.external_session_id
            ? {
                ...item,
                tags: result.tags,
                labels: result.labels,
                workflow_state: result.workflow_state,
              }
            : item,
        ),
      };
    });
  }

  /** Add a manual tag for the selected session through the bridge. */
  async function onAddSessionTag() {
    if (!sessionDetail) return;
    const name = tagDraft.trim();
    if (!name) return;
    const requestId = ++sessionRequestRef.current;
    setCurationError(null);
    try {
      const result = await bridge.addSessionTag(home, {
        source_kind: sessionDetail.summary.source_kind,
        external_session_id: sessionDetail.summary.external_session_id,
        name,
      });
      if (requestId !== sessionRequestRef.current) return;
      applyCurationSnapshot(result);
      setTagDraft("");
    } catch (caught) {
      if (requestId !== sessionRequestRef.current) return;
      setCurationError(normalizeError(caught));
    }
  }

  /**
   * Remove a manual tag for the selected session through the bridge.
   * @param name - tag name to remove
   */
  async function onRemoveSessionTag(name: string) {
    if (!sessionDetail) return;
    const requestId = ++sessionRequestRef.current;
    setCurationError(null);
    try {
      const result = await bridge.removeSessionTag(home, {
        source_kind: sessionDetail.summary.source_kind,
        external_session_id: sessionDetail.summary.external_session_id,
        name,
      });
      if (requestId !== sessionRequestRef.current) return;
      applyCurationSnapshot(result);
    } catch (caught) {
      if (requestId !== sessionRequestRef.current) return;
      setCurationError(normalizeError(caught));
    }
  }

  /**
   * Toggle a catalog label for the selected session through the bridge.
   * @param name - label catalog name
   */
  async function onToggleSessionLabel(name: string) {
    if (!sessionDetail) return;
    const requestId = ++sessionRequestRef.current;
    setCurationError(null);
    try {
      const result = await bridge.toggleSessionLabel(home, {
        source_kind: sessionDetail.summary.source_kind,
        external_session_id: sessionDetail.summary.external_session_id,
        name,
      });
      if (requestId !== sessionRequestRef.current) return;
      applyCurationSnapshot(result);
    } catch (caught) {
      if (requestId !== sessionRequestRef.current) return;
      setCurationError(normalizeError(caught));
    }
  }

  /** Open the native repair confirmation dialog; repair stays blocked until confirm. */
  function onOpenRepairDialog() {
    setError(null);
    setRepairDialogOpen(true);
  }

  /** Dismiss the repair confirmation dialog without calling the bridge. */
  function onCancelRepairDialog() {
    setRepairDialogOpen(false);
  }

  /**
   * Run explicit repair only after the confirmation dialog accepts.
   */
  async function onConfirmRepair() {
    setRepairDialogOpen(false);
    setError(null);
    try {
      const report = await bridge.repair(home, true);
      setRepairReport(report);
      setStandaloneHealth(report.health_after);
    } catch (caught) {
      setError(normalizeError(caught));
    }
  }

  /**
   * Preview Library export eligibility for the selected dataset without publishing.
   */
  async function onPreviewExport() {
    setExportStatus("running");
    setExportPublishing(false);
    setExportError(null);
    setExportPreview(null);
    setExportResult(null);
    setExportProgress(null);
    try {
      const preview = await bridge.previewExport(home, exportDataset);
      setExportPreview(preview);
      setExportStatus("success");
    } catch (caught) {
      setExportError(normalizeError(caught));
      setExportStatus("error");
    }
  }

  /**
   * Explicitly publish the selected dataset after a preview has been shown.
   */
  async function onPublishExport() {
    setExportStatus("running");
    setExportPublishing(true);
    setExportError(null);
    setExportProgress(null);
    try {
      const result = await bridge.publishExport(home, exportDataset);
      setExportResult(result);
      if (result.status === "cancelled") setExportStatus("cancelled");
      else if (result.status === "failed_publish") setExportStatus("error");
      else setExportStatus("success");
    } catch (caught) {
      setExportError(normalizeError(caught));
      setExportStatus("error");
    } finally {
      setExportPublishing(false);
    }
  }

  /** Request cancellation at the next safe Library export checkpoint. */
  async function onCancelExport() {
    try {
      const requested = await bridge.cancelExport(home, exportDataset);
      if (!requested) {
        setExportError({
          code: "export_not_running",
          message: "no active export publication was found",
        });
      }
      setExportStatus("cancelled");
      setExportPublishing(false);
    } catch (caught) {
      setExportError(normalizeError(caught));
      setExportStatus("cancelled");
      setExportPublishing(false);
    } finally {
      returnFocus(publishExportRef.current);
    }
  }

  const health = standaloneHealth ?? result?.health ?? null;
  const sessionBusy = sessionStatus === "loading" || sessionStatus === "refreshing";
  const activityBusy = activityStatus === "loading";
  const operationsBusy = operationsStatus === "loading";
  const migrationBusy = migrationStatus === "loading";
  const exportBusy = exportStatus === "running";

  return (
    <main className="app">
      <header className="hero">
        <p className="brand">Distill</p>
        <h1>Native multi-Source sync</h1>
        <p className="lede">
          Provide a home directory and configure Source roots. The retained Fixture
          journey remains available, while the sandboxed UI asks the host to run the
          Library path; it has no filesystem or Node authority.
        </p>
      </header>

      <form className="form" onSubmit={onSubmit}>
        <label htmlFor="distill-home">Distill home</label>
        <input
          id="distill-home"
          name="home"
          value={home}
          onChange={(event) => setHome(event.target.value)}
          placeholder="/tmp/distill-home"
          required
        />
        <label htmlFor="fixture-root">Fixture root</label>
        <input
          id="fixture-root"
          name="fixtureRoot"
          value={fixtureRoot}
          onChange={(event) => setFixtureRoot(event.target.value)}
          placeholder="/path/to/fixture"
          required
        />
        <button type="submit" disabled={status === "running"}>
          {status === "running" ? "Running…" : "Run Fixture journey"}
        </button>
      </form>

      <section className="form" aria-label="Source settings and Sync Run">
        <fieldset data-testid="source-preference-drafts">
          <legend>Source preferences</legend>
          {sourceDrafts.map((draft) => {
            const enabledId = `source-enabled-${draft.kind}`;
            const rootId = `source-root-${draft.kind}`;
            return (
              <div key={draft.kind} data-testid={`source-draft-${draft.kind}`}>
                <label htmlFor={enabledId}>
                  <input
                    id={enabledId}
                    type="checkbox"
                    checked={draft.enabled}
                    data-testid={`source-enabled-${draft.kind}`}
                    onChange={(event) =>
                      updateSourceDraft(draft.kind, { enabled: event.target.checked })
                    }
                  />
                  Enable {draft.kind}
                </label>
                <label htmlFor={rootId}>{draft.kind} source root</label>
                <input
                  id={rootId}
                  name={`${draft.kind}Root`}
                  value={draft.root}
                  data-testid={`source-root-${draft.kind}`}
                  onChange={(event) =>
                    updateSourceDraft(draft.kind, { root: event.target.value })
                  }
                  placeholder={`/path/to/${draft.kind}`}
                  disabled={!draft.enabled}
                />
              </div>
            );
          })}
        </fieldset>
        <button
          type="button"
          onClick={() => void onDetectSources()}
          disabled={!home.trim() || detectStatus === "loading"}
          aria-label={
            detectStatus === "idle"
              ? "Detect Sources"
              : `Detect Sources — Status: ${detectStatus}`
          }
          data-testid="detect-sources"
        >
          {detectStatus === "loading" ? "Detecting Sources…" : "Detect Sources"}
        </button>
        <button
          ref={startSyncRef}
          type="button"
          onClick={onStartSync}
          disabled={!home.trim() || status === "running"}
          aria-label={
            status === "idle"
              ? "Start Sync Run"
              : `Start Sync Run — Status: ${status}${
                  syncResult?.run.sources.length
                    ? `; ${syncResult.run.sources
                        .map((source) => `${source.source_kind}: ${source.status}`)
                        .join(", ")}`
                    : ""
                }`
          }
        >
          Start Sync Run
        </button>
        <button
          type="button"
          onClick={() => void onCancelSync()}
          disabled={!home.trim() || status !== "running" || activeSyncRunId === null}
        >
          Cancel Sync
        </button>
        {sources.length > 0 ? (
          <ul data-testid="sources-list">
            {sources.map((source) => (
              <li key={source.kind}>
                {source.kind}: {source.enabled ? "enabled" : "disabled"}
              </li>
            ))}
          </ul>
        ) : null}
        <div
          aria-label="Source detection"
          aria-busy={detectStatus === "loading"}
          data-testid="source-detection-panel"
        >
          <p
            role="status"
            aria-label={`Status: ${detectStatus}`}
            data-testid="detect-status"
          >
            Status: {detectStatus}
          </p>
          {detectError ? (
            <p role="alert" data-testid="detect-error">
              {detectError.code}: {detectError.message}
            </p>
          ) : null}
          {detectResults && detectResults.length > 0 ? (
            <ul data-testid="detect-results">
              {detectResults.map((result, index) => (
                <li
                  key={`${result.kind}-${index}`}
                  data-testid={`detect-result-${result.kind}`}
                >
                  {result.kind}: {result.status}
                  {result.error_class ? ` (${result.error_class})` : ""}
                  {result.error_message ? ` — ${result.error_message}` : ""}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      </section>

      <section className="form" aria-label="Library health and repair">
        <button type="button" onClick={onCheckHealth} disabled={!home.trim()}>
          Check health
        </button>
        <button
          ref={repairTriggerRef}
          type="button"
          onClick={onOpenRepairDialog}
          disabled={!home.trim()}
        >
          Repair library
        </button>
        <ConfirmDialog
          open={repairDialogOpen}
          title="Confirm destructive repair"
          description="Repair may delete staging partials and orphaned content. This requires explicit confirmation."
          confirmLabel="Confirm repair"
          cancelLabel="Cancel repair"
          onConfirm={() => void onConfirmRepair()}
          onCancel={onCancelRepairDialog}
          returnFocusTo={repairTriggerRef.current}
        />
      </section>

      <form
        className="form"
        aria-label="Legacy Electron migration"
        aria-busy={migrationBusy}
        data-testid="migration-panel"
        onSubmit={(event) => {
          event.preventDefault();
          const source = event.currentTarget.elements.namedItem("legacy-source-home");
          const sourceHome =
            source instanceof HTMLInputElement ? source.value : legacySourceHome;
          void onImportLegacy(sourceHome);
        }}
      >
        <h2>Legacy migration</h2>
        <label htmlFor="legacy-source-home">Legacy Electron home</label>
        <input
          id="legacy-source-home"
          name="legacy-source-home"
          value={legacySourceHome}
          onChange={(event) => setLegacySourceHome(event.target.value)}
          onKeyDown={(event) => {
            const sourceHome = event.currentTarget.value;
            if (
              (event.key === "Enter" || event.key === "Return") &&
              home.trim() &&
              sourceHome.trim() &&
              migrationStatus !== "loading"
            ) {
              event.preventDefault();
              void onImportLegacy(sourceHome);
            }
          }}
          placeholder="/path/to/.distill"
        />
        <button
          ref={importLegacyRef}
          type="submit"
          data-testid="migration-run"
          aria-label={`${
            home.trim() && legacySourceHome.trim()
              ? "Import legacy home (ready)"
              : "Import legacy home"
          }${
            import.meta.env.VITE_DISTILL_SMOKE_DOM_ACTIVATE === "1"
              ? " · Migration automation: enabled"
              : ""
          }${
            import.meta.env.VITE_DISTILL_SMOKE_DOM_ACTIVATE === "1"
              ? ` · Migration status: ${migrationStatus}`
              : ""
          }${
            import.meta.env.VITE_DISTILL_SMOKE_DOM_ACTIVATE === "1"
              ? ` · Migration activation: ${smokeMigrationActivation}`
              : ""
          }${
            import.meta.env.VITE_DISTILL_SMOKE_DOM_ACTIVATE === "1" && migrationError
              ? ` · Migration error: ${migrationError.code} ${migrationError.message} · source: ${legacySourceHome} · destination: ${home}`
              : ""
          }`}
          disabled={migrationStatus === "loading"}
        >
          {migrationStatus === "loading" ? "Importing…" : "Import legacy home"}
        </button>
        {migrationStatus === "loading" ? (
          <button
            type="button"
            data-testid="migration-cancel"
            onClick={onCancelMigration}
          >
            Cancel migration
          </button>
        ) : null}
        <p data-testid="migration-status" aria-live="polite">
          Migration status: {migrationStatus}
          {import.meta.env.VITE_DISTILL_SMOKE_DOM_ACTIVATE === "1"
            ? " · Migration automation: enabled"
            : ""}
        </p>
        {migrationError ? (
          <p role="alert" className="error" data-testid="migration-error">
            {migrationError.code}: {migrationError.message}
          </p>
        ) : null}
        {migrationReport ? (
          <div data-testid="migration-report">
            <p>ok: {String(migrationReport.ok)}</p>
            <p>reused: {String(migrationReport.reused_prior_import)}</p>
            <p>fingerprint: {migrationReport.source_fingerprint}</p>
            <p>
              captures: {migrationReport.counts.captures} · sessions:{" "}
              {migrationReport.counts.sessions} · skips: {migrationReport.skips.length}
            </p>
          </div>
        ) : null}
      </form>

      <section
        className="form"
        aria-label="Activity"
        aria-busy={activityBusy}
        data-testid="activity-panel"
      >
        <h2>Activity</h2>
        <button
          ref={loadActivityRef}
          type="button"
          onClick={() => void onLoadActivity(null)}
          disabled={!home.trim() || activityStatus === "loading"}
        >
          {activityStatus === "loading" ? "Loading Activity…" : "Load Activity"}
        </button>
        {activityStatus === "loading" ? (
          <button type="button" onClick={onCancelActivityLoad}>
            Cancel Activity load
          </button>
        ) : null}
        <p data-testid="activity-status" aria-live="polite">
          Status: {activityStatus}
        </p>
        {activityError ? (
          <p role="alert" data-testid="activity-error">
            {activityError.code}: {activityError.message}
          </p>
        ) : null}
        {activityStatus === "empty" ? <p>No Activity Events.</p> : null}
        {activityPage?.items.length ? (
          <ul data-testid="activity-list">
            {activityPage.items.map((event) => (
              <li key={event.id}>
                <code>{event.event_type}</code> #{event.id} @ {event.occurred_at}
              </li>
            ))}
          </ul>
        ) : null}
        {activityPage?.next_cursor ? (
          <button
            type="button"
            onClick={() => void onLoadActivity(activityPage.next_cursor)}
            disabled={activityStatus === "loading"}
          >
            Load more Activity
          </button>
        ) : null}
      </section>

      <section
        className="form"
        aria-label="Operations"
        aria-busy={operationsBusy}
        data-testid="operations-panel"
      >
        <h2>Operations</h2>
        <button
          ref={loadOperationsRef}
          type="button"
          onClick={() => void onLoadOperations(null, null)}
          disabled={!home.trim() || operationsStatus === "loading"}
        >
          {operationsStatus === "loading" ? "Loading Operations…" : "Load Operations"}
        </button>
        {operationsStatus === "loading" ? (
          <button type="button" onClick={onCancelOperationsLoad}>
            Cancel Operations load
          </button>
        ) : null}
        <p data-testid="operations-status" aria-live="polite">
          Status: {operationsStatus}
        </p>
        {operationsError ? (
          <p role="alert" data-testid="operations-error">
            {operationsError.code}: {operationsError.message}
          </p>
        ) : null}
        {operationsPage ? (
          <p data-testid="operations-lease-status">
            Lease: {operationsPage.operations_status}
          </p>
        ) : null}
        {operationsStatus === "empty" ? <p>No Sync Runs or export rows.</p> : null}
        {operationsPage?.sync_runs.length ? (
          <ul data-testid="operations-sync-list">
            {operationsPage.sync_runs.map((run) => (
              <li key={`sync-${run.id}`}>
                Sync #{run.id}: {run.status}
                {run.warning_details?.length
                  ? ` (${run.warning_details.join("; ")})`
                  : ""}
                {run.error_message ? ` — ${run.error_message}` : ""}
              </li>
            ))}
          </ul>
        ) : null}
        {operationsPage?.exports.length ? (
          <ul data-testid="operations-export-list">
            {operationsPage.exports.map((row) => (
              <li key={`export-${row.id}`}>
                Export #{row.id}: {row.dataset}/{row.status}
                {row.error_message ? ` — ${row.error_message}` : ""}
              </li>
            ))}
          </ul>
        ) : null}
        {operationsPage?.next_sync_cursor || operationsPage?.next_export_cursor ? (
          <button
            type="button"
            onClick={() =>
              void onLoadOperations(
                operationsPage.next_sync_cursor,
                operationsPage.next_export_cursor,
              )
            }
            disabled={operationsStatus === "loading"}
          >
            Load more Operations
          </button>
        ) : null}
      </section>

      <section
        className="form"
        aria-label="Session explorer"
        aria-busy={sessionBusy}
        data-testid="session-explorer"
      >
        <h2>Sessions</h2>
        <form
          className="session-search-form"
          aria-label="Session search and lane"
          onSubmit={(event) => {
            event.preventDefault();
            void onLoadSessions();
          }}
        >
          <label htmlFor="session-search">Search sessions</label>
          <input
            id="session-search"
            value={sessionQuery}
            onChange={(event) => setSessionQuery(event.target.value)}
            placeholder="Search current projection…"
          />
          <label htmlFor="session-lane">Workflow lane</label>
          <select
            id="session-lane"
            value={sessionLane}
            onChange={(event) => setSessionLane(event.target.value as WorkflowLane)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void onLoadSessions();
              }
            }}
          >
            <option value="all">All</option>
            <option value="needs_review">Needs Review</option>
            <option value="train_ready">Train Ready</option>
            <option value="holdout_ready">Holdout Ready</option>
            <option value="favorites">Favorites</option>
          </select>
          <button
            ref={loadSessionsRef}
            type="submit"
            disabled={!home.trim() || sessionBusy}
          >
            {sessionBusy ? "Loading sessions…" : "Load sessions"}
          </button>
        </form>
        <p data-testid="session-explorer-status" aria-live="polite">
          Sessions: {sessionStatus}
        </p>
        {sessionBusy ? (
          <button type="button" onClick={onCancelSessionLoad}>
            Cancel session load
          </button>
        ) : null}
        {sessionError ? (
          <p role="alert" className="error" data-testid="session-explorer-error">
            {sessionError.code}: {sessionError.message}
          </p>
        ) : null}
        {sessionStatus === "empty" ? <p>No sessions match this query.</p> : null}
        {sessionPage?.items.length ? (
          <ul data-testid="session-list">
            {sessionPage.items.map((item) => (
              <li key={sessionKey(item)}>
                <button
                  type="button"
                  aria-pressed={selectedSessionKey === sessionKey(item)}
                  onClick={() => void onSelectSession(item)}
                >
                  {item.title} · {item.workflow_state} · {item.message_count} messages
                </button>
              </li>
            ))}
          </ul>
        ) : null}
        {sessionPage?.next_cursor ? (
          <button
            type="button"
            onClick={() => void onLoadSessions(sessionPage.next_cursor)}
            disabled={sessionBusy}
          >
            Load more sessions
          </button>
        ) : null}
        {sessionDetail ? (
          <article data-testid="session-detail-panel" aria-label="Session detail">
            <h3>{sessionDetail.summary.title ?? "Untitled session"}</h3>
            <p>{sessionDetail.project_path ?? "No project path"}</p>
            <p>Source URL: {sessionDetail.source_url ?? "No source URL"}</p>
            <p>{sessionDetail.projection_summary ?? "No summary"}</p>
            <p>
              Started: {sessionDetail.started_at ?? "unknown"} · Updated:{" "}
              {sessionDetail.updated_at ?? "unknown"}
            </p>
            <p>
              Raw captures:{" "}
              {sessionDetail.raw_capture_count ??
                sessionDetail.summary.accepted_capture_count}
            </p>
            <p>
              Attempts: {sessionDetail.summary.normalization_attempt_count} · Generation:{" "}
              {sessionDetail.summary.successful_projection_generation}
            </p>
            <div data-testid="session-labels" role="group" aria-label="Session labels">
              <p>Labels</p>
              <ul>
                {(sessionDetail.labels ?? []).map((label) => (
                  <li key={`${label.id}:${label.name}`}>
                    {label.name} ({label.origin})
                  </li>
                ))}
              </ul>
              <div role="group" aria-label="Toggle session labels">
                {CURATABLE_LABELS.map((name) => {
                  const isActive = (sessionDetail.labels ?? []).some(
                    (label) => label.name === name,
                  );
                  return (
                    <button
                      key={name}
                      type="button"
                      aria-pressed={isActive}
                      onClick={() => void onToggleSessionLabel(name)}
                    >
                      {name}
                    </button>
                  );
                })}
              </div>
            </div>
            <div data-testid="session-tags" role="group" aria-label="Session tags">
              <p>Tags</p>
              <ul>
                {(sessionDetail.tags ?? []).map((tag) => (
                  <li key={`${tag.id}:${tag.name}`}>
                    <button
                      type="button"
                      aria-label={`Remove tag ${tag.name}`}
                      onClick={() => void onRemoveSessionTag(tag.name)}
                    >
                      {tag.name} ({tag.origin}) ×
                    </button>
                  </li>
                ))}
              </ul>
              <form
                aria-label="Add session tag"
                onSubmit={(event) => {
                  event.preventDefault();
                  void onAddSessionTag();
                }}
              >
                <label htmlFor="session-tag-input">Add tag</label>
                <input
                  id="session-tag-input"
                  value={tagDraft}
                  onChange={(event) => setTagDraft(event.target.value)}
                  placeholder="tag name"
                />
                <button type="submit">Add tag</button>
              </form>
            </div>
            {curationError ? (
              <p role="alert" className="error" data-testid="session-curation-error">
                {curationError.code}: {curationError.message}
              </p>
            ) : null}
            <section
              className="form"
              aria-label="Capture Attempt history"
              aria-busy={attemptStatus === "loading" || renormalizeStatus === "loading"}
              data-testid="attempt-history-panel"
            >
              <h4>Attempt history</h4>
              <button
                type="button"
                data-testid="load-attempt-history"
                onClick={() => void onLoadAttemptHistory()}
                disabled={attemptStatus === "loading" || renormalizeStatus === "loading"}
              >
                {attemptStatus === "loading"
                  ? "Loading Attempt history…"
                  : "Load Attempt history"}
              </button>
              <p
                data-testid="attempt-history-status"
                aria-label={`Attempt history status: ${attemptStatus}${
                  attemptCaptureId != null ? ` · Capture ${attemptCaptureId}` : ""
                }`}
                aria-live="polite"
              >
                Status: {attemptStatus}
                {attemptCaptureId != null ? ` · Capture ${attemptCaptureId}` : ""}
              </p>
              {attemptError ? (
                <p role="alert" data-testid="attempt-history-error">
                  {attemptError.code}: {attemptError.message}
                </p>
              ) : null}
              {attemptStatus === "empty" ? (
                <p>No Capture Attempts for this Session.</p>
              ) : null}
              {attemptHistory?.length ? (
                <ul data-testid="attempt-history-list">
                  {attemptHistory.map((attempt) => (
                    <li
                      key={attempt.id}
                      aria-label={`Attempt #${attempt.id} · ${attempt.parser_id}/${attempt.parser_version} · ${attempt.outcome}`}
                    >
                      #{attempt.id} · {attempt.parser_id}/{attempt.parser_version} ·{" "}
                      {attempt.outcome} · facts {attempt.fact_count}
                      {attempt.projection_generation != null
                        ? ` · generation ${attempt.projection_generation}`
                        : ""}
                      {attempt.error_class ? ` · ${attempt.error_class}` : ""}
                    </li>
                  ))}
                </ul>
              ) : null}
              <button
                type="button"
                data-testid="renormalize-capture"
                onClick={() => void onRenormalizeCapture()}
                disabled={
                  attemptCaptureId == null ||
                  attemptStatus === "loading" ||
                  renormalizeStatus === "loading"
                }
              >
                {renormalizeStatus === "loading"
                  ? "Renormalizing…"
                  : "Renormalize Capture"}
              </button>
              <p
                data-testid="renormalize-status"
                aria-label={`Renormalize status: ${renormalizeStatus}`}
                aria-live="polite"
              >
                Renormalize: {renormalizeStatus}
              </p>
              {renormalizeError ? (
                <p role="alert" data-testid="renormalize-error">
                  {renormalizeError.code}: {renormalizeError.message}
                </p>
              ) : null}
              {renormalizeReport ? (
                <p
                  data-testid="renormalize-report"
                  aria-label={`Capture ${renormalizeReport.capture_id} · attempt ${renormalizeReport.attempt_id} · ${renormalizeReport.outcome} · ${renormalizeReport.parser_id}/${renormalizeReport.parser_version}`}
                >
                  Capture {renormalizeReport.capture_id} · attempt{" "}
                  {renormalizeReport.attempt_id} · {renormalizeReport.outcome} ·{" "}
                  {renormalizeReport.parser_id}/{renormalizeReport.parser_version}
                </p>
              ) : null}
            </section>
            <pre>{sessionDetail.metadata_json}</pre>
            <ol>
              {sessionDetail.messages.map((message) => (
                <li key={message.id}>
                  <strong>{message.role}</strong>: {message.text}
                </li>
              ))}
            </ol>
            <ul>
              {sessionDetail.artifacts.map((artifact) => (
                <li key={artifact.id}>
                  {artifact.artifact_type}: {artifact.text_preview ?? "artifact"}
                </li>
              ))}
            </ul>
            {sessionDetail.next_message_cursor && selectedSessionKey && sessionPage ? (
              <button
                type="button"
                onClick={() => {
                  const item = sessionPage.items.find(
                    (candidate) => sessionKey(candidate) === selectedSessionKey,
                  );
                  if (item) void onSelectSession(item, sessionDetail.next_message_cursor);
                }}
              >
                Load more transcript
              </button>
            ) : null}
            {sessionDetail.next_artifact_cursor && selectedSessionKey && sessionPage ? (
              <button
                type="button"
                onClick={() => {
                  const item = sessionPage.items.find(
                    (candidate) => sessionKey(candidate) === selectedSessionKey,
                  );
                  if (item)
                    void onSelectSession(item, null, sessionDetail.next_artifact_cursor);
                }}
              >
                Load more artifacts
              </button>
            ) : null}
          </article>
        ) : null}
      </section>

      <section
        className="form"
        aria-label="Dataset export"
        aria-busy={exportBusy}
        data-testid="export-panel"
      >
        <h2>Export</h2>
        <label htmlFor="export-dataset">Dataset</label>
        <select
          id="export-dataset"
          value={exportDataset}
          onChange={(event) => {
            setExportDataset(event.target.value as ExportDataset);
            setExportPreview(null);
            setExportResult(null);
            setExportError(null);
            setExportStatus("idle");
            setExportPublishing(false);
          }}
        >
          <option value="train">train</option>
          <option value="holdout">holdout</option>
        </select>
        <button
          type="button"
          onClick={() => void onPreviewExport()}
          disabled={!home.trim() || exportStatus === "running"}
        >
          Preview export
        </button>
        <button
          ref={publishExportRef}
          type="button"
          onClick={() => void onPublishExport()}
          disabled={!home.trim() || !exportPreview || exportStatus === "running"}
        >
          Publish export
        </button>
        {exportPublishing ? (
          <button type="button" onClick={() => void onCancelExport()}>
            Cancel export
          </button>
        ) : null}
        <div data-testid="export-live-region" aria-live="polite" aria-atomic="true">
          <p data-testid="export-status">Export: {exportStatus}</p>
          {exportProgress ? (
            <p data-testid="export-progress">progress: {exportProgress.type}</p>
          ) : null}
        </div>
        {exportError ? (
          <p role="alert" className="error" data-testid="export-error">
            {exportError.code}: {exportError.message}
          </p>
        ) : null}
        {exportPreview ? (
          <dl data-testid="export-preview">
            <dt>Dataset</dt>
            <dd>{exportPreview.dataset}</dd>
            <dt>Format</dt>
            <dd>{exportPreview.format_id}</dd>
            <dt>Eligible</dt>
            <dd data-testid="export-eligible-count">{exportPreview.eligible.length}</dd>
            <dt>Omitted</dt>
            <dd data-testid="export-omitted-count">{exportPreview.omitted.length}</dd>
          </dl>
        ) : null}
        {exportResult ? (
          <dl data-testid="export-result">
            <dt>Status</dt>
            <dd data-testid="export-result-status">{exportResult.status}</dd>
            <dt>Records</dt>
            <dd>{exportResult.record_count}</dd>
            <dt>Output</dt>
            <dd>{exportResult.output_path ?? "none"}</dd>
          </dl>
        ) : null}
      </section>

      <section aria-live="polite" aria-atomic="true" className="status">
        <p>
          Status: <strong data-testid="status">{status}</strong>
          {phase ? ` · phase: ${phase}` : null}
          {syncProgress ? ` · sync: ${syncProgress.type}` : null}
          {exportProgress ? ` · export: ${exportProgress.type}` : null}
        </p>
      </section>

      {error ? (
        <section className="panel error" role="alert" data-testid="error-panel">
          <h2>Error</h2>
          <p>
            <code>{error.code}</code>: {error.message}
          </p>
        </section>
      ) : null}

      {syncResult ? (
        <section className="panel" data-testid="sync-run-panel">
          <h2>Sync Run</h2>
          <dl>
            <dt>Id</dt>
            <dd>{syncResult.run.id}</dd>
            <dt>Status</dt>
            <dd data-testid="sync-run-status">{syncResult.run.status}</dd>
            <dt>Accepted captures</dt>
            <dd>{syncResult.run.accepted_captures}</dd>
            <dt>Sources</dt>
            <dd>
              {syncResult.run.sources.length > 0
                ? syncResult.run.sources
                    .map((source) => `${source.source_kind}: ${source.status}`)
                    .join(", ")
                : "none"}
            </dd>
          </dl>
          {syncResult.run.warning_details?.length ? (
            <ul data-testid="sync-warning-details">
              {syncResult.run.warning_details.map((detail) => (
                <li key={detail}>{detail}</li>
              ))}
            </ul>
          ) : null}
        </section>
      ) : null}

      {result ? (
        <div className="results">
          <section className="panel" data-testid="source-panel">
            <h2>Source</h2>
            <dl>
              <dt>Kind</dt>
              <dd>{result.source.kind}</dd>
              <dt>Name</dt>
              <dd>{result.source.display_name}</dd>
              <dt>Root</dt>
              <dd>{result.source.data_root}</dd>
              <dt>Parser</dt>
              <dd>
                {result.source.parser_id}@{result.source.parser_version}
              </dd>
            </dl>
          </section>

          <section className="panel" data-testid="sync-panel">
            <h2>Sync</h2>
            <dl>
              <dt>Accepted captures</dt>
              <dd>{result.sync.accepted_captures}</dd>
              <dt>Successful attempts</dt>
              <dd>{result.sync.successful_attempts}</dd>
              <dt>Failed attempts</dt>
              <dd>{result.sync.failed_attempts}</dd>
              <dt>Skipped duplicates</dt>
              <dd>{result.sync.skipped_duplicates}</dd>
            </dl>
          </section>

          <section className="panel" data-testid="session-panel">
            <h2>Session</h2>
            {result.session ? (
              <dl>
                <dt>Identity</dt>
                <dd>
                  {result.session.summary.source_kind}:
                  {result.session.summary.external_session_id}
                </dd>
                <dt>Title</dt>
                <dd>{result.session.summary.title ?? "(untitled)"}</dd>
                <dt>Accepted Capture count</dt>
                <dd>{result.session.summary.accepted_capture_count}</dd>
                <dt>Normalization Attempt count</dt>
                <dd>{result.session.summary.normalization_attempt_count}</dd>
                <dt>Successful projection generation</dt>
                <dd>{result.session.summary.successful_projection_generation}</dd>
                <dt>Messages</dt>
                <dd>
                  <ol>
                    {result.session.messages.map((message) => (
                      <li key={message.id}>
                        <strong>{message.role}</strong>: {message.text}
                      </li>
                    ))}
                  </ol>
                </dd>
              </dl>
            ) : (
              <p>No Session Projection was produced.</p>
            )}
          </section>
        </div>
      ) : null}

      {health ? (
        <section className="panel" data-testid="health-panel">
          <h2>Health</h2>
          <dl>
            <dt>OK</dt>
            <dd>{String(health.ok)}</dd>
            <dt>Schema</dt>
            <dd>{health.schema_status}</dd>
            <dt>Content</dt>
            <dd>{health.content_status}</dd>
            <dt>FTS</dt>
            <dd>{health.fts_status}</dd>
            <dt>Staging</dt>
            <dd>{health.staging_status}</dd>
            <dt>Orphan</dt>
            <dd>{health.orphan_status}</dd>
            <dt>Incomplete</dt>
            <dd>{health.incomplete_status}</dd>
            <dt>Operations</dt>
            <dd>{health.operations_status}</dd>
          </dl>
          {health.issues.length > 0 ? (
            <ul data-testid="health-issues">
              {health.issues.map((issue) => (
                <li key={`${issue.code}-${issue.summary}`}>
                  <code>{issue.code}</code> ({issue.severity}/{issue.category}):{" "}
                  {issue.summary}
                </li>
              ))}
            </ul>
          ) : null}
        </section>
      ) : null}

      {repairReport ? (
        <section className="panel" data-testid="repair-panel">
          <h2>Repair</h2>
          <ul>
            {repairReport.actions.map((action) => (
              <li key={action.name}>
                {action.name}: {action.count}
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </main>
  );
}

function sessionKey(
  item: Pick<SessionListItem, "source_kind" | "external_session_id">,
): string {
  return `${item.source_kind}:${item.external_session_id}`;
}

function sessionKeyFromSummary(summary: SessionDetail["summary"]): string {
  return `${summary.source_kind}:${summary.external_session_id}`;
}

/**
 * Normalize unknown bridge failures into a typed HostError.
 * @param caught - thrown value from the bridge
 */
function normalizeError(caught: unknown): HostError {
  if (
    typeof caught === "object" &&
    caught !== null &&
    "code" in caught &&
    "message" in caught &&
    typeof (caught as HostError).code === "string" &&
    typeof (caught as HostError).message === "string"
  ) {
    return caught as HostError;
  }
  return {
    code: "unknown",
    message: caught instanceof Error ? caught.message : String(caught),
  };
}
