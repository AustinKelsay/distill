-- Distill Library schema v2: per-Source preferences and durable Sync Runs.
-- Additive only. Never rewrite 0001_initial.sql.

ALTER TABLE sources ADD COLUMN enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1));
ALTER TABLE sources ADD COLUMN configured_root TEXT;

CREATE TABLE sync_runs (
  id INTEGER PRIMARY KEY NOT NULL,
  status TEXT NOT NULL CHECK (
    status IN ('queued', 'running', 'completed', 'warning', 'failed', 'cancelled')
  ),
  requested_at TEXT NOT NULL,
  started_at TEXT,
  finished_at TEXT,
  cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
  owner_id TEXT NOT NULL,
  heartbeat_at TEXT NOT NULL,
  lease_expires_at TEXT NOT NULL,
  metrics_json TEXT NOT NULL DEFAULT '{}',
  error_class TEXT,
  error_message TEXT,
  warning_details_json TEXT NOT NULL DEFAULT '[]'
);

-- At most one queued or running Sync Run per Distill home.
CREATE UNIQUE INDEX idx_sync_runs_one_active
  ON sync_runs((1))
  WHERE status IN ('queued', 'running');

CREATE TABLE sync_run_source_outcomes (
  id INTEGER PRIMARY KEY NOT NULL,
  sync_run_id INTEGER NOT NULL REFERENCES sync_runs(id) ON DELETE CASCADE,
  source_kind TEXT NOT NULL,
  status TEXT NOT NULL CHECK (
    status IN ('pending', 'running', 'completed', 'warning', 'failed', 'cancelled', 'skipped')
  ),
  accepted_captures INTEGER NOT NULL DEFAULT 0,
  skipped_duplicates INTEGER NOT NULL DEFAULT 0,
  successful_attempts INTEGER NOT NULL DEFAULT 0,
  failed_attempts INTEGER NOT NULL DEFAULT 0,
  error_class TEXT,
  error_message TEXT,
  metrics_json TEXT NOT NULL DEFAULT '{}',
  UNIQUE (sync_run_id, source_kind)
);

CREATE INDEX idx_sync_runs_status ON sync_runs(status);
CREATE INDEX idx_sync_run_outcomes_run ON sync_run_source_outcomes(sync_run_id);
