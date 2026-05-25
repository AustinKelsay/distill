PRAGMA foreign_keys = ON;

-- Source installations and discovered local roots.
CREATE TABLE IF NOT EXISTS sources (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL,
  executable_path TEXT,
  data_root TEXT,
  install_status TEXT NOT NULL DEFAULT 'unknown',
  detected_at TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- One row per imported raw source file or snapshot.
CREATE TABLE IF NOT EXISTS captures (
  id INTEGER PRIMARY KEY,
  source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
  capture_kind TEXT NOT NULL,
  external_session_id TEXT,
  source_path TEXT,
  source_modified_at TEXT,
  source_size_bytes INTEGER,
  raw_sha256 TEXT NOT NULL,
  raw_blob_path TEXT,
  raw_payload_json TEXT,
  parser_version TEXT,
  status TEXT NOT NULL DEFAULT 'captured',
  error_text TEXT,
  captured_at TEXT NOT NULL,
  imported_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE (source_id, source_path, raw_sha256)
);

CREATE INDEX IF NOT EXISTS idx_captures_source_session
  ON captures(source_id, external_session_id);

CREATE INDEX IF NOT EXISTS idx_captures_status
  ON captures(status);

-- One normalized session per provider session.
CREATE TABLE IF NOT EXISTS sessions (
  id INTEGER PRIMARY KEY,
  source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
  external_session_id TEXT NOT NULL,
  title TEXT,
  project_path TEXT,
  source_url TEXT,
  model TEXT,
  model_provider TEXT,
  cli_version TEXT,
  git_branch TEXT,
  started_at TEXT,
  updated_at TEXT,
  message_count INTEGER NOT NULL DEFAULT 0,
  raw_capture_count INTEGER NOT NULL DEFAULT 0,
  token_estimate INTEGER,
  summary TEXT,
  import_status TEXT NOT NULL DEFAULT 'ready',
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_recorded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE (source_id, external_session_id)
);

CREATE INDEX IF NOT EXISTS idx_sessions_source_updated
  ON sessions(source_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_sessions_project_path
  ON sessions(project_path);

CREATE INDEX IF NOT EXISTS idx_sessions_model
  ON sessions(model);

-- Raw parsed records from a capture before normalization.
CREATE TABLE IF NOT EXISTS capture_records (
  id INTEGER PRIMARY KEY,
  capture_id INTEGER NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
  line_no INTEGER NOT NULL,
  record_type TEXT NOT NULL,
  record_timestamp TEXT,
  provider_message_id TEXT,
  parent_provider_message_id TEXT,
  role TEXT,
  is_meta INTEGER NOT NULL DEFAULT 0,
  content_text TEXT,
  content_json TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE (capture_id, line_no)
);

CREATE INDEX IF NOT EXISTS idx_capture_records_capture
  ON capture_records(capture_id, line_no);

CREATE INDEX IF NOT EXISTS idx_capture_records_provider_message
  ON capture_records(provider_message_id);

-- User-visible normalized messages.
CREATE TABLE IF NOT EXISTS messages (
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  capture_record_id INTEGER REFERENCES capture_records(id) ON DELETE SET NULL,
  external_message_id TEXT,
  parent_external_message_id TEXT,
  ordinal INTEGER NOT NULL,
  role TEXT NOT NULL,
  text TEXT NOT NULL,
  text_hash TEXT NOT NULL,
  created_at TEXT,
  message_kind TEXT NOT NULL DEFAULT 'text',
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_recorded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE (session_id, ordinal),
  UNIQUE (session_id, external_message_id)
);

CREATE INDEX IF NOT EXISTS idx_messages_session
  ON messages(session_id, ordinal);

CREATE INDEX IF NOT EXISTS idx_messages_external_id
  ON messages(external_message_id);

CREATE INDEX IF NOT EXISTS idx_messages_text_hash
  ON messages(session_id, text_hash, role, created_at);

-- Non-text payloads and large artifacts.
CREATE TABLE IF NOT EXISTS artifacts (
  id INTEGER PRIMARY KEY,
  session_id INTEGER REFERENCES sessions(id) ON DELETE CASCADE,
  message_id INTEGER REFERENCES messages(id) ON DELETE CASCADE,
  capture_record_id INTEGER REFERENCES capture_records(id) ON DELETE SET NULL,
  kind TEXT NOT NULL,
  mime_type TEXT,
  blob_path TEXT,
  sha256 TEXT,
  byte_size INTEGER,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_artifacts_session
  ON artifacts(session_id);

-- Lightweight descriptors.
CREATE TABLE IF NOT EXISTS tags (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL DEFAULT 'general',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS tag_assignments (
  id INTEGER PRIMARY KEY,
  object_type TEXT NOT NULL,
  object_id INTEGER NOT NULL,
  tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  origin TEXT NOT NULL,
  confidence REAL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE (object_type, object_id, tag_id, origin)
);

CREATE INDEX IF NOT EXISTS idx_tag_assignments_object
  ON tag_assignments(object_type, object_id);

-- Stronger curation states.
CREATE TABLE IF NOT EXISTS labels (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  scope TEXT NOT NULL DEFAULT 'session',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS label_assignments (
  id INTEGER PRIMARY KEY,
  object_type TEXT NOT NULL,
  object_id INTEGER NOT NULL,
  label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
  origin TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE (object_type, object_id, label_id)
);

CREATE INDEX IF NOT EXISTS idx_label_assignments_object
  ON label_assignments(object_type, object_id);

-- Background automation.
CREATE TABLE IF NOT EXISTS jobs (
  id INTEGER PRIMARY KEY,
  job_type TEXT NOT NULL,
  object_type TEXT NOT NULL,
  object_id INTEGER NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  attempts INTEGER NOT NULL DEFAULT 0,
  run_after TEXT,
  last_error TEXT,
  payload_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_jobs_status_run_after
  ON jobs(status, run_after);

-- Product activity feed.
CREATE TABLE IF NOT EXISTS activity_events (
  id INTEGER PRIMARY KEY,
  event_type TEXT NOT NULL,
  object_type TEXT NOT NULL,
  object_id INTEGER,
  session_id INTEGER REFERENCES sessions(id) ON DELETE CASCADE,
  payload_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_activity_events_created
  ON activity_events(created_at DESC);

CREATE INDEX IF NOT EXISTS idx_activity_events_session
  ON activity_events(session_id, created_at DESC);

-- Export bookkeeping.
CREATE TABLE IF NOT EXISTS exports (
  id INTEGER PRIMARY KEY,
  export_type TEXT NOT NULL,
  label_filter TEXT,
  output_path TEXT NOT NULL,
  record_count INTEGER NOT NULL DEFAULT 0,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- User preferences (key-value store for UI settings).
CREATE TABLE IF NOT EXISTS user_preferences (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Search index over normalized message text and session metadata.
CREATE VIRTUAL TABLE IF NOT EXISTS message_fts
USING fts5(
  session_id UNINDEXED,
  message_id UNINDEXED,
  title,
  project_path,
  role,
  text,
  tokenize = 'unicode61'
);

CREATE TRIGGER IF NOT EXISTS messages_ai
AFTER INSERT ON messages
BEGIN
  INSERT INTO message_fts(session_id, message_id, title, project_path, role, text)
  VALUES (
    NEW.session_id,
    NEW.id,
    COALESCE((SELECT title FROM sessions WHERE id = NEW.session_id), ''),
    COALESCE((SELECT project_path FROM sessions WHERE id = NEW.session_id), ''),
    NEW.role,
    NEW.text
  );
END;

CREATE TRIGGER IF NOT EXISTS messages_ad
AFTER DELETE ON messages
BEGIN
  DELETE FROM message_fts WHERE message_id = OLD.id;
END;

CREATE TRIGGER IF NOT EXISTS messages_au
AFTER UPDATE ON messages
BEGIN
  DELETE FROM message_fts WHERE message_id = OLD.id;
  INSERT INTO message_fts(session_id, message_id, title, project_path, role, text)
  VALUES (
    NEW.session_id,
    NEW.id,
    COALESCE((SELECT title FROM sessions WHERE id = NEW.session_id), ''),
    COALESCE((SELECT project_path FROM sessions WHERE id = NEW.session_id), ''),
    NEW.role,
    NEW.text
  );
END;
