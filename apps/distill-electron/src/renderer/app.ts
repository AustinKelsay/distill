import {
  AppView,
  AppSettingsSnapshot,
  BackgroundSyncStatus,
  DashboardData,
  DatasetExportTarget,
  DbBrowseRequest,
  DbBrowseResult,
  DbColumnInfo,
  DbExplorerSnapshot,
  DbFilter,
  DbFilterOperator,
  DbQueryRequest,
  DbQueryResult,
  DbRowCount,
  DbResultColumn,
  DbResultRow,
  DbSort,
  DbTableSummary,
  DoctorReport,
  DiscoveredSource,
  ExportReport,
  LogEntry,
  LogsPageData,
  SearchResult,
  SessionArtifact,
  SessionDetail,
  SessionListItem,
  SessionWorkflowState,
  SourceColors
} from "../shared/types";

declare global {
  interface Window {
    distillApi: {
      getDoctorReport: () => DoctorReport;
      getDashboardData: () => DashboardData;
      getSessionDetail: (sessionId: number) => SessionDetail | undefined;
      searchSessions: (query: string) => SearchResult[];
      getLogsPageData: () => LogsPageData;
      addSessionTag: (sessionId: number, tagName: string) => void;
      removeSessionTag: (sessionId: number, tagId: number) => void;
      toggleSessionLabel: (sessionId: number, labelName: string) => void;
      getDefaultLabelNames: () => string[];
      exportApprovedSessions: (dataset: DatasetExportTarget) => ExportReport;
      setSourceColor: (sourceKind: string, color: string) => SourceColors;
      getAppSettings: () => AppSettingsSnapshot;
      getDbExplorerSnapshot: () => Promise<DbExplorerSnapshot>;
      browseDbTable: (request: DbBrowseRequest) => Promise<DbBrowseResult>;
      runDbQuery: (request: DbQueryRequest) => Promise<DbQueryResult>;
      getBackgroundSyncStatus: () => Promise<BackgroundSyncStatus>;
      requestBackgroundSync: () => Promise<BackgroundSyncStatus>;
      onBackgroundSyncStatus: (listener: (status: BackgroundSyncStatus) => void) => () => void;
    };
  }
}

type SessionFilterLane = "all" | "needs_review" | "train_ready" | "holdout_ready" | "favorite";

let dashboardData: DashboardData | null = null;
let logsPageData: LogsPageData | null = null;
let activeSessionId: number | null = null;
let exportTimeout: ReturnType<typeof setTimeout> | null = null;
let syncStatusUnsubscribe: (() => void) | null = null;
let isSettingsOpen = false;
let floatingOverlaysBound = false;
let activeView: AppView = "sessions";
let activeSessionLane: SessionFilterLane = "all";
let logsSearchQuery = "";
let activeLogsFilter: "all" | "sync" | "export" | "errors" = "all";
let exportDropdownDismissBound = false;
let dbExplorerSnapshot: DbExplorerSnapshot | null = null;
let dbExplorerError: string | null = null;
let dbExplorerLoading = false;
let dbShowInternalTables = false;
let dbSelectedTableName: string | null = null;
let dbActiveTab: "browse" | "query" = "browse";
let dbBrowseResult: DbBrowseResult | null = null;
let dbBrowseError: string | null = null;
let dbBrowseLoading = false;
let dbBrowseFilters: DbFilter[] = [];
let dbBrowseSort: DbSort | undefined;
let dbBrowsePage = 1;
let dbBrowsePageSize = 50;
let dbQueryText = "";
let dbQueryResult: DbQueryResult | null = null;
let dbQueryError: string | null = null;
let dbQueryRunning = false;
let dbQueryIsStale = false;
let dbSelectedRowKey: string | null = null;
let dbSnapshotRequestToken = 0;
let dbBrowseRequestToken = 0;
let dbQueryRequestToken = 0;
let tooltipShowTimeout: ReturnType<typeof setTimeout> | null = null;
let activeTooltipAnchor: HTMLElement | null = null;
let activeHelpTipAnchor: HTMLElement | null = null;

const DB_QUERY_PLACEHOLDER = `SELECT id, title, updated_at\nFROM sessions\nORDER BY updated_at DESC\nLIMIT 25;`;

const DB_OPERATOR_LABELS: Record<DbFilterOperator, string> = {
  contains: "contains",
  equals: "equals",
  not_equals: "not equals",
  starts_with: "starts with",
  ends_with: "ends with",
  eq: "=",
  neq: "!=",
  gt: ">",
  gte: ">=",
  lt: "<",
  lte: "<=",
  is_null: "is null",
  is_not_null: "is not null"
};

/* Helpers */

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function timeAgo(dateStr: string | undefined): string {
  if (!dateStr) return "-";
  const diff = Date.now() - new Date(dateStr).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(dateStr).toLocaleDateString();
}

function showExportToast(message: string): void {
  const el = document.querySelector<HTMLElement>("[data-export-status]");
  if (!el) return;
  el.textContent = message;
  el.classList.add("visible");
  if (exportTimeout) clearTimeout(exportTimeout);
  exportTimeout = setTimeout(() => el.classList.remove("visible"), 4000);
}

function renderHelpTip(text: string, title?: string, label = "?"): string {
  const safe = escapeHtml(text);
  const safeTitle = title ? ` data-help-title="${escapeHtml(title)}"` : "";
  return `<button class="help-tip" type="button" data-help-tip="${safe}"${safeTitle} aria-label="${safe}">${escapeHtml(label)}</button>`;
}

function tooltipAttrs(text: string): string {
  const safe = escapeHtml(text);
  return `data-tooltip="${safe}"`;
}

function titleAttr(text: string): string {
  return `title="${escapeHtml(text)}"`;
}

function sourceLabel(sourceKind: SessionListItem["sourceKind"] | SearchResult["sourceKind"] | SessionDetail["sourceKind"]): string {
  if (sourceKind === "claude_code") {
    return "claude";
  }

  if (sourceKind === "opencode") {
    return "opencode";
  }

  return "codex";
}

function sourceBadgeClass(sourceKind: SessionListItem["sourceKind"] | SearchResult["sourceKind"] | SessionDetail["sourceKind"]): string {
  if (sourceKind === "claude_code") return "badge-source-claude";
  if (sourceKind === "opencode") return "badge-source-opencode";
  return "badge-source-codex";
}

function hexToRgba(hex: string, alpha: number): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function applySourceColors(colors: SourceColors): void {
  const root = document.documentElement;
  root.style.setProperty("--source-codex", colors.codex ?? "#3dbf9a");
  root.style.setProperty("--source-codex-bg", hexToRgba(colors.codex ?? "#3dbf9a", 0.14));
  root.style.setProperty("--source-claude", colors.claude_code ?? "#d4944a");
  root.style.setProperty("--source-claude-bg", hexToRgba(colors.claude_code ?? "#d4944a", 0.14));
  root.style.setProperty("--source-opencode", colors.opencode ?? "#a88cd4");
  root.style.setProperty("--source-opencode-bg", hexToRgba(colors.opencode ?? "#a88cd4", 0.14));
}

function renderSyncStatus(status: BackgroundSyncStatus): void {
  const el = document.querySelector<HTMLElement>("[data-sync-status]");
  if (!el) return;

  el.textContent = syncStatusText(status);
  el.title = status.errorText ?? status.summary;
  el.dataset.state = syncStatusTone(status);
}

function syncStatusText(status: BackgroundSyncStatus | undefined): string {
  if (!status) return "idle";

  return status.state === "queued" ? "sync queued"
    : status.state === "running" ? "syncing..."
    : status.state === "warning" ? (status.finishedAt ? `sync warnings ${timeAgo(status.finishedAt)}` : "sync warnings")
    : status.state === "failed" ? "sync failed"
    : status.state === "completed" ? (status.finishedAt ? `synced ${timeAgo(status.finishedAt)}` : "synced")
    : "idle";
}

function syncStatusTone(status: BackgroundSyncStatus | undefined): "idle" | "queued" | "running" | "completed" | "warning" | "failed" {
  if (!status) {
    return "idle";
  }

  if (
    status.state === "idle"
    || status.state === "queued"
    || status.state === "running"
    || status.state === "warning"
    || status.state === "failed"
  ) {
    return status.state;
  }

  return "completed";
}

function formatDateTime(dateStr: string | undefined): string {
  if (!dateStr) return "-";
  return new Date(dateStr).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit"
  });
}

function renderDetailContextRow(label: string, value: string, kind: "value" | "copy" = "value"): string {
  return `
    <div class="detail-context-row">
      <div class="detail-context-label">${escapeHtml(label)}</div>
      <div class="${kind === "copy" ? "detail-context-copy" : "detail-context-value"}">${escapeHtml(value)}</div>
    </div>
  `;
}

function renderDetailContextFullRow(label: string, content: string, kind: "value" | "copy" | "json"): string {
  const valueHtml =
    kind === "json" ? `<pre class="detail-context-json">${escapeHtml(content)}</pre>`
    : `<div class="${kind === "copy" ? "detail-context-copy" : "detail-context-value"}">${escapeHtml(content)}</div>`;

  return `
    <div class="detail-context-row detail-context-row--full">
      <div class="detail-context-label">${escapeHtml(label)}</div>
      ${valueHtml}
    </div>
  `;
}

function logStatusBadgeClass(entry: LogEntry): string {
  if (entry.status === "failed") return "badge-status-failed";
  if (entry.status === "running") return "badge-status-running";
  if (entry.status === "queued") return "badge-status-queued";
  if (entry.status === "warning") return "badge-status-warning";
  if (entry.level === "error") return "badge-status-failed";
  return "badge-status-completed";
}

function renderLogMetrics(entry: LogEntry): string {
  if (!entry.metrics) {
    return "";
  }

  if (entry.kind === "sync") {
    const d = entry.metrics.discoveredCaptures ?? 0;
    const i = entry.metrics.importedCaptures ?? 0;
    const s = entry.metrics.skippedCaptures ?? 0;
    const f = entry.metrics.failedCaptures ?? 0;
    return `
      <div class="log-metrics">
        <span ${tooltipAttrs("Captures discovered from source directories")}>${d} found</span>
        <span ${tooltipAttrs("Captures imported into the database")}>${i} imported</span>
        <span ${tooltipAttrs("Captures already imported, unchanged")}>${s} skipped</span>
        <span ${tooltipAttrs("Captures that failed to parse or import")}>${f} failed</span>
      </div>
    `;
  }

  return `
    <div class="log-metrics">
      <span>${entry.sourceLabel ? `${entry.sourceLabel} dataset` : "dataset export"}</span>
      <span>${entry.metrics.recordCount ?? 0} records</span>
    </div>
  `;
}

function renderLogEntry(entry: LogEntry): string {
  const details = entry.details;
  const sourceSummaries = details?.sourceSummaries?.length
    ? `
      <section class="log-detail-section">
        <div class="log-detail-title">Source Summary</div>
        <div class="log-source-summary-list">
          ${details.sourceSummaries.map((summary) => `
            <div class="log-source-summary">
              <span class="badge ${sourceBadgeClass(summary.kind)}">${sourceLabel(summary.kind)}</span>
              <span>${summary.discoveredCaptures} found</span>
              <span>${summary.importedCaptures} imported</span>
              <span>${summary.skippedCaptures} skipped</span>
              <span>${summary.failedCaptures} failed</span>
            </div>
          `).join("")}
        </div>
      </section>
    `
    : "";
  const failedEntries = details?.failedEntries?.length
    ? `
      <section class="log-detail-section">
        <div class="log-detail-title">Failures</div>
        <div class="log-failure-list">
          ${details.failedEntries.map((failure) => `
            <div class="log-failure-item">
              <div class="log-failure-head">
                <span class="badge ${sourceBadgeClass(failure.sourceKind)}">${sourceLabel(failure.sourceKind)}</span>
                <span class="log-failure-path">${escapeHtml(failure.sourcePath)}</span>
              </div>
              <div class="log-failure-copy">${escapeHtml(failure.errorText)}</div>
            </div>
          `).join("")}
        </div>
      </section>
    `
    : "";
  const detailRows = [
    details?.reason ? `<div class="log-detail-row"><span class="log-detail-key">Reason</span><span class="log-detail-value">${escapeHtml(details.reason)}</span></div>` : "",
    details?.dataset ? `<div class="log-detail-row"><span class="log-detail-key">Dataset</span><span class="log-detail-value">${escapeHtml(details.dataset)}</span></div>` : "",
    details?.outputPath ? `<div class="log-detail-row"><span class="log-detail-key">Output</span><span class="log-detail-value">${escapeHtml(details.outputPath)}</span></div>` : ""
  ].filter(Boolean).join("");

  return `
    <details class="log-card ${entry.level === "error" ? "is-error" : entry.status === "warning" ? "is-warning" : ""}">
      <summary>
        <div class="log-card-topline">
          <span class="log-timestamp">${escapeHtml(formatDateTime(entry.updatedAt ?? entry.createdAt))}</span>
          <span class="badge badge-log-kind">${escapeHtml(entry.kind)}</span>
          <span class="badge ${logStatusBadgeClass(entry)}">${escapeHtml(entry.status)}</span>
          ${entry.sourceLabel ? `<span class="log-source-label">${escapeHtml(entry.sourceLabel)}</span>` : ""}
        </div>
        <div class="log-card-title">${escapeHtml(entry.summary)}</div>
        <div class="log-card-subtitle">
          <span>${escapeHtml(entry.title)}</span>
          ${renderLogMetrics(entry)}
        </div>
      </summary>
      <div class="log-card-body">
        ${detailRows ? `<section class="log-detail-section">${detailRows}</section>` : ""}
        ${sourceSummaries}
        ${failedEntries}
        <section class="log-detail-section">
          <div class="log-detail-title">Raw</div>
          <pre class="log-raw">${escapeHtml(entry.rawJson)}</pre>
        </section>
      </div>
    </details>
  `;
}

function filteredLogEntries(data: LogsPageData): LogEntry[] {
  const query = logsSearchQuery.trim().toLowerCase();
  return data.entries.filter((entry) => {
    if (activeLogsFilter === "sync" && entry.kind !== "sync") return false;
    if (activeLogsFilter === "export" && entry.kind !== "export") return false;
    if (activeLogsFilter === "errors" && entry.level !== "error") return false;

    if (!query) return true;

    const haystack = [
      entry.kind,
      entry.status,
      entry.title,
      entry.summary,
      entry.sourceLabel,
      entry.details?.reason,
      entry.details?.dataset,
      entry.details?.outputPath,
      entry.rawJson,
      ...(entry.details?.failedEntries ?? []).flatMap((failure) => [failure.sourceKind, failure.sourcePath, failure.errorText])
    ]
      .filter(Boolean)
      .join("\n")
      .toLowerCase();

    return haystack.includes(query);
  });
}

function renderLogsView(data: LogsPageData): void {
  const root = document.querySelector<HTMLElement>("[data-session-detail]");
  if (!root) return;

  const entries = filteredLogEntries(data);
  const lastSyncLabel = syncStatusText(data.lastSyncStatus);
  const lastSyncTone = syncStatusTone(data.lastSyncStatus);
  const emptyState = data.entries.length === 0
    ? `
      <div class="detail-empty">
        <div class="empty-state">
          <div class="empty-title">No logs yet</div>
          <div class="empty-copy">Sync and export activity will show up here once Distill has operational history to surface.</div>
        </div>
      </div>
    `
    : `
      <div class="detail-empty">
        <div class="empty-state">
          <div class="empty-title">No matching logs</div>
          <div class="empty-copy">Adjust the log search or filters to widen the current result set.</div>
        </div>
      </div>
    `;

  root.innerHTML = `
    <div class="logs-view fade-in">
      <div class="logs-header">
        <div>
          <div class="detail-title">Logs</div>
          <div class="logs-subtitle">Operational sync and export history that normally only shows up in the shell. ${renderHelpTip("Each card shows a sync or export operation with timing, metrics, and any errors. Expand a card to see per-source breakdowns and raw JSON.", "Logs")}</div>
        </div>
        <div class="logs-summary">
          <span class="log-summary-chip">${data.counts.total} entries</span>
          <span class="log-summary-chip ${data.counts.errors ? "is-error" : ""}">${data.counts.errors} errors</span>
          <span class="log-summary-chip ${lastSyncTone === "failed" ? "is-error" : lastSyncTone === "warning" ? "is-warning" : ""}">${escapeHtml(lastSyncLabel)}</span>
        </div>
      </div>
      <div class="logs-controls">
        <input class="logs-search-input" type="search" placeholder="Search logs…" value="${escapeHtml(logsSearchQuery)}" data-logs-search />
        <div class="logs-filter-row">
          <button class="chip ${activeLogsFilter === "all" ? "active" : ""}" type="button" data-logs-filter="all">All</button>
          <button class="chip ${activeLogsFilter === "sync" ? "active" : ""}" type="button" data-logs-filter="sync">Sync</button>
          <button class="chip ${activeLogsFilter === "export" ? "active" : ""}" type="button" data-logs-filter="export">Exports</button>
          <button class="chip ${activeLogsFilter === "errors" ? "active" : ""}" type="button" data-logs-filter="errors">Errors</button>
          ${renderHelpTip("Filter by operation type. Sync logs cover background imports from local source files. Export logs cover JSONL generation triggered from Sessions.", "Log Filters")}
        </div>
      </div>
      <div class="logs-list">
        ${entries.length ? entries.map(renderLogEntry).join("") : emptyState}
      </div>
    </div>
  `;

  bindLogsControls();
}

function renderLogsListOnly(data: LogsPageData): void {
  const listEl = document.querySelector<HTMLElement>(".logs-list");
  if (!listEl) {
    renderLogsView(data);
    return;
  }

  const entries = filteredLogEntries(data);
  const emptyHtml = data.entries.length === 0
    ? `<div class="detail-empty"><div class="empty-state"><div class="empty-title">No logs yet</div><div class="empty-copy">Sync and export activity will show up here once Distill has operational history to surface.</div></div></div>`
    : `<div class="detail-empty"><div class="empty-state"><div class="empty-title">No matching logs</div><div class="empty-copy">Adjust the log search or filters to widen the current result set.</div></div></div>`;

  listEl.innerHTML = `<div class="fade-in">${entries.length ? entries.map(renderLogEntry).join("") : emptyHtml}</div>`;
}

function getErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }

  return String(error);
}

function basenamePath(value: string): string {
  const parts = value.split(/[\\/]/);
  return parts[parts.length - 1] || value;
}

function defaultDbFilterOperator(column: DbColumnInfo): DbFilterOperator {
  if (column.filterKind === "text") {
    return "contains";
  }

  if (column.filterKind === "numeric" || column.filterKind === "date") {
    return "eq";
  }

  return "equals";
}

function dbOperatorsForColumn(column: DbColumnInfo): DbFilterOperator[] {
  if (column.filterKind === "text") {
    return ["contains", "equals", "not_equals", "starts_with", "ends_with", "is_null", "is_not_null"];
  }

  if (column.filterKind === "numeric" || column.filterKind === "date") {
    return ["eq", "neq", "gt", "gte", "lt", "lte", "is_null", "is_not_null"];
  }

  return ["equals", "not_equals", "is_null", "is_not_null"];
}

function dbOperatorNeedsValue(operator: DbFilterOperator): boolean {
  return operator !== "is_null" && operator !== "is_not_null";
}

function getVisibleDbTables(snapshot = dbExplorerSnapshot): DbTableSummary[] {
  if (!snapshot) {
    return [];
  }

  return dbShowInternalTables
    ? [...snapshot.coreTables, ...snapshot.advancedTables]
    : [...snapshot.coreTables];
}

function findDbTableSummary(tableName: string | null): DbTableSummary | undefined {
  if (!tableName || !dbExplorerSnapshot) {
    return undefined;
  }

  return [...dbExplorerSnapshot.coreTables, ...dbExplorerSnapshot.advancedTables].find(
    (table) => table.name === tableName
  );
}

function getDbVisibleColumns(): DbColumnInfo[] {
  return dbBrowseResult?.schemaColumns.filter((column) => !column.isHidden) ?? [];
}

function createDefaultDbFilter(column = getDbVisibleColumns()[0]): DbFilter | null {
  if (!column) {
    return null;
  }

  return {
    column: column.name,
    operator: defaultDbFilterOperator(column),
    value: ""
  };
}

function formatDbRowCount(totalRows: DbRowCount): string {
  return totalRows.toLocaleString();
}

function getDbPageCount(totalRows: DbRowCount, pageSize: number): number {
  if (typeof totalRows === "bigint") {
    if (totalRows <= 0n) {
      return 1;
    }

    const pageCount = (totalRows + BigInt(pageSize) - 1n) / BigInt(pageSize);
    return pageCount > BigInt(Number.MAX_SAFE_INTEGER)
      ? Number.MAX_SAFE_INTEGER
      : Number(pageCount);
  }

  return Math.max(1, Math.ceil(totalRows / pageSize));
}

function normalizeDbSelectedTable(): void {
  const visibleTables = getVisibleDbTables();
  if (!visibleTables.length) {
    dbSelectedTableName = null;
    return;
  }

  if (dbSelectedTableName && visibleTables.some((table) => table.name === dbSelectedTableName)) {
    return;
  }

  const defaultTableName = dbExplorerSnapshot?.defaultTableName;
  const hasDefaultTable = defaultTableName
    ? visibleTables.some((table) => table.name === defaultTableName)
    : false;
  dbSelectedTableName =
    (hasDefaultTable ? defaultTableName : undefined)
    ?? visibleTables[0]?.name
    ?? null;
}

function renderDbEmptyState(title: string, copy: string, extraHtml = ""): string {
  return `
    <div class="detail-empty">
      <div class="empty-state">
        <div class="empty-title">${escapeHtml(title)}</div>
        <div class="empty-copy">${escapeHtml(copy)}</div>
        ${extraHtml}
      </div>
    </div>
  `;
}

function renderDbSchemaChip(column: DbColumnInfo): string {
  const badges = [
    column.isPrimaryKey ? `<span class="badge badge-db-meta">pk</span>` : "",
    column.isNullable ? `<span class="badge badge-db-meta">nullable</span>` : "",
    column.isHidden ? `<span class="badge badge-db-hidden">hidden</span>` : ""
  ].filter(Boolean).join("");

  return `
    <div class="db-schema-chip ${column.isHidden ? "is-hidden" : ""}">
      <div class="db-schema-title">${escapeHtml(column.name)}</div>
      <div class="db-schema-meta">
        <span>${escapeHtml(column.type ?? "ANY")}</span>
        ${badges}
      </div>
    </div>
  `;
}

function renderDbTableItem(table: DbTableSummary): string {
  return `
    <button
      class="session-item db-table-item ${dbSelectedTableName === table.name ? "selected" : ""}"
      type="button"
      data-db-table-name="${escapeHtml(table.name)}"
    >
      <div class="session-item-title">${escapeHtml(table.name)}</div>
      <div class="session-item-meta">
        <span class="badge ${table.kind === "virtual" ? "badge-db-virtual" : "badge-db-table"}">${escapeHtml(table.kind)}</span>
        ${table.isCore ? `<span class="badge badge-db-core">core</span>` : `<span class="badge badge-db-advanced">internal</span>`}
      </div>
    </button>
  `;
}

function renderDbSkeletonWorkspace(): string {
  const schemaChips = Array.from({length: 6}, () =>
    `<div class="skeleton-block" style="flex:0 0 auto;width:120px;height:58px;border-radius:10px"></div>`
  ).join("");

  const gridRows = Array.from({length: 8}, () =>
    `<div class="skeleton-row" style="margin-bottom:4px"></div>`
  ).join("");

  return `
    <div class="db-view fade-in" style="padding:20px 24px 48px">
      <div style="display:flex;flex-direction:column;gap:8px;margin-bottom:20px">
        <div class="skeleton-line" style="width:180px;height:18px"></div>
        <div class="skeleton-line" style="width:320px;height:12px"></div>
      </div>
      <div style="display:flex;gap:8px;margin-bottom:20px">
        <div class="skeleton-line" style="width:64px;height:30px;border-radius:6px"></div>
        <div class="skeleton-line" style="width:56px;height:30px;border-radius:6px"></div>
      </div>
      <div style="margin-bottom:20px">
        <div class="skeleton-line" style="width:60px;height:12px;margin-bottom:12px"></div>
        <div style="display:flex;gap:10px;overflow:hidden">${schemaChips}</div>
      </div>
      <div>${gridRows}</div>
    </div>
  `;
}

function renderDbSidebar(): void {
  const sessionsEl = document.querySelector<HTMLElement>("[data-sessions]");
  const countEl = document.querySelector<HTMLElement>("[data-session-count]");
  const scannedEl = document.querySelector<HTMLElement>("[data-scanned-at]");
  const sourcesToggle = document.querySelector<HTMLElement>("[data-sources-toggle]");
  const sourcesPanel = document.querySelector<HTMLElement>("[data-sources]");
  const statsEl = document.querySelector<HTMLElement>("[data-stats]");
  const onboarding = document.querySelector<HTMLElement>("[data-onboarding]");

  if (!sessionsEl) {
    return;
  }

  onboarding?.classList.remove("visible");
  sourcesToggle?.classList.add("is-hidden");
  sourcesPanel?.classList.remove("visible");
  if (sourcesPanel) {
    sourcesPanel.innerHTML = "";
  }
  if (statsEl) {
    statsEl.innerHTML = "";
  }

  const visibleTables = getVisibleDbTables();
  if (countEl) {
    countEl.textContent = dbExplorerSnapshot ? `${visibleTables.length} tables` : "Tables";
  }
  if (scannedEl) {
    scannedEl.textContent = dbExplorerSnapshot ? basenamePath(dbExplorerSnapshot.databasePath) : "";
  }

  if (dbExplorerLoading && !dbExplorerSnapshot) {
    sessionsEl.innerHTML = `
      <div class="fade-in" style="padding:12px 16px;display:flex;flex-direction:column;gap:6px">
        <div class="skeleton-line" style="width:100px;height:12px;margin-bottom:4px"></div>
        ${Array.from({length: 5}, () => `<div class="skeleton-block" style="height:42px"></div>`).join("")}
      </div>
    `;
    return;
  }

  if (dbExplorerError) {
    sessionsEl.innerHTML = `<div class="db-sidebar-message is-error">${escapeHtml(dbExplorerError)}</div>`;
    return;
  }

  if (!dbExplorerSnapshot?.databaseExists) {
    sessionsEl.innerHTML = `<div class="db-sidebar-message">No SQLite database found yet.</div>`;
    return;
  }

  const coreMarkup = dbExplorerSnapshot.coreTables.length
    ? `
      <div class="db-sidebar-section">
        <div class="db-sidebar-section-title">Core Tables</div>
        ${dbExplorerSnapshot.coreTables.map(renderDbTableItem).join("")}
      </div>
    `
    : "";

  const advancedMarkup = dbShowInternalTables && dbExplorerSnapshot.advancedTables.length
    ? `
      <div class="db-sidebar-section">
        <div class="db-sidebar-section-title">Internal Tables</div>
        ${dbExplorerSnapshot.advancedTables.map(renderDbTableItem).join("")}
      </div>
    `
    : "";

  sessionsEl.innerHTML = `
    <div class="fade-in">
    <div class="db-sidebar-controls">
      <label class="db-toggle">
        <input type="checkbox" data-db-show-internal ${dbShowInternalTables ? "checked" : ""} />
        <span>Show internal tables</span>
        ${renderHelpTip("Internal tables are used by Distill for indexing, FTS, and metadata. They are safe to inspect but not typically needed for day-to-day browsing.", "Internal Tables")}
      </label>
      <div class="db-sidebar-copy">Read-only browser over ${escapeHtml(dbExplorerSnapshot.databasePath)}</div>
    </div>
    ${coreMarkup}
    ${advancedMarkup}
    </div>
  `;
}

function renderDbFilterRow(filter: DbFilter, index: number, columns: DbColumnInfo[]): string {
  const column = columns.find((entry) => entry.name === filter.column) ?? columns[0];
  if (!column) {
    return "";
  }

  const operatorOptions = dbOperatorsForColumn(column).map((operator) =>
    `<option value="${operator}" ${filter.operator === operator ? "selected" : ""}>${escapeHtml(DB_OPERATOR_LABELS[operator])}</option>`
  ).join("");

  const columnOptions = columns.map((entry) =>
    `<option value="${escapeHtml(entry.name)}" ${entry.name === column.name ? "selected" : ""}>${escapeHtml(entry.name)}</option>`
  ).join("");

  return `
    <div class="db-filter-row">
      <select class="db-select" data-db-filter-column="${index}">
        ${columnOptions}
      </select>
      <select class="db-select" data-db-filter-operator="${index}">
        ${operatorOptions}
      </select>
      ${dbOperatorNeedsValue(filter.operator)
        ? `<input class="db-input" type="text" data-db-filter-value="${index}" value="${escapeHtml(filter.value ?? "")}" placeholder="Value" />`
        : `<div class="db-null-pill">No value</div>`}
      <button class="btn-ghost db-icon-button" type="button" data-db-remove-filter="${index}" title="Remove filter">×</button>
    </div>
  `;
}

function resolveDbSelectedRow(rows: DbResultRow[]): DbResultRow | undefined {
  if (!rows.length) {
    return undefined;
  }

  return rows.find((row) => row.key === dbSelectedRowKey) ?? rows[0];
}

function renderDbRowInspector(columns: DbResultColumn[], row: DbResultRow): string {
  const fields = columns.map((column, index) => {
    const cell = row.cells[index];
    if (!cell) {
      return "";
    }

    const notes = [
      cell.detailTruncated ? `<div class="db-field-note">Value truncated to 8 KB for display.</div>` : "",
      cell.kind === "blob" && cell.byteLength
        ? `<div class="db-field-note">Blob preview only. ${cell.byteLength} bytes.</div>`
        : ""
    ].filter(Boolean).join("");

    return `
      <div class="db-field">
        <div class="db-field-header">
          <span class="db-field-name">${escapeHtml(column.name)}</span>
          ${column.type ? `<span class="db-field-type">${escapeHtml(column.type)}</span>` : ""}
        </div>
        <pre class="db-field-value">${escapeHtml(cell.detail)}</pre>
        ${notes}
      </div>
    `;
  }).join("");

  return `
    <section class="db-row-inspector">
      <div class="db-section-title">Row Inspector ${renderHelpTip("Click any row in the table above to see all of its column values here. Long text and blob values are shown in full.", "Row Inspector")}</div>
      <div class="db-field-grid">
        ${fields}
      </div>
    </section>
  `;
}

function renderDbResultGrid(
  columns: DbResultColumn[],
  rows: DbResultRow[],
  emptyTitle: string,
  emptyCopy: string,
  notice?: string
): string {
  if (!rows.length) {
    return renderDbEmptyState(emptyTitle, emptyCopy);
  }

  const selectedRow = resolveDbSelectedRow(rows);
  const selectedRowKey = selectedRow?.key;
  const headerMarkup = columns.map((column) => `
    <th ${column.table || column.type ? titleAttr(
      [column.table, column.sourceColumn, column.type].filter(Boolean).join(" • ")
    ) : ""}>
      <div class="db-grid-head">
        <span>${escapeHtml(column.name)}</span>
        ${column.type ? `<span>${escapeHtml(column.type)}</span>` : ""}
      </div>
    </th>
  `).join("");

  const rowMarkup = rows.map((row) => `
    <tr
      class="${selectedRowKey === row.key ? "selected" : ""}"
      data-db-row-key="${escapeHtml(row.key)}"
      tabindex="0"
    >
      ${row.cells.map((cell) => `
        <td class="db-cell-${cell.kind}" ${titleAttr(cell.preview)}>
          ${escapeHtml(cell.preview)}
        </td>
      `).join("")}
    </tr>
  `).join("");

  return `
    ${notice ? `<div class="db-inline-notice">${escapeHtml(notice)}</div>` : ""}
    <div class="db-grid-shell">
      <div class="db-grid-wrap">
        <table class="db-grid">
          <thead>
            <tr>${headerMarkup}</tr>
          </thead>
          <tbody>${rowMarkup}</tbody>
        </table>
      </div>
      ${selectedRow ? renderDbRowInspector(columns, selectedRow) : ""}
    </div>
  `;
}

function renderDbBrowseTab(): string {
  if (dbBrowseLoading && !dbBrowseResult) {
    const skeletonRows = Array.from({length: 6}, () =>
      `<div class="skeleton-row" style="margin-bottom:4px"></div>`
    ).join("");
    return `
      <section class="db-panel fade-in">
        <div class="db-toolbar-row" style="pointer-events:none;opacity:0.5">
          <div class="db-toolbar-group">
            <div class="skeleton-line" style="width:60px;height:12px;margin-bottom:12px"></div>
            <div class="skeleton-line" style="width:100%;height:34px;border-radius:8px"></div>
          </div>
          <div class="db-toolbar-group">
            <div class="skeleton-line" style="width:40px;height:12px;margin-bottom:12px"></div>
            <div class="skeleton-line" style="width:100%;height:34px;border-radius:8px"></div>
          </div>
        </div>
        <div style="padding:0 16px 16px">${skeletonRows}</div>
      </section>
    `;
  }

  if (dbBrowseError) {
    return renderDbEmptyState("Browse unavailable", dbBrowseError);
  }

  if (!dbBrowseResult) {
    return renderDbEmptyState("Select a table", "Choose a table from the left to browse rows and schema.");
  }

  const columns = getDbVisibleColumns();
  const sort = dbBrowseSort ?? dbBrowseResult.sort;
  const filterRows = dbBrowseFilters.length
    ? dbBrowseFilters.map((filter, index) => renderDbFilterRow(filter, index, columns)).join("")
    : `<div class="db-filter-empty">No filters applied. Add one to narrow the current table.</div>`;
  const pageCount = getDbPageCount(dbBrowseResult.totalRows, dbBrowseResult.pageSize);
  const canGoBack = dbBrowseResult.page > 1;
  const canGoForward = dbBrowseResult.page < pageCount;
  const sortColumnOptions = columns.map((column) =>
    `<option value="${escapeHtml(column.name)}" ${sort.column === column.name ? "selected" : ""}>${escapeHtml(column.name)}</option>`
  ).join("");

  return `
    <section class="db-panel">
      <div class="db-toolbar-row">
        <div class="db-toolbar-group">
          <div class="db-section-title">Filters ${renderHelpTip("Add column filters to narrow the visible rows. Choose a column, an operator, and a value. Click Apply to execute.", "Filters")}</div>
          ${filterRows}
          <div class="db-filter-actions">
            <button class="btn-ghost" type="button" data-db-add-filter ${columns.length ? "" : "disabled"}>+ Filter</button>
            <button class="btn-primary" type="button" data-db-apply-browse ${dbBrowseLoading ? "disabled" : ""}>${dbBrowseLoading ? '<span class="spinner spinner--small"></span> Applying\u2026' : "Apply"}</button>
            <button class="btn-ghost" type="button" data-db-reset-browse>Reset</button>
          </div>
        </div>
        <div class="db-toolbar-group">
          <div class="db-section-title">Sort</div>
          <div class="db-sort-row">
            <select class="db-select" data-db-sort-column ${columns.length ? "" : "disabled"}>
              ${sortColumnOptions}
            </select>
            <select class="db-select" data-db-sort-direction>
              <option value="asc" ${sort.direction === "asc" ? "selected" : ""}>asc</option>
              <option value="desc" ${sort.direction === "desc" ? "selected" : ""}>desc</option>
            </select>
          </div>
          <div class="db-sort-divider"></div>
          <div class="db-section-title">Per Page</div>
          <div class="db-sort-row">
            <select class="db-select" data-db-page-size>
              <option value="25" ${dbBrowsePageSize === 25 ? "selected" : ""}>25 rows</option>
              <option value="50" ${dbBrowsePageSize === 50 ? "selected" : ""}>50 rows</option>
              <option value="100" ${dbBrowsePageSize === 100 ? "selected" : ""}>100 rows</option>
            </select>
          </div>
        </div>
      </div>
      ${renderDbResultGrid(
        dbBrowseResult.columns,
        dbBrowseResult.rows,
        "No matching rows",
        "Adjust the current filters or choose a different table."
      )}
      <div class="db-pagination">
        <button class="btn-ghost" type="button" data-db-page="prev" ${canGoBack ? "" : "disabled"}>← Prev</button>
        <span>Page ${dbBrowseResult.page} of ${pageCount}</span>
        <button class="btn-ghost" type="button" data-db-page="next" ${canGoForward ? "" : "disabled"}>Next →</button>
      </div>
    </section>
  `;
}

function renderDbQueryTab(): string {
  const queryResultMarkup =
    dbQueryRunning && !dbQueryResult
      ? `<div class="detail-empty"><div class="empty-state"><div class="spinner" style="margin-bottom:16px"></div><div class="empty-copy">Executing query\u2026</div></div></div>`
      : dbQueryError
        ? renderDbEmptyState("Query failed", dbQueryError)
        : dbQueryResult
          ? renderDbResultGrid(
            dbQueryResult.columns,
            dbQueryResult.rows,
            "Query returned no rows",
            "The statement ran successfully but there was nothing to display.",
            dbQueryResult.truncated ? "Showing the first 100 rows. Refine the query to inspect more data." : undefined
          )
          : renderDbEmptyState("Run a read-only query", "Custom SQL is read-only and limited to one statement.");

  return `
    <section class="db-panel">
      <div class="db-query-shell">
        <div class="db-query-header">
          <div class="db-section-title">Custom SQL ${renderHelpTip("Write and run a single read-only SQL statement against the Distill database. Results are limited to 100 rows. Use Ctrl/Cmd+Enter as a shortcut.", "Custom SQL")}</div>
          <div class="db-query-copy">Single read-only statement.</div>
        </div>
        <textarea
          class="db-query-editor"
          data-db-query-input
          placeholder="${escapeHtml(DB_QUERY_PLACEHOLDER)}"
        >${escapeHtml(dbQueryText)}</textarea>
        <div class="db-query-actions">
          <button class="btn-primary" type="button" data-db-run-query ${dbQueryRunning ? "disabled" : ""}>${dbQueryRunning ? '<span class="spinner spinner--small"></span> Running\u2026' : "Run Query"}</button>
          <span class="db-query-hint">Use Ctrl/Cmd + Enter to run the current query.</span>
        </div>
        ${dbQueryIsStale ? `<div class="db-inline-notice">Results may be stale after sync. Rerun the query.</div>` : ""}
      </div>
      ${queryResultMarkup}
    </section>
  `;
}

function renderDbWorkspace(): void {
  const root = document.querySelector<HTMLElement>("[data-session-detail]");
  if (!root) return;

  if (dbExplorerLoading && !dbExplorerSnapshot) {
    root.innerHTML = renderDbSkeletonWorkspace();
    return;
  }

  if (dbExplorerError) {
    root.innerHTML = renderDbEmptyState("Database unavailable", dbExplorerError);
    return;
  }

  if (!dbExplorerSnapshot?.databaseExists) {
    root.innerHTML = renderDbEmptyState(
      "No database yet",
      "Distill has not created a local SQLite database yet.",
      dbExplorerSnapshot
        ? `<div class="db-empty-code">${escapeHtml(dbExplorerSnapshot.databasePath)}</div>`
        : ""
    );
    return;
  }

  if (!dbSelectedTableName) {
    root.innerHTML = renderDbEmptyState("No tables available", "The current database does not expose any browseable tables.");
    return;
  }

  const table = findDbTableSummary(dbSelectedTableName);
  const visibleColumnCount = dbBrowseResult?.schemaColumns.filter((column) => !column.isHidden).length ?? 0;
  const rowCountLabel = dbBrowseResult
    ? `${formatDbRowCount(dbBrowseResult.totalRows)} rows`
    : dbBrowseLoading ? "Loading rows…" : "0 rows";
  const schemaStrip = dbBrowseResult
    ? dbBrowseResult.schemaColumns.map(renderDbSchemaChip).join("")
    : `<div class="db-schema-empty">Schema will appear once the selected table finishes loading.</div>`;

  root.innerHTML = `
    <div class="db-view fade-in">
      <div class="detail-toolbar">
        <span class="detail-title">DB Explorer ${renderHelpTip("A read-only browser over the Distill SQLite database. Inspect tables, filter rows, view schema, and run custom SQL queries. No data is modified.", "DB Explorer")}</span>
        <div class="detail-meta-secondary">
          <span class="badge ${table?.kind === "virtual" ? "badge-db-virtual" : "badge-db-table"}">${escapeHtml(table?.kind ?? "table")}</span>
          <span>${escapeHtml(dbSelectedTableName)}</span>
          <span>${escapeHtml(rowCountLabel)}</span>
          <span>${visibleColumnCount} visible cols</span>
          <span class="db-path-label">${escapeHtml(dbExplorerSnapshot.databasePath)}</span>
        </div>
      </div>
      <div class="db-subtabs">
        <button class="chip ${dbActiveTab === "browse" ? "active" : ""}" type="button" data-db-tab="browse">Browse</button>
        <button class="chip ${dbActiveTab === "query" ? "active" : ""}" type="button" data-db-tab="query">Query</button>
      </div>
      <section class="db-schema-section">
        <div class="db-section-title db-section-title--prominent">Schema ${renderHelpTip("Each chip shows a column in the selected table with its SQLite type, primary-key status, and nullability. Hidden columns are excluded from the browse grid.", "Schema")}</div>
        <div class="db-schema-strip">
          ${schemaStrip}
        </div>
      </section>
      ${dbActiveTab === "browse" ? renderDbBrowseTab() : renderDbQueryTab()}
    </div>
  `;

  bindDbViewControls();
}

async function loadDbExplorerSnapshot(): Promise<void> {
  const token = ++dbSnapshotRequestToken;
  dbExplorerLoading = true;
  dbExplorerError = null;

  if (activeView === "db") {
    renderCurrentView();
  }

  try {
    const snapshot = await window.distillApi.getDbExplorerSnapshot();
    if (token !== dbSnapshotRequestToken) {
      return;
    }

    dbExplorerSnapshot = snapshot;
    dbExplorerError = null;
    normalizeDbSelectedTable();

    if (!snapshot.databaseExists) {
      dbBrowseResult = null;
      dbBrowseError = null;
      dbSelectedRowKey = null;
      return;
    }
  } catch (error) {
    if (token !== dbSnapshotRequestToken) {
      return;
    }

    dbExplorerSnapshot = null;
    dbExplorerError = getErrorMessage(error);
    dbBrowseResult = null;
    dbBrowseError = null;
    dbSelectedRowKey = null;
    return;
  } finally {
    if (token === dbSnapshotRequestToken) {
      dbExplorerLoading = false;
      if (activeView === "db") {
        renderCurrentView();
      }
    }
  }

}

async function loadDbBrowseResult(): Promise<void> {
  if (!dbExplorerSnapshot?.databaseExists || !dbSelectedTableName) {
    return;
  }

  const token = ++dbBrowseRequestToken;
  dbBrowseLoading = true;
  dbBrowseError = null;
  dbBrowseResult = dbBrowseResult?.table.name === dbSelectedTableName ? dbBrowseResult : null;

  if (activeView === "db") {
    renderCurrentView();
  }

  try {
    const result = await window.distillApi.browseDbTable({
      tableName: dbSelectedTableName,
      filters: dbBrowseFilters,
      sort: dbBrowseSort,
      page: dbBrowsePage,
      pageSize: dbBrowsePageSize
    });

    if (token !== dbBrowseRequestToken) {
      return;
    }

    dbBrowseResult = result;
    dbBrowseError = null;
    dbBrowseFilters = result.appliedFilters;
    dbBrowseSort = result.sort;
    dbBrowsePage = result.page;
    dbBrowsePageSize = result.pageSize;
    dbSelectedRowKey = result.rows[0]?.key ?? null;
  } catch (error) {
    if (token !== dbBrowseRequestToken) {
      return;
    }

    dbBrowseResult = null;
    dbBrowseError = getErrorMessage(error);
    dbSelectedRowKey = null;
  } finally {
    if (token === dbBrowseRequestToken) {
      dbBrowseLoading = false;
      if (activeView === "db" && (dbBrowseResult !== null || dbBrowseError !== null)) {
        renderCurrentView();
      }
    }
  }
}

async function executeDbQuery(): Promise<void> {
  if (dbQueryRunning) {
    return;
  }

  const token = ++dbQueryRequestToken;
  dbQueryRunning = true;
  dbQueryError = null;
  dbQueryResult = null;
  dbQueryIsStale = false;
  dbSelectedRowKey = null;

  if (activeView === "db") {
    renderCurrentView();
  }

  try {
    const result = await window.distillApi.runDbQuery({
      sql: dbQueryText
    });

    if (token !== dbQueryRequestToken) {
      return;
    }

    dbQueryResult = result;
    dbQueryError = null;
    dbQueryIsStale = false;
    dbSelectedRowKey = result.rows[0]?.key ?? null;
  } catch (error) {
    if (token !== dbQueryRequestToken) {
      return;
    }

    dbQueryResult = null;
    dbQueryError = getErrorMessage(error);
    dbSelectedRowKey = null;
  } finally {
    if (token === dbQueryRequestToken) {
      dbQueryRunning = false;
      if (activeView === "db") {
        renderCurrentView();
      }
    }
  }
}

function ensureDbViewData(): void {
  if (!dbExplorerSnapshot && !dbExplorerLoading && !dbExplorerError) {
    void loadDbExplorerSnapshot();
    return;
  }

  if (
    dbExplorerSnapshot?.databaseExists
    && dbSelectedTableName
    && !dbBrowseResult
    && !dbBrowseLoading
    && !dbBrowseError
  ) {
    void loadDbBrowseResult();
  }
}

function refreshDbAfterSync(): void {
  dbQueryIsStale = Boolean(dbQueryResult);
  void loadDbExplorerSnapshot();
}

function bindDbViewControls(): void {
  const showInternal = document.querySelector<HTMLInputElement>("[data-db-show-internal]");
  if (showInternal) {
    showInternal.onchange = () => {
      dbShowInternalTables = showInternal.checked;
      normalizeDbSelectedTable();
      if (dbSelectedTableName) {
        dbBrowseFilters = [];
        dbBrowseSort = undefined;
        dbBrowsePage = 1;
        dbBrowseResult = null;
        dbBrowseError = null;
        void loadDbBrowseResult();
      }
      renderCurrentView();
    };
  }

  for (const btn of document.querySelectorAll<HTMLElement>("[data-db-table-name]")) {
    btn.onclick = () => {
      const tableName = btn.dataset.dbTableName;
      if (!tableName || tableName === dbSelectedTableName) {
        return;
      }

      dbSelectedTableName = tableName;
      dbBrowseFilters = [];
      dbBrowseSort = undefined;
      dbBrowsePage = 1;
      dbBrowseResult = null;
      dbBrowseError = null;
      dbSelectedRowKey = null;
      void loadDbBrowseResult();
    };
  }

  for (const btn of document.querySelectorAll<HTMLElement>("[data-db-tab]")) {
    btn.onclick = () => {
      const tab = btn.dataset.dbTab;
      if (tab !== "browse" && tab !== "query") {
        return;
      }

      dbActiveTab = tab;
      renderCurrentView();
    };
  }

  for (const btn of document.querySelectorAll<HTMLElement>("[data-db-row-key]")) {
    btn.onclick = () => {
      const rowKey = btn.dataset.dbRowKey;
      if (!rowKey) {
        return;
      }

      dbSelectedRowKey = rowKey;
      renderCurrentView();
    };

    btn.onkeydown = (event) => {
      if (event.key !== "Enter" && event.key !== " ") {
        return;
      }

      if (event.key === " ") {
        event.preventDefault();
      }

      btn.click();
    };
  }

  const visibleColumns = getDbVisibleColumns();

  for (const btn of document.querySelectorAll<HTMLElement>("[data-db-add-filter]")) {
    btn.onclick = () => {
      const filter = createDefaultDbFilter();
      if (!filter) {
        return;
      }

      dbBrowseFilters = [...dbBrowseFilters, filter];
      renderCurrentView();
    };
  }

  for (const btn of document.querySelectorAll<HTMLElement>("[data-db-remove-filter]")) {
    btn.onclick = () => {
      const index = Number(btn.dataset.dbRemoveFilter);
      if (!Number.isFinite(index)) {
        return;
      }

      dbBrowseFilters = dbBrowseFilters.filter((_filter, filterIndex) => filterIndex !== index);
      renderCurrentView();
    };
  }

  for (const select of document.querySelectorAll<HTMLSelectElement>("[data-db-filter-column]")) {
    select.onchange = () => {
      const index = Number(select.dataset.dbFilterColumn);
      const column = visibleColumns.find((entry) => entry.name === select.value);
      if (!Number.isFinite(index) || !column) {
        return;
      }

      const nextFilters = [...dbBrowseFilters];
      nextFilters[index] = {
        column: column.name,
        operator: defaultDbFilterOperator(column),
        value: ""
      };
      dbBrowseFilters = nextFilters;
      renderCurrentView();
    };
  }

  for (const select of document.querySelectorAll<HTMLSelectElement>("[data-db-filter-operator]")) {
    select.onchange = () => {
      const index = Number(select.dataset.dbFilterOperator);
      if (!Number.isFinite(index)) {
        return;
      }

      const nextFilters = [...dbBrowseFilters];
      const current = nextFilters[index];
      if (!current) {
        return;
      }

      nextFilters[index] = {
        ...current,
        operator: select.value as DbFilterOperator,
        value: dbOperatorNeedsValue(select.value as DbFilterOperator) ? current.value ?? "" : ""
      };
      dbBrowseFilters = nextFilters;
      renderCurrentView();
    };
  }

  for (const input of document.querySelectorAll<HTMLInputElement>("[data-db-filter-value]")) {
    input.oninput = () => {
      const index = Number(input.dataset.dbFilterValue);
      if (!Number.isFinite(index)) {
        return;
      }

      const nextFilters = [...dbBrowseFilters];
      const current = nextFilters[index];
      if (!current) {
        return;
      }

      nextFilters[index] = {
        ...current,
        value: input.value
      };
      dbBrowseFilters = nextFilters;
    };
  }

  const sortColumn = document.querySelector<HTMLSelectElement>("[data-db-sort-column]");
  if (sortColumn) {
    sortColumn.onchange = () => {
      dbBrowseSort = {
        column: sortColumn.value,
        direction: dbBrowseSort?.direction ?? dbBrowseResult?.sort.direction ?? "desc"
      };
    };
  }

  const sortDirection = document.querySelector<HTMLSelectElement>("[data-db-sort-direction]");
  if (sortDirection) {
    sortDirection.onchange = () => {
      dbBrowseSort = {
        column: dbBrowseSort?.column ?? dbBrowseResult?.sort.column ?? visibleColumns[0]?.name ?? "",
        direction: sortDirection.value === "asc" ? "asc" : "desc"
      };
    };
  }

  const pageSize = document.querySelector<HTMLSelectElement>("[data-db-page-size]");
  if (pageSize) {
    pageSize.onchange = () => {
      dbBrowsePageSize = Number(pageSize.value);
    };
  }

  const applyBrowse = document.querySelector<HTMLElement>("[data-db-apply-browse]");
  if (applyBrowse) {
    applyBrowse.onclick = () => {
      dbBrowsePage = 1;
      void loadDbBrowseResult();
    };
  }

  const resetBrowse = document.querySelector<HTMLElement>("[data-db-reset-browse]");
  if (resetBrowse) {
    resetBrowse.onclick = () => {
      dbBrowseFilters = [];
      dbBrowseSort = undefined;
      dbBrowsePage = 1;
      dbBrowsePageSize = 50;
      void loadDbBrowseResult();
    };
  }

  for (const btn of document.querySelectorAll<HTMLElement>("[data-db-page]")) {
    btn.onclick = () => {
      const direction = btn.dataset.dbPage;
      if (direction === "prev" && dbBrowsePage > 1) {
        dbBrowsePage -= 1;
        void loadDbBrowseResult();
      } else if (direction === "next" && dbBrowseResult) {
        const pageCount = getDbPageCount(dbBrowseResult.totalRows, dbBrowseResult.pageSize);
        if (dbBrowsePage < pageCount) {
          dbBrowsePage += 1;
          void loadDbBrowseResult();
        }
      }
    };
  }

  const queryInput = document.querySelector<HTMLTextAreaElement>("[data-db-query-input]");
  if (queryInput) {
    queryInput.oninput = () => {
      dbQueryText = queryInput.value;
    };

    queryInput.onkeydown = (event) => {
      if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
        event.preventDefault();
        if (dbQueryRunning) {
          return;
        }
        void executeDbQuery();
      }
    };
  }

  const runQuery = document.querySelector<HTMLElement>("[data-db-run-query]");
  if (runQuery) {
    runQuery.onclick = () => {
      if (dbQueryRunning) {
        return;
      }
      void executeDbQuery();
    };
  }
}

/* Sources */

function renderSource(source: DiscoveredSource): string {
  const dot = source.installStatus === "installed" ? "ok"
    : source.installStatus === "partial" ? "warn" : "miss";

  const checks = source.checks.map((check) => {
    const pillClass = check.exists ? "pill-ok" : "pill-miss";
    const pillText = check.exists ? "\u2713" : "\u2717";
    const count = typeof check.fileCount === "number" ? `${check.fileCount} files` : "";
    return `<div>
      <span class="pill ${pillClass}" ${tooltipAttrs(`${check.label}: ${check.exists ? "found" : "missing"} ${count}`.trim())}>${pillText}</span>
      ${escapeHtml(check.label)} <span style="color:var(--dim)">${escapeHtml(count)}</span>
    </div>`;
  }).join("");

  return `
    <div class="source-row" ${tooltipAttrs(`Source root: ${source.dataRoot ?? "not found"}`)}>
      <span class="status-dot ${dot}"></span>
      <span class="source-name">${escapeHtml(source.displayName)}</span>
      <span class="source-path">${escapeHtml(source.dataRoot ?? "not found")}</span>
    </div>
    <div class="source-checks">${checks}</div>
  `;
}

/* Session list item */

function workflowBadgeClass(workflowState: SessionWorkflowState): string {
  if (workflowState === "needs_review") return "badge-workflow-review";
  if (workflowState === "train_ready") return "badge-workflow-train";
  if (workflowState === "holdout_ready") return "badge-workflow-holdout";
  if (workflowState === "favorite") return "badge-workflow-favorite";
  return "";
}

function workflowBadgeLabel(workflowState: SessionWorkflowState): string | undefined {
  if (workflowState === "needs_review") return "review";
  if (workflowState === "train_ready") return "train";
  if (workflowState === "holdout_ready") return "holdout";
  if (workflowState === "favorite") return "favorite";
  return undefined;
}

function renderWorkflowBadge(
  item: Pick<SessionListItem, "workflowState" | "labels"> | Pick<SearchResult, "workflowState" | "labels">
): string {
  const label = workflowBadgeLabel(item.workflowState);
  if (!label) {
    return "";
  }

  const tooltip = item.labels.length ? `Labels: ${item.labels.join(", ")}` : label;
  return `<span class="badge ${workflowBadgeClass(item.workflowState)}" ${titleAttr(tooltip)}>${escapeHtml(label)}</span>`;
}

function matchesSessionLane(
  item: Pick<SessionListItem, "workflowState" | "labels"> | Pick<SearchResult, "workflowState" | "labels">,
  lane = activeSessionLane
): boolean {
  if (lane === "all") {
    return true;
  }

  if (lane === "favorite") {
    return item.labels.includes("favorite");
  }

  return item.workflowState === lane;
}

function filterSessionItems<T extends SessionListItem | SearchResult>(items: T[]): T[] {
  return items.filter((item) => matchesSessionLane(item));
}

function visibleSessionItems(report: DashboardData, query: string): Array<SessionListItem | SearchResult> {
  return query
    ? filterSessionItems(window.distillApi.searchSessions(query))
    : filterSessionItems(report.sessions);
}

function renderSessionLaneFilters(report: DashboardData): string {
  const lanes: Array<{ lane: SessionFilterLane; label: string }> = [
    { lane: "all", label: "All" },
    { lane: "needs_review", label: "Needs Review" },
    { lane: "train_ready", label: "Train Ready" },
    { lane: "holdout_ready", label: "Holdout Ready" },
    { lane: "favorite", label: "Favorites" }
  ];

  return `
    <div class="session-filter-bar">
      ${lanes.map(({ lane, label }) => {
        const count = lane === "all"
          ? report.sessions.length
          : report.sessions.filter((session) => matchesSessionLane(session, lane)).length;
        return `<button class="chip ${activeSessionLane === lane ? "active" : ""}" type="button" data-session-lane="${lane}">${escapeHtml(label)} <span class="session-filter-count">${count}</span></button>`;
      }).join("")}
    </div>
  `;
}

function renderSessionItem(session: SessionListItem): string {
  const metaTooltip = `${session.sourceKind} session, ${session.messageCount} messages${session.model ? `, model ${session.model}` : ""}${session.gitBranch ? `, branch ${session.gitBranch}` : ""}`;
  return `
    <div class="session-item" data-session-id="${session.id}" ${titleAttr(metaTooltip)}>
      <div class="session-item-title">${escapeHtml(session.title)}</div>
      <div class="session-item-meta">
        <span class="badge ${sourceBadgeClass(session.sourceKind)}">${sourceLabel(session.sourceKind)}</span>
        ${renderWorkflowBadge(session)}
        ${session.model ? `<span class="badge badge-model">${escapeHtml(session.model)}</span>` : ""}
        <span>${session.messageCount} msgs</span>
        <span>${timeAgo(session.updatedAt)}</span>
        ${session.gitBranch ? `<span>\u2387 ${escapeHtml(session.gitBranch)}</span>` : ""}
      </div>
      ${session.preview ? `<div class="session-item-preview">${escapeHtml(session.preview.slice(0, 140))}</div>` : ""}
    </div>
  `;
}

function renderSearchItem(result: SearchResult): string {
  return `
    <div class="session-item" data-session-id="${result.sessionId}">
      <div class="session-item-title">${escapeHtml(result.title)}</div>
      <div class="session-item-meta">
        <span class="badge ${sourceBadgeClass(result.sourceKind)}">${sourceLabel(result.sourceKind)}</span>
        ${renderWorkflowBadge(result)}
        <span>${timeAgo(result.updatedAt)}</span>
      </div>
      <div class="session-item-preview">${escapeHtml(result.snippet)}</div>
    </div>
  `;
}

function renderArtifact(artifact: SessionArtifact): string {
  const metaBits = [
    artifact.kind,
    artifact.mimeType,
    typeof artifact.messageOrdinal === "number" ? `msg #${artifact.messageOrdinal}` : undefined,
    typeof artifact.sourceLineNo === "number" ? `line ${artifact.sourceLineNo}` : undefined,
    artifact.createdAt ? timeAgo(artifact.createdAt) : undefined
  ].filter(Boolean).map((part) => `<span>${escapeHtml(String(part))}</span>`).join("");

  return `
    <details class="artifact-card">
      <summary>
        <div class="artifact-title">${escapeHtml(artifact.summary)}</div>
        <div class="artifact-meta">${metaBits}</div>
        <div class="artifact-preview">${escapeHtml(artifact.payloadPreview)}</div>
      </summary>
      <pre class="artifact-payload">${escapeHtml(artifact.payloadJson)}</pre>
    </details>
  `;
}

function renderSourceColorRow(sourceKind: string, displayName: string, color: string): string {
  return `
    <div class="color-picker-row">
      <span class="color-picker-label">${escapeHtml(displayName)}</span>
      <span class="color-picker-preview" style="background:${hexToRgba(color, 0.14)};color:${escapeHtml(color)}">${escapeHtml(displayName)}</span>
      <input type="color" class="color-picker-swatch" value="${escapeHtml(color)}" data-source-color="${escapeHtml(sourceKind)}" />
    </div>
  `;
}

function renderSettingsPanel(settings: AppSettingsSnapshot): string {
  const labels = settings.defaultLabels.map((label) => `<span class="chip chip-static">${escapeHtml(label)}</span>`).join("");
  const sources = settings.sourceKinds.map((source) =>
    `<div class="settings-row" ${tooltipAttrs(`${source} is part of the current local import pipeline.`)}><span>${escapeHtml(source)}</span><span class="settings-note">enabled</span></div>`
  ).join("");

  const colors = settings.sourceColors;
  const colorRows = [
    renderSourceColorRow("codex", "Codex", colors.codex ?? "#3dbf9a"),
    renderSourceColorRow("claude_code", "Claude", colors.claude_code ?? "#d4944a"),
    renderSourceColorRow("opencode", "OpenCode", colors.opencode ?? "#a88cd4")
  ].join("");

  return `
    <div class="settings-overlay ${isSettingsOpen ? "visible" : ""}" data-settings-overlay>
      <section class="settings-panel" aria-hidden="${isSettingsOpen ? "false" : "true"}">
        <div class="settings-header">
          <div>
            <div class="section-title">Settings</div>
            <div class="settings-subtitle">Source colors are editable. Other settings are read-only for now.</div>
          </div>
          <button class="btn-ghost" type="button" data-settings-close title="Close settings">\u2715</button>
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Storage</div>
          <div class="settings-code">${escapeHtml(settings.distillHome)}</div>
          <div class="settings-row" ${tooltipAttrs("SQLite database path used for imported sessions, messages, artifacts, and curation state.")}><span>Database</span><span class="settings-note">${escapeHtml(settings.databasePath)}</span></div>
          <div class="settings-row" ${tooltipAttrs("Whether DISTILL_HOME is explicitly set in the environment instead of using the default ~/.distill path.")}><span>DISTILL_HOME override</span><span class="settings-note">${settings.envOverrides.distillHome ? "on" : "off"}</span></div>
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Sources</div>
          ${sources}
          <div class="settings-row" ${tooltipAttrs("Local root used to discover Codex archived sessions and history files.")}><span>Codex root</span><span class="settings-note">${escapeHtml(settings.codexHome)}</span></div>
          <div class="settings-row" ${tooltipAttrs("Local root used to discover Claude Code project session files and history.")}><span>Claude root</span><span class="settings-note">${escapeHtml(settings.claudeHome)}</span></div>
          <div class="settings-row" ${tooltipAttrs("SQLite database path used to discover OpenCode sessions.")}><span>OpenCode DB</span><span class="settings-note">${escapeHtml(settings.opencodeDatabasePath)}</span></div>
          <div class="settings-row" ${tooltipAttrs("Whether OPENCODE_DB_PATH is explicitly set in the environment instead of using the default OpenCode database path.")}><span>OPENCODE_DB_PATH override</span><span class="settings-note">${settings.envOverrides.opencodeDbPath ? "on" : "off"}</span></div>
          <div class="settings-row" ${tooltipAttrs("OpenCode config directory used for runtime configuration.")}><span>OpenCode config</span><span class="settings-note">${escapeHtml(settings.opencodeConfigDir)}</span></div>
          <div class="settings-row" ${tooltipAttrs("OpenCode state directory used for prompt history and related local state.")}><span>OpenCode state</span><span class="settings-note">${escapeHtml(settings.opencodeStateDir)}</span></div>
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Source Colors</div>
          ${colorRows}
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Sync</div>
          <div class="settings-row" ${tooltipAttrs("How often Distill re-checks local source files while the app is open.")}><span>Background interval</span><span class="settings-note">every ${settings.backgroundSyncIntervalMinutes} min</span></div>
          <div class="settings-row" ${tooltipAttrs("You can force a refresh at any time with the sync button in the top bar.")}><span>Manual sync</span><span class="settings-note">top bar button</span></div>
        </div>

        <div class="settings-section">
          <div class="settings-section-title">Curation</div>
          <div class="settings-chip-row">${labels}</div>
        </div>
      </section>
    </div>
  `;
}

/* Session detail pane */

function renderSessionDetail(detail: SessionDetail | undefined): void {
  const root = document.querySelector<HTMLElement>("[data-session-detail]");
  if (!root) return;

  root.scrollTop = 0;

  if (!detail) {
    root.innerHTML = `
      <div class="detail-empty fade-in">
        <div class="empty-state">
          <div class="empty-title">Select a session</div>
          <div class="empty-copy">Choose a conversation from the left to inspect the transcript, labels, tags, and artifacts.</div>
        </div>
      </div>
    `;
    activeSessionId = null;
    return;
  }

  activeSessionId = detail.id;

  const defaultLabels = window.distillApi.getDefaultLabelNames();
  const activeLabels = new Set(detail.labels.map((label) => label.name));
  const labelChips = defaultLabels.map((name) =>
    `<button class="chip ${activeLabels.has(name) ? "active" : ""}" data-toggle-label="${escapeHtml(name)}" ${titleAttr(`${activeLabels.has(name) ? "Remove" : "Apply"} label "${name}"`)}>${escapeHtml(name)}</button>`
  ).join("");

  const tagChips = detail.tags.map((tag) =>
    `<span class="tag-chip"><span>#${escapeHtml(tag.name)}</span><button class="tag-remove" data-remove-tag-id="${tag.id}" title="Remove tag ${escapeHtml(tag.name)}">\u2715</button></span>`
  ).join("");

  const messages = detail.messages.map((msg) => {
    const roleClass = msg.role === "user" ? "msg-user"
      : msg.role === "assistant" ? "msg-assistant"
      : msg.role === "tool" ? "msg-tool" : "msg-system";
    return `
      <div class="msg ${roleClass} ${msg.messageKind === "meta" ? "msg-meta" : ""}">
        <div class="msg-header">
          <span class="role">${escapeHtml(msg.role)}</span>
          <span class="kind">${escapeHtml(msg.messageKind)}</span>
          <span>#${msg.ordinal}</span>
          <span>${timeAgo(msg.createdAt)}</span>
        </div>
        <div>${escapeHtml(msg.text)}</div>
      </div>
    `;
  }).join("");

  const artifacts = detail.artifacts.length
    ? `
      <section class="artifact-list">
        <div class="section-title">Artifacts</div>
        ${detail.artifacts.map(renderArtifact).join("")}
      </section>
    `
    : "";

  const projectionMetadataRows = [
    renderDetailContextRow("External session", detail.externalSessionId),
    renderDetailContextRow("Raw captures", detail.rawCaptureCount.toString()),
    detail.startedAt ? renderDetailContextRow("Started", formatDateTime(detail.startedAt)) : "",
    detail.sourceUrl ? renderDetailContextRow("Source URL", detail.sourceUrl) : ""
  ].filter(Boolean).join("");

  const projectionMetadataExtras = [
    detail.summary ? renderDetailContextFullRow("Summary", detail.summary, "copy") : "",
    Object.keys(detail.metadata).length > 0
      ? renderDetailContextFullRow("Provenance", JSON.stringify(detail.metadata, null, 2), "json")
      : ""
  ].filter(Boolean).join("");

  const projectionMetadata = projectionMetadataRows || projectionMetadataExtras
    ? `
      <section class="detail-context">
        <div class="section-title">Projection metadata</div>
        <div class="detail-context-grid">
          ${projectionMetadataRows}
          ${projectionMetadataExtras}
        </div>
      </section>
    `
    : "";

  root.innerHTML = `
    <div class="fade-in">
    <div class="detail-toolbar">
      <span class="detail-title">${escapeHtml(detail.title)}</span>
      <div class="dropdown" data-export-dropdown>
        <button class="btn btn-secondary" data-export-toggle ${tooltipAttrs("Export the approved train or holdout dataset. Sessions marked sensitive or exclude stay in review and are skipped.")}>\u2913 Export</button>
        <div class="dropdown-menu" data-export-menu>
          <button class="dropdown-item" data-export-dataset="train">Export train dataset</button>
          <button class="dropdown-item" data-export-dataset="holdout">Export holdout dataset</button>
        </div>
      </div>
      <div class="detail-meta-secondary">
        <span class="badge ${sourceBadgeClass(detail.sourceKind)}">${sourceLabel(detail.sourceKind)}</span>
        ${detail.model ? `<span>${escapeHtml(detail.model)}</span>` : ""}
        <span>${detail.messageCount} msgs</span>
        <span>${detail.artifactCount} artifacts ${renderHelpTip("Artifacts are tool calls, tool results, images, and files extracted from the conversation.", "Artifacts")}</span>
        ${detail.gitBranch ? `<span>\u2387 ${escapeHtml(detail.gitBranch)}</span>` : ""}
        ${detail.projectPath ? `<span>${escapeHtml(detail.projectPath)}</span>` : ""}
        <span>${timeAgo(detail.updatedAt)}</span>
      </div>
    </div>
    <div class="curation-bar">
      <div class="curation-group">
        <span class="curation-group-label">Labels</span>
        ${labelChips}
        ${renderHelpTip("Train, holdout, and exclude are mutually exclusive dataset labels. Sensitive and exclude send a session to review and block standard dataset export. Favorite is bookmark-only. Tags are free-form notes.", "Labels & Tags")}
      </div>
      <div class="curation-group">
        <span class="curation-group-label">Tags</span>
        ${tagChips}
        <form data-tag-form style="display:inline-flex;gap:6px;margin:0;align-items:center">
          <input class="tag-input" type="text" name="tagName" placeholder="Add tag..." />
        </form>
      </div>
    </div>
    ${projectionMetadata}
    ${artifacts}
    <div class="message-list">${messages}</div>
    </div>
  `;

  bindDetailCuration(detail.id);
  bindExportDropdown();
}

function refreshActiveSession(): void {
  if (activeSessionId === null) return;
  renderSessionDetail(window.distillApi.getSessionDetail(activeSessionId));
}

/* Event binding */

function bindDetailCuration(sessionId: number): void {
  for (const btn of document.querySelectorAll<HTMLElement>("[data-toggle-label]")) {
    btn.addEventListener("click", () => {
      const label = btn.dataset.toggleLabel;
      if (!label) return;
      window.distillApi.toggleSessionLabel(sessionId, label);
      refreshDashboard();
    });
  }

  for (const btn of document.querySelectorAll<HTMLElement>("[data-remove-tag-id]")) {
    btn.addEventListener("click", () => {
      const tagId = Number(btn.dataset.removeTagId);
      if (!Number.isFinite(tagId)) return;
      window.distillApi.removeSessionTag(sessionId, tagId);
      refreshDashboard();
    });
  }

  const form = document.querySelector<HTMLFormElement>("[data-tag-form]");
  if (form) {
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      const input = form.elements.namedItem("tagName");
      if (!(input instanceof HTMLInputElement)) return;
      const name = input.value.trim();
      if (!name) return;
      window.distillApi.addSessionTag(sessionId, name);
      input.value = "";
      refreshDashboard();
    });
  }
}

function bindSessionClicks(): void {
  const items = document.querySelectorAll<HTMLElement>("[data-session-id]");
  for (const item of items) {
    item.addEventListener("click", () => {
      const id = Number(item.dataset.sessionId);
      if (!Number.isFinite(id)) return;
      renderSessionDetail(window.distillApi.getSessionDetail(id));
      for (const other of items) other.classList.toggle("selected", other === item);
    });
  }
}

function bindSessionLaneFilters(): void {
  for (const btn of document.querySelectorAll<HTMLElement>("[data-session-lane]")) {
    btn.onclick = () => {
      const lane = btn.dataset.sessionLane;
      if (
        lane !== "all"
        && lane !== "needs_review"
        && lane !== "train_ready"
        && lane !== "holdout_ready"
        && lane !== "favorite"
      ) {
        return;
      }

      if (activeSessionLane === lane) {
        return;
      }

      activeSessionLane = lane;
      renderCurrentView();
    };
  }
}

function renderSessionList(report: DashboardData, items: Array<SessionListItem | SearchResult>): void {
  const container = document.querySelector<HTMLElement>("[data-sessions]");
  if (!container) return;

  const query = document.querySelector<HTMLInputElement>("[data-search-input]")?.value.trim() ?? "";
  const listHtml = query
    ? (items as SearchResult[]).map(renderSearchItem).join("")
    : (items as SessionListItem[]).map(renderSessionItem).join("");
  const emptyCopy = query
    ? "No sessions match the current search and workflow lane."
    : "No sessions are in this workflow lane yet.";
  container.innerHTML = `
    <div class="fade-in">
      ${renderSessionLaneFilters(report)}
      ${listHtml ? listHtml : `<div class="sidebar-empty">${escapeHtml(emptyCopy)}</div>`}
    </div>
  `;

  bindSessionClicks();
  bindSessionLaneFilters();
}

function bindSearch(report: DashboardData): void {
  const input = document.querySelector<HTMLInputElement>("[data-search-input]");
  if (!input) return;

  input.oninput = () => {
    const q = input.value.trim();
    const items = visibleSessionItems(report, q);
    renderSessionList(report, items);

    const first = items[0];
    const firstId = first ? ("id" in first ? first.id : first.sessionId) : undefined;
    renderSessionDetail(firstId !== undefined ? window.distillApi.getSessionDetail(firstId) : undefined);

    if (firstId !== undefined) {
      document.querySelector<HTMLElement>(`[data-session-id="${firstId}"]`)?.classList.add("selected");
    }

    const countEl = document.querySelector<HTMLElement>("[data-session-count]");
    if (countEl) countEl.textContent = q ? `${items.length} results` : `${items.length} sessions`;
  };
}

function bindExportDropdown(): void {
  const toggle = document.querySelector<HTMLElement>("[data-export-toggle]");
  const menu = document.querySelector<HTMLElement>("[data-export-menu]");
  if (!toggle || !menu) return;

  toggle.onclick = (e) => {
    e.stopPropagation();
    menu.classList.toggle("visible");
  };

  for (const btn of menu.querySelectorAll<HTMLElement>("[data-export-dataset]")) {
    btn.onclick = () => {
      const dataset = btn.dataset.exportDataset as DatasetExportTarget | undefined;
      if (dataset !== "train" && dataset !== "holdout") return;
      const report = window.distillApi.exportApprovedSessions(dataset);
      refreshLogsData(false);
      showExportToast(`Exported ${report.recordCount} ${report.dataset} \u2192 ${report.outputPath}`);
      menu.classList.remove("visible");
    };
  }

  if (!exportDropdownDismissBound) {
    document.addEventListener("click", (e) => {
      if (!(e.target instanceof Element)) return;
      if (!e.target.closest("[data-export-dropdown]")) {
        document.querySelector<HTMLElement>("[data-export-menu]")?.classList.remove("visible");
      }
    });
    exportDropdownDismissBound = true;
  }
}

function bindSyncButton(): void {
  const syncBtn = document.querySelector<HTMLElement>("[data-sync-now]");
  if (syncBtn) {
    const originalLabel = syncBtn.innerHTML;
    syncBtn.onclick = async () => {
      syncBtn.setAttribute("disabled", "true");
      syncBtn.innerHTML = '<span class="spinner spinner--small"></span> Syncing\u2026';
      try {
        const status = await window.distillApi.requestBackgroundSync();
        renderSyncStatus(status);
      } finally {
        syncBtn.removeAttribute("disabled");
        syncBtn.innerHTML = originalLabel;
      }
    };
  }
}

function bindSourcesToggle(): void {
  const toggle = document.querySelector<HTMLElement>("[data-sources-toggle]");
  const panel = document.querySelector<HTMLElement>("[data-sources]");
  if (!toggle || !panel) return;

  toggle.onclick = () => {
    toggle.classList.toggle("open");
    panel.classList.toggle("visible");
  };
}

function positionFloating(floatEl: HTMLElement, anchorRect: DOMRect, preferAbove = true): void {
  const gap = 8;
  const pad = 12;

  // Reset so we can measure
  floatEl.style.left = "0px";
  floatEl.style.top = "0px";
  const floatRect = floatEl.getBoundingClientRect();
  const fw = floatRect.width;
  const fh = floatRect.height;
  const vw = window.innerWidth;
  const vh = window.innerHeight;

  // Horizontal: center on anchor, clamp to viewport
  let left = anchorRect.left + anchorRect.width / 2 - fw / 2;
  left = Math.max(pad, Math.min(left, vw - fw - pad));

  // Vertical: prefer above, fall back to below
  let top: number;
  if (preferAbove && anchorRect.top - fh - gap >= pad) {
    top = anchorRect.top - fh - gap;
  } else if (anchorRect.bottom + fh + gap <= vh - pad) {
    top = anchorRect.bottom + gap;
  } else {
    // Last resort: whichever side has more room
    top = anchorRect.top - fh - gap >= 0
      ? Math.max(pad, anchorRect.top - fh - gap)
      : Math.min(vh - fh - pad, anchorRect.bottom + gap);
  }

  floatEl.style.left = `${left}px`;
  floatEl.style.top = `${top}px`;
}

function tooltipFloatEl(): HTMLElement | null {
  return document.querySelector<HTMLElement>("[data-tooltip-float]");
}

function helpPopoverEl(): HTMLElement | null {
  return document.querySelector<HTMLElement>("[data-help-popover]");
}

function clearTooltipShowTimeout(): void {
  if (!tooltipShowTimeout) return;
  clearTimeout(tooltipShowTimeout);
  tooltipShowTimeout = null;
}

function hideFloatingElement(el: HTMLElement | null): void {
  if (!el) return;
  el.classList.remove("visible");
}

function dismissTooltip(): void {
  clearTooltipShowTimeout();
  hideFloatingElement(tooltipFloatEl());
  activeTooltipAnchor = null;
}

function dismissHelpPopover(): void {
  hideFloatingElement(helpPopoverEl());
  activeHelpTipAnchor = null;
}

function dismissFloatingOverlays(): void {
  dismissTooltip();
  dismissHelpPopover();
}

function showFloatingElement(
  el: HTMLElement | null,
  anchor: HTMLElement,
  render: (surface: HTMLElement) => void
): void {
  if (!el || !anchor.isConnected) return;
  render(el);
  el.classList.add("visible");
  positionFloating(el, anchor.getBoundingClientRect(), true);
}

function bindFloatingOverlays(): void {
  if (floatingOverlaysBound) return;

  document.addEventListener("mouseenter", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const anchor = target.closest<HTMLElement>("[data-tooltip]");
    if (!anchor) return;

    dismissTooltip();
    tooltipShowTimeout = setTimeout(() => {
      const text = anchor.dataset.tooltip;
      if (!text || !anchor.isConnected) return;
      activeTooltipAnchor = anchor;
      showFloatingElement(tooltipFloatEl(), anchor, (surface) => {
        surface.textContent = text;
      });
    }, 180);
  }, true);

  document.addEventListener("mouseleave", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const anchor = target.closest<HTMLElement>("[data-tooltip]");
    if (!anchor) return;
    if (activeTooltipAnchor === anchor) {
      dismissTooltip();
      return;
    }
    clearTooltipShowTimeout();
  }, true);

  document.addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;

    const tip = target.closest<HTMLElement>("[data-help-tip]");
    if (tip) {
      event.preventDefault();
      event.stopPropagation();
      dismissTooltip();
      const text = tip.dataset.helpTip;
      if (!text) return;

      if (activeHelpTipAnchor === tip) {
        dismissHelpPopover();
        return;
      }

      activeHelpTipAnchor = tip;
      showFloatingElement(helpPopoverEl(), tip, (surface) => {
        const popoverTitle = tip.dataset.helpTitle || "Help";
        surface.innerHTML = `<div class="help-popover-title">${escapeHtml(popoverTitle)}</div><div>${escapeHtml(text)}</div>`;
      });
      return;
    }

    const popover = helpPopoverEl();
    if (popover && popover.contains(target)) return;
    dismissHelpPopover();
  }, true);

  document.addEventListener("scroll", dismissFloatingOverlays, true);
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") dismissFloatingOverlays();
  });
  window.addEventListener("resize", dismissFloatingOverlays);
  window.addEventListener("blur", dismissFloatingOverlays);

  floatingOverlaysBound = true;
}

function bindSourceColorPickers(): void {
  for (const input of document.querySelectorAll<HTMLInputElement>("[data-source-color]")) {
    input.oninput = () => {
      const sourceKind = input.dataset.sourceColor;
      if (!sourceKind) return;
      const colors = window.distillApi.setSourceColor(sourceKind, input.value);
      applySourceColors(colors);

      // Update the inline preview badge next to this picker
      const row = input.closest(".color-picker-row");
      const preview = row?.querySelector<HTMLElement>(".color-picker-preview");
      if (preview) {
        preview.style.background = hexToRgba(input.value, 0.14);
        preview.style.color = input.value;
      }
    };
  }
}

function bindSettingsPanel(): void {
  const openBtn = document.querySelector<HTMLElement>("[data-settings-open]");
  const closeBtn = document.querySelector<HTMLElement>("[data-settings-close]");
  const overlay = document.querySelector<HTMLElement>("[data-settings-overlay]");

  if (openBtn) {
    openBtn.onclick = () => {
      isSettingsOpen = true;
      renderCurrentView();
    };
  }

  if (closeBtn) {
    closeBtn.onclick = () => {
      isSettingsOpen = false;
      renderCurrentView();
    };
  }

  if (overlay) {
    overlay.onclick = (event) => {
      if (event.target === overlay) {
        isSettingsOpen = false;
        renderCurrentView();
      }
    };
  }

  bindSourceColorPickers();
}

function refreshDashboard(): void {
  dashboardData = window.distillApi.getDashboardData();
  if (activeView === "sessions") {
    renderCurrentView();
  }
}

function refreshLogsData(shouldRender = activeView === "logs"): void {
  logsPageData = window.distillApi.getLogsPageData();
  if (shouldRender) {
    renderCurrentView();
  }
}

function bindViewSwitch(): void {
  for (const btn of document.querySelectorAll<HTMLElement>("[data-view-target]")) {
    btn.onclick = () => {
      const target = btn.dataset.viewTarget;
      if (target !== "sessions" && target !== "db" && target !== "logs") return;
      if (activeView === target) return;
      activeView = target;
      if (activeView === "logs") {
        refreshLogsData(false);
      } else if (activeView === "db") {
        ensureDbViewData();
      }
      renderCurrentView();
    };
  }
}

function bindLogsControls(): void {
  const input = document.querySelector<HTMLInputElement>("[data-logs-search]");
  if (input) {
    input.oninput = () => {
      logsSearchQuery = input.value;
      if (logsPageData) {
        renderLogsListOnly(logsPageData);
      }
    };
  }

  for (const btn of document.querySelectorAll<HTMLElement>("[data-logs-filter]")) {
    btn.onclick = () => {
      const filter = btn.dataset.logsFilter;
      if (filter !== "all" && filter !== "sync" && filter !== "export" && filter !== "errors") return;
      activeLogsFilter = filter;
      for (const other of document.querySelectorAll<HTMLElement>("[data-logs-filter]")) {
        other.classList.toggle("active", other.dataset.logsFilter === filter);
      }
      if (logsPageData) {
        renderLogsListOnly(logsPageData);
      }
    };
  }
}

function bindBackgroundSync(): void {
  if (syncStatusUnsubscribe) {
    syncStatusUnsubscribe();
  }

  syncStatusUnsubscribe = window.distillApi.onBackgroundSyncStatus((status) => {
    renderSyncStatus(status);
    refreshLogsData(false);

    if (status.state === "completed" || status.state === "warning") {
      dashboardData = window.distillApi.getDashboardData();
      if (activeView === "db") {
        refreshDbAfterSync();
      }
    }

    if (activeView === "logs" || status.state === "completed" || status.state === "warning") {
      renderCurrentView();
    }
  });
}

/* Main render */

function renderSessionsView(report: DashboardData): void {
  const sources = document.querySelector<HTMLElement>("[data-sources]");
  const sessionsEl = document.querySelector<HTMLElement>("[data-sessions]");
  const statsEl = document.querySelector<HTMLElement>("[data-stats]");
  const countEl = document.querySelector<HTMLElement>("[data-session-count]");
  const scannedEl = document.querySelector<HTMLElement>("[data-scanned-at]");
  const onboarding = document.querySelector<HTMLElement>("[data-onboarding]");
  const sourcesToggle = document.querySelector<HTMLElement>("[data-sources-toggle]");

  if (!sources || !sessionsEl) return;

  sourcesToggle?.classList.remove("is-hidden");
  sources.classList.remove("is-hidden");

  const totalSessions = report.sessions.length;
  const totalMessages = report.sessions.reduce((sum, session) => sum + session.messageCount, 0);
  const sourceCount = report.doctor.sources.filter((source) => source.installStatus === "installed").length;

  if (statsEl) {
    statsEl.innerHTML = `
      <span><span class="stat-value">${totalSessions}</span> sessions</span>
      <span><span class="stat-value">${totalMessages.toLocaleString()}</span> messages</span>
      <span><span class="stat-value">${sourceCount}</span> sources</span>
    `;
  }

  if (countEl) countEl.textContent = `${totalSessions} sessions`;
  if (scannedEl) scannedEl.textContent = timeAgo(report.doctor.scannedAt);

  if (onboarding) {
    const needsOnboarding = totalSessions === 0;
    onboarding.classList.toggle("visible", needsOnboarding);
  }

  sources.innerHTML = report.doctor.sources.map(renderSource).join("");
  const query = document.querySelector<HTMLInputElement>("[data-search-input]")?.value.trim() ?? "";
  const items = visibleSessionItems(report, query);
  renderSessionList(report, items);
  bindSearch(report);
  bindSyncButton();
  bindSourcesToggle();
  bindFloatingOverlays();
  bindSettingsPanel();

  if (countEl) countEl.textContent = query ? `${items.length} results` : `${items.length} sessions`;

  const visibleSessionIds = new Set(
    items.map((item) => ("id" in item ? item.id : item.sessionId))
  );

  const preferredId =
    activeSessionId !== null
      && visibleSessionIds.has(activeSessionId)
      && window.distillApi.getSessionDetail(activeSessionId)
      ? activeSessionId
      : items[0] ? ("id" in items[0] ? items[0].id : items[0].sessionId) : undefined;

  if (preferredId !== undefined) {
    renderSessionDetail(window.distillApi.getSessionDetail(preferredId));
    document.querySelector<HTMLElement>(`[data-session-id="${preferredId}"]`)?.classList.add("selected");
  } else {
    renderSessionDetail(undefined);
  }
}

function renderCurrentView(): void {
  const app = document.querySelector<HTMLElement>("[data-app-shell]");
  const onboarding = document.querySelector<HTMLElement>("[data-onboarding]");
  const searchInput = document.querySelector<HTMLInputElement>("[data-search-input]");
  const sessionsEl = document.querySelector<HTMLElement>("[data-sessions]");
  const statsEl = document.querySelector<HTMLElement>("[data-stats]");
  const countEl = document.querySelector<HTMLElement>("[data-session-count]");
  const scannedEl = document.querySelector<HTMLElement>("[data-scanned-at]");
  const sourcesToggle = document.querySelector<HTMLElement>("[data-sources-toggle]");
  const sourcesPanel = document.querySelector<HTMLElement>("[data-sources]");
  const settingsRoot = document.querySelector<HTMLElement>("[data-settings-root]");
  const appSettings = window.distillApi.getAppSettings();

  if (document.body) {
    document.body.dataset.appView = activeView;
  }

  dismissFloatingOverlays();

  app?.classList.toggle("logs-mode", activeView === "logs");
  app?.classList.toggle("db-mode", activeView === "db");
  searchInput?.classList.toggle("is-hidden", activeView === "logs" || activeView === "db");

  for (const btn of document.querySelectorAll<HTMLElement>("[data-view-target]")) {
    btn.classList.toggle("active", btn.dataset.viewTarget === activeView);
  }

  if (activeView === "logs") {
    onboarding?.classList.remove("visible");
    if (sessionsEl) {
      sessionsEl.innerHTML = "";
    }
    if (countEl) {
      countEl.textContent = "Logs";
    }
    if (scannedEl) {
      scannedEl.textContent = "";
    }
    sourcesToggle?.classList.add("is-hidden");
    sourcesPanel?.classList.remove("visible");
    if (sourcesPanel) {
      sourcesPanel.innerHTML = "";
    }
    if (statsEl) {
      statsEl.innerHTML = "";
    }
    if (logsPageData) {
      renderLogsView(logsPageData);
    } else {
      refreshLogsData(false);
      renderLogsView(logsPageData ?? { entries: [], counts: { total: 0, errors: 0, running: 0 } });
    }
  } else if (activeView === "db") {
    onboarding?.classList.remove("visible");
    if (statsEl) {
      statsEl.innerHTML = "";
    }
    ensureDbViewData();
    renderDbSidebar();
    renderDbWorkspace();
  } else if (dashboardData) {
    renderSessionsView(dashboardData);
  }

  if (settingsRoot) {
    settingsRoot.innerHTML = renderSettingsPanel(appSettings);
  }
  applySourceColors(appSettings.sourceColors);
  bindSyncButton();
  bindViewSwitch();
  bindFloatingOverlays();
  bindSettingsPanel();
}

document.addEventListener("DOMContentLoaded", () => {
  dashboardData = window.distillApi.getDashboardData();
  logsPageData = window.distillApi.getLogsPageData();
  renderCurrentView();
  bindBackgroundSync();
  window.distillApi.getBackgroundSyncStatus().then(renderSyncStatus).catch(() => {
    renderSyncStatus({
      state: "failed",
      discoveredCaptures: 0,
      importedCaptures: 0,
      skippedCaptures: 0,
      failedCaptures: 0,
      summary: "Sync status unavailable",
      errorText: "Sync status unavailable"
    });
  });
});

export {};
