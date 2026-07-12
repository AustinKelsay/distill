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
};

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
};
