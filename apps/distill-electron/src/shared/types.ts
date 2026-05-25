export type SourceKind = "codex" | "claude_code" | "opencode";

export type InstallStatus = "installed" | "not_found" | "partial";

export type CaptureStatus = "captured" | "failed_parse" | "normalized";

export type CaptureContentRef =
  | {
    kind: "inline";
    mediaType: string;
    text: string;
    sha256: string;
    byteSize: number;
  }
  | {
    kind: "blob";
    mediaType: string;
    blobPath: string;
    sha256: string;
    byteSize: number;
  };

export type SourcePathCheck = {
  label: string;
  path: string;
  exists: boolean;
  fileCount?: number;
};

export type DiscoveredSource = {
  kind: SourceKind;
  displayName: string;
  executablePath?: string;
  dataRoot?: string;
  installStatus: InstallStatus;
  checks: SourcePathCheck[];
  metadata: Record<string, unknown>;
};

export type DoctorReport = {
  scannedAt: string;
  sources: DiscoveredSource[];
};

export type DiscoveredCapture = {
  sourceKind: SourceKind;
  captureKind: string;
  sourcePath: string;
  externalSessionId?: string;
  sourceModifiedAt?: string;
  sourceSizeBytes?: number;
  metadata: Record<string, unknown>;
};

type ImportedCaptureBase = {
  sourcePath: string;
  externalSessionId?: string;
};

export type ImportedCapture =
  | (ImportedCaptureBase & {
    rawSha256: string;
    status: "imported";
    skipped?: false;
    errorText?: undefined;
  })
  | (ImportedCaptureBase & {
    rawSha256?: string;
    status: "failed";
    skipped?: false;
    errorText: string;
  })
  | (ImportedCaptureBase & {
    rawSha256: string;
    status: "skipped";
    skipped: true;
    errorText?: undefined;
  });

export type ImportedCaptureStatus = ImportedCapture["status"];

export type ParsedCaptureRecord = {
  lineNo: number;
  recordType: string;
  recordTimestamp?: string;
  providerMessageId?: string;
  parentProviderMessageId?: string;
  role?: string;
  isMeta: boolean;
  contentText?: string;
  contentJson: Record<string, unknown>;
  metadata: Record<string, unknown>;
};

export type NormalizedSession = {
  sourceKind: SourceKind;
  externalSessionId: string;
  title?: string;
  projectPath?: string;
  sourceUrl?: string;
  model?: string;
  modelProvider?: string;
  cliVersion?: string;
  gitBranch?: string;
  startedAt?: string;
  updatedAt?: string;
  summary?: string;
  metadata: Record<string, unknown>;
};

export type NormalizedMessage = {
  sourceLineNo: number;
  externalMessageId?: string;
  parentExternalMessageId?: string;
  role: "user" | "assistant" | "system" | "tool";
  text: string;
  createdAt?: string;
  messageKind: "text" | "meta";
  metadata: Record<string, unknown>;
};

export type NormalizedArtifact = {
  sourceLineNo: number;
  externalMessageId?: string;
  kind: "image" | "file" | "tool_call" | "tool_result" | "raw_json";
  mimeType?: string;
  payload: Record<string, unknown>;
};

export type ParsedCapture = {
  session: NormalizedSession;
  messages: NormalizedMessage[];
  artifacts: NormalizedArtifact[];
  rawRecords: ParsedCaptureRecord[];
};

export type ImportReport = {
  importedAt: string;
  databasePath: string;
  distillHome: string;
  sourceSummaries: ImportSourceSummary[];
  failedEntries: ImportFailureEntry[];
  captures: ImportedCapture[];
};

export type ImportSourceSummary = {
  kind: SourceKind;
  discoveredCaptures: number;
  importedCaptures: number;
  skippedCaptures: number;
  failedCaptures: number;
};

export type ImportFailureEntry = {
  sourceKind: SourceKind;
  sourcePath: string;
  errorText: string;
};

export type DatasetExportTarget = "train" | "holdout";

export type SessionWorkflowState =
  | "needs_review"
  | "train_ready"
  | "holdout_ready"
  | "favorite"
  | "neutral";

export type ExportReport = {
  exportedAt: string;
  dataset: DatasetExportTarget;
  outputPath: string;
  recordCount: number;
};

// Export records intentionally keep snake_case field names to match the
// external JSON schema consumed by Python/ML tooling. Renaming any of these
// fields requires coordinated updates across the export/ingest pipeline and
// downstream consumers.
export type ExportMessageRecord = {
  ordinal: number;
  role: string;
  text: string;
  created_at: string | null;
  message_kind: "text" | "meta";
  metadata: Record<string, unknown>;
};

export type ExportSessionRecord = {
  exported_at: string;
  source: SourceKind;
  external_session_id: string;
  title: string | null;
  project_path: string | null;
  updated_at: string | null;
  started_at: string | null;
  source_url: string | null;
  model: string | null;
  git_branch: string | null;
  summary: string | null;
  metadata: Record<string, unknown>;
  labels: string[];
  tags: string[];
  messages: ExportMessageRecord[];
  turn_pairs: Array<{
    user: string;
    assistant: string;
  }>;
};

export type BackgroundSyncStatus = {
  state: "idle" | "queued" | "running" | "warning" | "completed" | "failed";
  jobId?: number;
  reason?: string;
  startedAt?: string;
  finishedAt?: string;
  discoveredCaptures: number;
  importedCaptures: number;
  skippedCaptures: number;
  failedCaptures: number;
  summary: string;
  errorText?: string;
  sourceSummaries?: ImportSourceSummary[];
  failedEntries?: ImportFailureEntry[];
};

export type AppView = "sessions" | "db" | "logs";

export type LogEntryKind = "sync" | "export";

export type LogEntryStatus = "queued" | "running" | "warning" | "completed" | "failed";

export type LogEntryLevel = "info" | "error";

export type LogEntry = {
  id: string;
  kind: LogEntryKind;
  status: LogEntryStatus;
  level: LogEntryLevel;
  title: string;
  summary: string;
  createdAt: string;
  updatedAt?: string;
  sourceLabel?: string;
  metrics?: {
    discoveredCaptures?: number;
    importedCaptures?: number;
    skippedCaptures?: number;
    failedCaptures?: number;
    recordCount?: number;
  };
  details?: {
    reason?: string;
    outputPath?: string;
    dataset?: DatasetExportTarget;
    sourceSummaries?: ImportSourceSummary[];
    failedEntries?: ImportFailureEntry[];
  };
  rawJson: string;
};

export type LogsPageData = {
  entries: LogEntry[];
  counts: {
    total: number;
    errors: number;
    running: number;
  };
  lastSyncStatus?: BackgroundSyncStatus;
};

export type SourceColors = Record<string, string>;

export type AppSettingsSnapshot = {
  distillHome: string;
  databasePath: string;
  codexHome: string;
  claudeHome: string;
  opencodeDatabasePath: string;
  opencodeConfigDir: string;
  opencodeStateDir: string;
  sourceKinds: SourceKind[];
  defaultLabels: string[];
  backgroundSyncIntervalMinutes: number;
  envOverrides: {
    distillHome: boolean;
    codexHome: boolean;
    claudeHome: boolean;
    opencodeDbPath: boolean;
    opencodeConfigDir: boolean;
    opencodeStateDir: boolean;
  };
  sourceColors: SourceColors;
};

export type DbTableKind = "table" | "virtual";

export type DbColumnFilterKind = "text" | "numeric" | "date" | "other";

export type DbResultValueKind = "null" | "number" | "text" | "blob";

export type DbFilterOperator =
  | "contains"
  | "equals"
  | "not_equals"
  | "starts_with"
  | "ends_with"
  | "eq"
  | "neq"
  | "gt"
  | "gte"
  | "lt"
  | "lte"
  | "is_null"
  | "is_not_null";

export type DbSortDirection = "asc" | "desc";

export type DbTableSummary = {
  name: string;
  kind: DbTableKind;
  isCore: boolean;
};

export type DbColumnInfo = {
  name: string;
  type?: string;
  filterKind: DbColumnFilterKind;
  isNullable: boolean;
  isPrimaryKey: boolean;
  isHidden: boolean;
  primaryKeyOrdinal?: number;
  defaultValue?: string;
};

export type DbForeignKeyInfo = {
  id: number;
  seq: number;
  table: string;
  from: string;
  to?: string;
  onUpdate: string;
  onDelete: string;
  match: string;
};

export type DbResultColumn = {
  name: string;
  sourceColumn?: string;
  table?: string;
  database?: string;
  type?: string;
};

export type DbCellValue = {
  kind: DbResultValueKind;
  preview: string;
  detail: string;
  previewTruncated: boolean;
  detailTruncated: boolean;
  byteLength?: number;
};

export type DbResultRow = {
  key: string;
  cells: DbCellValue[];
};

export type DbSort = {
  column: string;
  direction: DbSortDirection;
};

export type DbRowCount = number | bigint;

export type DbFilter = {
  column: string;
  operator: DbFilterOperator;
  value?: string;
};

export type DbExplorerSnapshot = {
  databasePath: string;
  databaseExists: boolean;
  coreTables: DbTableSummary[];
  advancedTables: DbTableSummary[];
  defaultTableName?: string;
};

export type DbBrowseRequest = {
  tableName: string;
  filters: DbFilter[];
  sort?: DbSort;
  page: number;
  pageSize: number;
};

export type DbBrowseResult = {
  databasePath: string;
  table: DbTableSummary;
  schemaColumns: DbColumnInfo[];
  foreignKeys: DbForeignKeyInfo[];
  appliedFilters: DbFilter[];
  sort: DbSort;
  page: number;
  pageSize: number;
  totalRows: DbRowCount;
  columns: DbResultColumn[];
  rows: DbResultRow[];
};

export type DbQueryRequest = {
  sql: string;
};

export type DbQueryResult = {
  databasePath: string;
  executedSql: string;
  durationMs: number;
  columns: DbResultColumn[];
  rows: DbResultRow[];
  rowCount: number;
  truncated: boolean;
};

export type SessionListItem = {
  id: number;
  sourceKind: SourceKind;
  title: string;
  projectPath?: string;
  updatedAt?: string;
  messageCount: number;
  model?: string;
  gitBranch?: string;
  labels: string[];
  workflowState: SessionWorkflowState;
  preview?: string;
};

export type DashboardData = {
  doctor: DoctorReport;
  sessions: SessionListItem[];
};

export type SearchResult = {
  sessionId: number;
  sourceKind: SourceKind;
  title: string;
  projectPath?: string;
  updatedAt?: string;
  labels: string[];
  workflowState: SessionWorkflowState;
  snippet: string;
};

export type SessionTag = {
  id: number;
  name: string;
  kind: string;
  origin: string;
};

export type SessionLabel = {
  id: number;
  name: string;
  scope: string;
  origin: string;
};

export type SessionDetailMessage = {
  id: number;
  ordinal: number;
  role: string;
  text: string;
  createdAt?: string;
  messageKind: "text" | "meta";
};

export type SessionArtifact = {
  id: number;
  kind: string;
  mimeType?: string;
  sourceLineNo?: number;
  messageOrdinal?: number;
  messageRole?: string;
  createdAt?: string;
  summary: string;
  payloadPreview: string;
  payloadJson: string;
};

export type SessionDetail = {
  id: number;
  sourceKind: SourceKind;
  externalSessionId: string;
  title: string;
  projectPath?: string;
  startedAt?: string;
  updatedAt?: string;
  sourceUrl?: string;
  messageCount: number;
  rawCaptureCount: number;
  model?: string;
  gitBranch?: string;
  summary?: string;
  preview?: string;
  metadata: Record<string, unknown>;
  artifactCount: number;
  tags: SessionTag[];
  labels: SessionLabel[];
  artifacts: SessionArtifact[];
  messages: SessionDetailMessage[];
};
