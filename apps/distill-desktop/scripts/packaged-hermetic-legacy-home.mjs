/**
 * Hermetic synthetic legacy Electron home for packaged migration smoke (#50).
 *
 * Builds a temporary Electron-shaped Distill home that mirrors the host/CLI
 * migrate fixtures (`host_legacy_import.rs` / `cli_fixture_journey.rs`):
 * `distill.db`, empty `blobs/` + `exports/`, one Fixture Source/Capture/Session,
 * and empty regular-file WAL/SHM sidecars so before/after SHA-256 coverage includes
 * the journal companion paths the importer fingerprints. Live WAL writers are not
 * held open across the smoke; Library LMI-001 remains the live-WAL contract.
 * Never points at a live user home or Electron product tree.
 */

import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { promises as fs } from "node:fs";
import path from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

/**
 * @typedef {object} HermeticLegacyHome
 * @property {string} legacyHome
 * @property {string} sessionTitle
 * @property {string} externalSessionId
 * @property {string} searchQuery
 * @property {number} expectedCaptures
 * @property {number} expectedSessions
 */

/**
 * Plan absolute paths for a synthetic legacy Electron home under a smoke base.
 *
 * @param {string} base - temporary parent owned by the packaged smoke
 * @param {{ sessionTitle: string, externalSessionId: string, searchQuery: string }} labels
 * @returns {HermeticLegacyHome}
 */
export function planHermeticLegacyHome(base, labels) {
  return {
    legacyHome: path.join(base, "legacy-home"),
    sessionTitle: labels.sessionTitle,
    externalSessionId: labels.externalSessionId,
    searchQuery: labels.searchQuery,
    expectedCaptures: 1,
    expectedSessions: 1,
  };
}

/**
 * SHA-256 hex digest of UTF-8 bytes.
 *
 * @param {string} text
 * @returns {string}
 */
function sha256Hex(text) {
  return createHash("sha256").update(text, "utf8").digest("hex");
}

/**
 * Escape a string for embedding inside a Python triple-quoted literal.
 *
 * @param {string} value
 * @returns {string}
 */
function pyTripleQuoted(value) {
  return value.replace(/\\/g, "\\\\").replace(/"""/g, '\\"""');
}

/**
 * Build the host/CLI-shaped legacy SQLite home with WAL sidecars via Python.
 *
 * Parameters match the host fixture: Fixture Source, inline Capture contentRef,
 * one Session/message, plus empty regular-file `distill.db-wal` / `distill.db-shm`
 * companions for packaged immutability hashing of sidecar paths.
 *
 * @param {string} base - temporary parent owned by the packaged smoke
 * @param {{ sessionTitle: string, externalSessionId: string, searchQuery: string }} labels
 * @returns {Promise<HermeticLegacyHome>}
 */
export async function seedHermeticLegacyHome(base, labels) {
  const planned = planHermeticLegacyHome(base, labels);
  await fs.mkdir(path.join(planned.legacyHome, "blobs"), { recursive: true });
  await fs.mkdir(path.join(planned.legacyHome, "exports"), { recursive: true });

  const text = "hello packaged legacy\n";
  const sha = sha256Hex(text);
  const payload = JSON.stringify({
    contentRef: {
      kind: "inline",
      mediaType: "text/plain",
      text,
      sha256: sha,
      byteSize: text.length,
    },
  });
  const dbPath = path.join(planned.legacyHome, "distill.db");

  const script = `
import json
import sqlite3
from pathlib import Path

db_path = Path("""${pyTripleQuoted(dbPath)}""")
conn = sqlite3.connect(db_path)
conn.executescript("""
CREATE TABLE sources (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL,
  data_root TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE captures (
  id INTEGER PRIMARY KEY,
  source_id INTEGER NOT NULL REFERENCES sources(id),
  capture_kind TEXT NOT NULL,
  external_session_id TEXT,
  source_path TEXT,
  source_modified_at TEXT,
  raw_sha256 TEXT NOT NULL,
  raw_blob_path TEXT,
  raw_payload_json TEXT,
  parser_version TEXT,
  status TEXT NOT NULL DEFAULT 'captured',
  captured_at TEXT NOT NULL
);
CREATE TABLE sessions (
  id INTEGER PRIMARY KEY,
  source_id INTEGER NOT NULL REFERENCES sources(id),
  external_session_id TEXT NOT NULL,
  title TEXT,
  project_path TEXT,
  source_url TEXT,
  summary TEXT,
  started_at TEXT,
  updated_at TEXT,
  raw_capture_count INTEGER NOT NULL DEFAULT 0,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  UNIQUE (source_id, external_session_id)
);
CREATE TABLE capture_records (
  id INTEGER PRIMARY KEY,
  capture_id INTEGER NOT NULL REFERENCES captures(id),
  line_no INTEGER NOT NULL,
  record_type TEXT NOT NULL,
  role TEXT,
  is_meta INTEGER NOT NULL DEFAULT 0,
  content_text TEXT,
  content_json TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE messages (
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES sessions(id),
  ordinal INTEGER NOT NULL,
  role TEXT NOT NULL,
  text TEXT NOT NULL,
  text_hash TEXT NOT NULL,
  created_at TEXT,
  message_kind TEXT NOT NULL DEFAULT 'text',
  metadata_json TEXT NOT NULL DEFAULT '{}',
  external_message_id TEXT
);
CREATE TABLE artifacts (
  id INTEGER PRIMARY KEY,
  session_id INTEGER REFERENCES sessions(id),
  kind TEXT NOT NULL,
  mime_type TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE tags (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL DEFAULT 'general',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE tag_assignments (
  id INTEGER PRIMARY KEY,
  object_type TEXT NOT NULL,
  object_id INTEGER NOT NULL,
  tag_id INTEGER NOT NULL REFERENCES tags(id),
  origin TEXT NOT NULL,
  confidence REAL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE (object_type, object_id, tag_id, origin)
);
CREATE TABLE labels (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  scope TEXT NOT NULL DEFAULT 'session',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE label_assignments (
  id INTEGER PRIMARY KEY,
  object_type TEXT NOT NULL,
  object_id INTEGER NOT NULL,
  label_id INTEGER NOT NULL REFERENCES labels(id),
  origin TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE (object_type, object_id, label_id)
);
CREATE TABLE activity_events (
  id INTEGER PRIMARY KEY,
  event_type TEXT NOT NULL,
  object_type TEXT NOT NULL,
  object_id INTEGER,
  session_id INTEGER,
  payload_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE exports (
  id INTEGER PRIMARY KEY,
  export_type TEXT NOT NULL,
  label_filter TEXT,
  output_path TEXT NOT NULL,
  record_count INTEGER NOT NULL DEFAULT 0,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
""")
sha = """${sha}"""
payload = """${pyTripleQuoted(payload)}"""
title = """${pyTripleQuoted(planned.sessionTitle)}"""
external = """${pyTripleQuoted(planned.externalSessionId)}"""
conn.execute(
    "INSERT INTO sources (kind, display_name) VALUES ('fixture', 'Fixture')"
)
source_id = conn.execute("SELECT last_insert_rowid()").fetchone()[0]
conn.execute(
    """INSERT INTO captures (
        source_id, capture_kind, external_session_id, source_path, raw_sha256,
        raw_payload_json, parser_version, status, captured_at
     ) VALUES (?, 'file', ?, 'a.jsonl', ?, ?, 'v0', 'normalized', '2026-01-01T00:00:00Z')""",
    (source_id, external, sha, payload),
)
capture_id = conn.execute("SELECT last_insert_rowid()").fetchone()[0]
conn.execute(
    """INSERT INTO capture_records (
        capture_id, line_no, record_type, role, is_meta, content_text, content_json
     ) VALUES (?, 1, 'message', 'user', 0, 'hello', '{}')""",
    (capture_id,),
)
conn.execute(
    """INSERT INTO sessions (
        source_id, external_session_id, title, raw_capture_count, metadata_json
     ) VALUES (?, ?, ?, 1, '{}')""",
    (source_id, external, title),
)
session_id = conn.execute("SELECT last_insert_rowid()").fetchone()[0]
conn.execute(
    """INSERT INTO messages (
        session_id, ordinal, role, text, text_hash, message_kind, metadata_json
     ) VALUES (?, 0, 'user', 'hello packaged legacy', 'x', 'text', '{}')""",
    (session_id,),
)
conn.commit()
conn.close()
# Python's sqlite3 checkpoints and removes live WAL files on close. Plant empty
# regular-file sidecars so packaged before/after hashes cover the same paths the
# importer fingerprints (distill.db-wal / distill.db-shm), matching host/CLI
# fixture homes that may carry journal companions without holding a live writer.
(db_path.with_name(db_path.name + "-wal")).write_bytes(b"")
(db_path.with_name(db_path.name + "-shm")).write_bytes(b"")
print(json.dumps({"ok": True}))
`;

  const result = await execFileAsync("python3", ["-c", script], {
    maxBuffer: 2 * 1024 * 1024,
  });
  const report = JSON.parse(String(result.stdout).trim());
  if (!report.ok) {
    throw new Error("hermetic legacy home seeder reported failure");
  }

  await fs.access(dbPath);
  // WAL sidecars must exist for LPKG-007/PKG-007 immutability coverage.
  await fs.access(`${dbPath}-wal`);
  await fs.access(`${dbPath}-shm`);
  return planned;
}
