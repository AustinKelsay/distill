-- Distill Library schema v1: Captures, Attempts, Facts, Projection, FTS, Activity.
-- Fresh rebuild schema. Does not include or migrate the legacy Electron schema.
-- `schema_migrations` is bootstrapped by the migrator before this script runs.

CREATE TABLE sources (
  id INTEGER PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL,
  data_root TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE captures (
  id INTEGER PRIMARY KEY NOT NULL,
  source_id INTEGER NOT NULL REFERENCES sources(id),
  source_kind TEXT NOT NULL,
  source_path TEXT NOT NULL,
  external_session_id TEXT,
  content_kind TEXT NOT NULL CHECK (content_kind IN ('inline', 'blob')),
  media_type TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
  inline_text TEXT,
  blob_path TEXT,
  source_modified_at TEXT,
  accepted_at TEXT NOT NULL,
  UNIQUE (source_kind, source_path, sha256),
  CHECK (
    (content_kind = 'inline' AND inline_text IS NOT NULL AND blob_path IS NULL)
    OR (content_kind = 'blob' AND blob_path IS NOT NULL AND inline_text IS NULL)
  )
);

CREATE TABLE normalization_attempts (
  id INTEGER PRIMARY KEY NOT NULL,
  capture_id INTEGER NOT NULL REFERENCES captures(id),
  parser_id TEXT NOT NULL,
  parser_version TEXT NOT NULL,
  started_at TEXT NOT NULL,
  finished_at TEXT,
  outcome TEXT NOT NULL CHECK (outcome IN ('pending', 'succeeded', 'failed')),
  error_class TEXT,
  error_message TEXT,
  projection_generation INTEGER,
  metrics_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE capture_facts (
  id INTEGER PRIMARY KEY NOT NULL,
  attempt_id INTEGER NOT NULL REFERENCES normalization_attempts(id),
  ordinal INTEGER NOT NULL,
  record_type TEXT NOT NULL,
  role TEXT,
  is_meta INTEGER NOT NULL DEFAULT 0 CHECK (is_meta IN (0, 1)),
  content_text TEXT,
  content_json TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  UNIQUE (attempt_id, ordinal)
);

CREATE TABLE sessions (
  id INTEGER PRIMARY KEY NOT NULL,
  source_kind TEXT NOT NULL,
  external_session_id TEXT NOT NULL,
  title TEXT,
  project_path TEXT,
  source_url TEXT,
  summary TEXT,
  started_at TEXT,
  updated_at TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  accepted_capture_count INTEGER NOT NULL DEFAULT 0,
  normalization_attempt_count INTEGER NOT NULL DEFAULT 0,
  successful_projection_generation INTEGER NOT NULL DEFAULT 0,
  current_attempt_id INTEGER REFERENCES normalization_attempts(id),
  UNIQUE (source_kind, external_session_id)
);

CREATE TABLE projection_messages (
  id INTEGER PRIMARY KEY NOT NULL,
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  projection_generation INTEGER NOT NULL,
  ordinal INTEGER NOT NULL,
  role TEXT NOT NULL,
  message_kind TEXT NOT NULL CHECK (message_kind IN ('text', 'meta')),
  text TEXT NOT NULL,
  external_message_id TEXT,
  created_at TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  UNIQUE (session_id, projection_generation, ordinal)
);

CREATE TABLE projection_artifacts (
  id INTEGER PRIMARY KEY NOT NULL,
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  projection_generation INTEGER NOT NULL,
  message_id INTEGER REFERENCES projection_messages(id) ON DELETE SET NULL,
  capture_fact_id INTEGER REFERENCES capture_facts(id) ON DELETE SET NULL,
  artifact_type TEXT NOT NULL,
  media_type TEXT,
  text_preview TEXT,
  content_json TEXT NOT NULL DEFAULT '{}',
  metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE activity_events (
  id INTEGER PRIMARY KEY NOT NULL,
  event_type TEXT NOT NULL,
  occurred_at TEXT NOT NULL,
  source_kind TEXT,
  session_id INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
  capture_id INTEGER REFERENCES captures(id) ON DELETE SET NULL,
  attempt_id INTEGER REFERENCES normalization_attempts(id) ON DELETE SET NULL,
  payload_json TEXT NOT NULL DEFAULT '{}'
);

CREATE VIRTUAL TABLE projection_fts USING fts5(
  session_id UNINDEXED,
  message_id UNINDEXED,
  title,
  project_path,
  role,
  text,
  tokenize = 'unicode61'
);

CREATE INDEX idx_captures_session ON captures(source_kind, external_session_id);
CREATE INDEX idx_attempts_capture ON normalization_attempts(capture_id);
CREATE INDEX idx_projection_messages_session ON projection_messages(session_id, projection_generation);
CREATE INDEX idx_activity_occurred ON activity_events(occurred_at);
