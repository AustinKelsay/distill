/**
 * Shared typed shapes for the Distill desktop renderer bridge.
 * These mirror Library/host JSON results without granting storage authority.
 */

/** Stable Session Identity for UI display. */
export type SessionIdentity = {
  source_kind: string;
  external_session_id: string;
};

/** Caller-facing Source observation. */
export type SourceSummary = {
  kind: string;
  display_name: string;
  data_root: string;
  parser_id: string;
  parser_version: string;
};

/** Fixture ingest counters shown as the sync result. */
export type IngestReport = {
  accepted_captures: number;
  skipped_duplicates: number;
  successful_attempts: number;
  failed_attempts: number;
  capture_ids: number[];
  session_identities: SessionIdentity[];
};

/** Compact session list row. */
export type SessionSummary = {
  id: number;
  source_kind: string;
  external_session_id: string;
  title: string | null;
  accepted_capture_count: number;
  normalization_attempt_count: number;
  successful_projection_generation: number;
};

/** Canonical workflow state derived from manual labels. */
export type WorkflowState =
  "needs_review" | "train_ready" | "holdout_ready" | "favorite" | "neutral";

/** Session filter lane. */
export type WorkflowLane =
  "all" | "needs_review" | "train_ready" | "holdout_ready" | "favorites";

/** Manual tag read-model entry. */
export type SessionTag = {
  id: number;
  name: string;
  kind: string;
  origin: string;
};

/** Manual label read-model entry. */
export type SessionLabel = {
  id: number;
  name: string;
  scope: string;
  origin: string;
};

/** Projected transcript message. */
export type ProjectedMessage = {
  id: number;
  ordinal: number;
  role: string;
  message_kind: string;
  text: string;
};

/** Session Projection detail for the first-run result. */
export type SessionDetail = {
  summary: SessionSummary;
  messages: ProjectedMessage[];
  artifacts: Array<{
    id: number;
    artifact_type: string;
    message_id: number | null;
    capture_fact_id: number | null;
    text_preview: string | null;
  }>;
  metadata_json: string;
  project_path?: string | null;
  source_url?: string | null;
  projection_summary?: string | null;
  started_at?: string | null;
  updated_at?: string | null;
  raw_capture_count?: number;
  tags?: SessionTag[];
  labels?: SessionLabel[];
  workflow_state?: WorkflowState;
  next_message_cursor?: string | null;
  next_artifact_cursor?: string | null;
};

/** Request for a typed current-projection session list/search page. */
export type SessionListRequest = {
  query?: string | null;
  lane: WorkflowLane;
  limit: number;
  cursor?: string | null;
};

/** One current-projection session list/search row. */
export type SessionListItem = {
  id: number;
  source_kind: string;
  external_session_id: string;
  title: string;
  project_path: string | null;
  updated_at: string | null;
  preview: string | null;
  message_count: number;
  accepted_capture_count: number;
  normalization_attempt_count: number;
  successful_projection_generation: number;
  labels: SessionLabel[];
  tags: SessionTag[];
  workflow_state: WorkflowState;
};

/** Page of current-projection session rows. */
export type SessionListPage = {
  items: SessionListItem[];
  next_cursor: string | null;
};

/** Request for a bounded current-projection session detail page. */
export type SessionDetailRequest = {
  source_kind: string;
  external_session_id: string;
  message_limit: number;
  artifact_limit: number;
  message_cursor?: string | null;
  artifact_cursor?: string | null;
};

/** Session Identity plus a tag or label name for a curation mutation. */
export type SessionCurationRequest = {
  source_kind: string;
  external_session_id: string;
  name: string;
};

/** Result of a manual tag or label mutation with the current curation snapshot. */
export type CurationMutationResult = {
  changed: boolean;
  identity: SessionIdentity;
  tags: SessionTag[];
  labels: SessionLabel[];
  workflow_state: WorkflowState;
};

/** Closed export dataset targets owned by the Library. */
export type ExportDataset = "train" | "holdout";

/** Why a labeled session was omitted from a dataset export. */
export type ExportOmissionReason = "exclude" | "sensitive" | "conflicting_dataset_labels";

/** One session omitted from an eligibility snapshot. */
export type ExportOmission = {
  identity: SessionIdentity;
  reason: ExportOmissionReason;
};

/** Side-effect-free export eligibility preview from the Library. */
export type ExportPreview = {
  dataset: ExportDataset;
  format_id: string;
  eligible: SessionIdentity[];
  omitted: ExportOmission[];
};

/** Typed export publication progress events. */
export type ExportProgress =
  | { type: "preparing"; export_id: number }
  | {
      type: "writing";
      export_id: number;
      records_written: number;
      record_total: number;
    }
  | { type: "committed"; export_id: number }
  | { type: "renamed"; export_id: number }
  | { type: "published"; export_id: number };

/** Terminal export publication status. */
export type ExportStatus =
  "preparing" | "committed" | "published" | "failed_publish" | "cancelled";

/** Terminal result of an export publish call. */
export type ExportResult = {
  export_id: number;
  dataset: ExportDataset;
  format_id: string;
  status: ExportStatus;
  output_path: string | null;
  sha256: string | null;
  byte_size: number | null;
  record_count: number;
  eligible: SessionIdentity[];
  omitted: ExportOmission[];
  error_class: string | null;
  error_message: string | null;
};

/** Explicit export panel lifecycle for preview/publish callers. */
export type ExportUiStatus = "idle" | "running" | "success" | "error" | "cancelled";

/** Typed Library health issue with redacted summary. */
export type HealthIssue = {
  code: string;
  severity: string;
  category: string;
  summary: string;
};

/** Safe open reconciliation counts. */
export type OpenReconciliation = {
  removed_staging_partials: number;
  classified_incomplete_exports?: number;
  removed_export_temp_files?: number;
};

/** Library health report. */
export type HealthReport = {
  ok: boolean;
  schema_status: string;
  content_status: string;
  fts_status: string;
  staging_status: string;
  orphan_status: string;
  incomplete_status: string;
  /** Sync/operations status: `ok`, `active`, or `failed`. */
  operations_status: string;
  issues: HealthIssue[];
  open_reconciliation: OpenReconciliation;
};

/** Per-Source preference. */
export type SourcePreference = {
  kind: string;
  enabled: boolean;
  configured_root: string | null;
  display_name: string | null;
  data_root: string | null;
};

/** Typed Sync Run progress event. */
export type SyncProgress =
  | { type: "run_queued"; sync_run_id: number }
  | { type: "run_started"; sync_run_id: number }
  | { type: "source_started"; sync_run_id: number; source_kind: string }
  | { type: "source_finished"; sync_run_id: number; source_kind: string; status: string }
  | {
      type: "candidate_started";
      sync_run_id: number;
      source_kind: string;
      candidate_id: string;
    }
  | {
      type: "candidate_finished";
      sync_run_id: number;
      source_kind: string;
      candidate_id: string;
      outcome: string;
    };

/** Sync Run summary. */
export type SyncRunSummary = {
  id: number;
  status: string;
  cancel_requested: boolean;
  accepted_captures: number;
  skipped_duplicates: number;
  successful_attempts: number;
  failed_attempts: number;
  error_class: string | null;
  error_message: string | null;
  warning_details?: string[];
  sources: Array<{
    source_kind: string;
    status: string;
    accepted_captures: number;
    skipped_duplicates: number;
    successful_attempts: number;
    failed_attempts: number;
    error_class: string | null;
    error_message: string | null;
  }>;
};

/** Terminal Sync Run result. */
export type SyncRunResult = {
  run: SyncRunSummary;
  session_identities: SessionIdentity[];
};

/** Request for a paged Activity Event listing. */
export type ActivityListRequest = {
  limit: number;
  cursor?: string | null;
};

/** One append-only Activity Event with redacted payload JSON. */
export type ActivityEvent = {
  id: number;
  event_type: string;
  occurred_at: string;
  source_kind: string | null;
  session_id: number | null;
  capture_id: number | null;
  attempt_id: number | null;
  payload_json: unknown;
};

/** Page of Activity Events. */
export type ActivityListPage = {
  items: ActivityEvent[];
  next_cursor: string | null;
};

/** Request for a paged Operations diagnostics view. */
export type OperationsRequest = {
  sync_limit: number;
  export_limit: number;
  sync_cursor?: string | null;
  export_cursor?: string | null;
};

/** Safe export lifecycle summary without filesystem paths. */
export type ExportLifecycleSummary = {
  id: number;
  dataset: string;
  format_id: string;
  status: string;
  created_at: string;
  updated_at: string;
  sha256: string | null;
  byte_size: number | null;
  record_count: number;
  error_class: string | null;
  error_message: string | null;
};

/** Operations diagnostics page over Sync Runs and exports. */
export type OperationsPage = {
  operations_status: string;
  sync_runs: SyncRunSummary[];
  next_sync_cursor: string | null;
  exports: ExportLifecycleSummary[];
  next_export_cursor: string | null;
};

/** Named repair action count. */
export type RepairAction = {
  name: string;
  count: number;
};

/** Explicit Library repair result. */
export type RepairReport = {
  actions: RepairAction[];
  health_after: HealthReport;
};

/** Combined first-run Fixture journey result. */
export type FixtureJourneyResult = {
  source: SourceSummary;
  sync: IngestReport;
  session: SessionDetail | null;
  health: HealthReport;
};

/** Typed progress phases emitted by the host. */
export type FixtureJourneyPhase =
  "detecting_source" | "syncing_captures" | "loading_session" | "checking_health";

/** Typed host/Library failure surfaced to the renderer. */
export type HostError = {
  code: string;
  message: string;
};

/** Inputs collected by the first-run form. */
export type FixtureJourneyInput = {
  home: string;
  fixtureRoot: string;
};

/** Typed redacted skip/loss entry from a legacy Electron import. */
export type LegacyImportSkip = {
  category: string;
  reason: string;
  legacy_kind?: string | null;
};

/** Aggregate counters for a legacy Electron import. */
export type LegacyImportCounts = {
  sources: number;
  captures: number;
  captures_skipped: number;
  attempts: number;
  facts: number;
  sessions: number;
  messages: number;
  artifacts: number;
  tags: number;
  tag_assignments: number;
  labels: number;
  label_assignments: number;
  activity_events: number;
  exports: number;
  exports_skipped: number;
};

/** Typed redacted legacy Electron import report. */
export type LegacyImportReport = {
  ok: boolean;
  reused_prior_import: boolean;
  source_fingerprint: string;
  source_db_sha256: string;
  content_fingerprint: string;
  counts: LegacyImportCounts;
  skips: LegacyImportSkip[];
};

/** Explicit UI lifecycle for the first-run migration panel. */
export type MigrationUiStatus =
  "idle" | "loading" | "success" | "warning" | "error" | "cancelled";

/**
 * Explicit Distill bridge. The renderer never reaches process, filesystem,
 * SQLite, or shell APIs directly.
 */
export type DistillBridge = {
  /**
   * Run the Fixture journey through the privileged host.
   * @param input - chosen Distill home and Fixture root
   */
  runFixtureJourney(input: FixtureJourneyInput): Promise<FixtureJourneyResult>;
  /**
   * Load typed Library health for a Distill home.
   * @param home - Distill home path
   */
  health(home: string): Promise<HealthReport>;
  /**
   * Import a legacy Electron Distill home into a native Distill home.
   * @param home - destination native Distill home
   * @param sourceHome - legacy Electron Distill home (read-only)
   */
  importLegacy(home: string, sourceHome: string): Promise<LegacyImportReport>;
  /**
   * Explicit Library repair after user confirmation.
   * @param home - Distill home path
   * @param confirm - must be true to authorize destructive repair
   */
  repair(home: string, confirm: boolean): Promise<RepairReport>;
  /**
   * List Source preferences.
   * @param home - Distill home path
   */
  listSources(home: string): Promise<SourcePreference[]>;
  /**
   * Upsert Source preference.
   */
  setSourcePreference(
    home: string,
    kind: string,
    enabled: boolean,
    configuredRoot?: string | null,
  ): Promise<SourcePreference>;
  /**
   * Start a Sync Run.
   */
  startSync(home: string, sourceKinds?: string[]): Promise<SyncRunResult>;
  /**
   * Load Sync Run status.
   */
  syncStatus(home: string, syncRunId?: number | null): Promise<SyncRunSummary>;
  /**
   * Request Sync Run cancellation.
   */
  cancelSync(home: string, syncRunId: number): Promise<SyncRunSummary>;
  /** List/search current-projection sessions. */
  listSessions(home: string, request: SessionListRequest): Promise<SessionListPage>;
  /** Load bounded current-projection session detail. */
  sessionDetail(
    home: string,
    request: SessionDetailRequest,
  ): Promise<SessionDetail | null>;
  /** Add a manual tag by Session Identity. */
  addSessionTag(
    home: string,
    request: SessionCurationRequest,
  ): Promise<CurationMutationResult>;
  /** Remove a manual tag by Session Identity. */
  removeSessionTag(
    home: string,
    request: SessionCurationRequest,
  ): Promise<CurationMutationResult>;
  /** Toggle a catalog label by Session Identity. */
  toggleSessionLabel(
    home: string,
    request: SessionCurationRequest,
  ): Promise<CurationMutationResult>;
  /**
   * Preview export eligibility for a closed dataset target.
   * @param home - Distill home path
   * @param dataset - `train` or `holdout`
   */
  previewExport(home: string, dataset: ExportDataset): Promise<ExportPreview>;
  /**
   * Publish a recoverable Library-owned export for a closed dataset target.
   * @param home - Distill home path
   * @param dataset - `train` or `holdout`
   */
  publishExport(home: string, dataset: ExportDataset): Promise<ExportResult>;
  /** Request cancellation at the next safe export checkpoint. */
  cancelExport(home: string, dataset: ExportDataset): Promise<boolean>;
  /** List append-only Activity Events with cursor paging. */
  listActivity(home: string, request: ActivityListRequest): Promise<ActivityListPage>;
  /** List operational Sync Run and export lifecycle summaries. */
  listOperations(home: string, request: OperationsRequest): Promise<OperationsPage>;
  /**
   * Subscribe to typed Fixture journey progress phases.
   * @param listener - progress callback
   * @returns unsubscribe function
   */
  onProgress(listener: (phase: FixtureJourneyPhase) => void): () => void;
  /**
   * Subscribe to typed Sync Run progress events.
   */
  onSyncProgress(listener: (progress: SyncProgress) => void): () => void;
  /**
   * Subscribe to typed export publication progress events.
   */
  onExportProgress(listener: (progress: ExportProgress) => void): () => void;
};
