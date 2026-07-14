-- Distill Library schema v4: durable export publication bookkeeping (#25).
-- Additive only. Never rewrite earlier migrations.

CREATE TABLE exports (
  id INTEGER PRIMARY KEY NOT NULL,
  format_id TEXT NOT NULL,
  dataset TEXT NOT NULL CHECK (dataset IN ('train', 'holdout')),
  status TEXT NOT NULL CHECK (
    status IN ('preparing', 'committed', 'published', 'failed_publish', 'cancelled')
  ),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  temp_path TEXT,
  output_path TEXT,
  sha256 TEXT,
  byte_size INTEGER CHECK (byte_size IS NULL OR byte_size >= 0),
  record_count INTEGER NOT NULL DEFAULT 0 CHECK (record_count >= 0),
  eligibility_snapshot_json TEXT NOT NULL DEFAULT '{}',
  error_class TEXT,
  error_message TEXT,
  CHECK (
    (status = 'published' AND output_path IS NOT NULL AND sha256 IS NOT NULL AND byte_size IS NOT NULL)
    OR (status != 'published')
  )
);

CREATE INDEX idx_exports_status ON exports(status);
CREATE INDEX idx_exports_dataset_created ON exports(dataset, created_at);
