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

/** Library health report. */
export type HealthReport = {
  ok: boolean;
  schema_status: string;
  content_status: string;
  fts_status: string;
  issues: string[];
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
   * Subscribe to typed progress phases.
   * @param listener - progress callback
   * @returns unsubscribe function
   */
  onProgress(listener: (phase: FixtureJourneyPhase) => void): () => void;
};
