-- Distill Library schema v5: durable legacy Electron import markers (#31).
-- Additive only. Never rewrite earlier migrations.
-- Stores redacted import reports keyed by source DB/content fingerprint for idempotent retry.

CREATE TABLE legacy_import_markers (
  id INTEGER PRIMARY KEY NOT NULL,
  source_fingerprint TEXT NOT NULL UNIQUE,
  source_db_sha256 TEXT NOT NULL,
  content_fingerprint TEXT NOT NULL,
  report_json TEXT NOT NULL,
  imported_at TEXT NOT NULL
);

CREATE INDEX idx_legacy_import_markers_imported
  ON legacy_import_markers(imported_at);
