-- Distill Library schema v3: curation storage for read models (#23) and mutations (#24).
-- Additive only. Does not implement curation write APIs.

CREATE TABLE tags (
  id INTEGER PRIMARY KEY NOT NULL,
  name TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL DEFAULT 'general',
  created_at TEXT NOT NULL
);

CREATE TABLE tag_assignments (
  id INTEGER PRIMARY KEY NOT NULL,
  object_type TEXT NOT NULL,
  object_id INTEGER NOT NULL,
  tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  origin TEXT NOT NULL,
  confidence REAL,
  created_at TEXT NOT NULL,
  UNIQUE (object_type, object_id, tag_id, origin)
);

CREATE INDEX idx_tag_assignments_object
  ON tag_assignments(object_type, object_id);

CREATE INDEX idx_tag_assignments_tag
  ON tag_assignments(tag_id);

CREATE TABLE labels (
  id INTEGER PRIMARY KEY NOT NULL,
  name TEXT NOT NULL UNIQUE,
  scope TEXT NOT NULL DEFAULT 'session',
  created_at TEXT NOT NULL
);

CREATE TABLE label_assignments (
  id INTEGER PRIMARY KEY NOT NULL,
  object_type TEXT NOT NULL,
  object_id INTEGER NOT NULL,
  label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
  origin TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE (object_type, object_id, label_id)
);

CREATE INDEX idx_label_assignments_object
  ON label_assignments(object_type, object_id);

CREATE INDEX idx_label_assignments_label
  ON label_assignments(label_id);

-- Starter label catalog for workflow / export lanes. Assignments are created by #24.
INSERT INTO labels (name, scope, created_at) VALUES
  ('train', 'session', '1970-01-01T00:00:00Z'),
  ('holdout', 'session', '1970-01-01T00:00:00Z'),
  ('exclude', 'session', '1970-01-01T00:00:00Z'),
  ('sensitive', 'session', '1970-01-01T00:00:00Z'),
  ('favorite', 'session', '1970-01-01T00:00:00Z');
