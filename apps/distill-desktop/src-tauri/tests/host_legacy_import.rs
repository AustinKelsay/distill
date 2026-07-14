//! Tauri host legacy Electron import seam for issue #31.

use std::fs;
use std::path::Path;

use distill_desktop_lib::{
    execute_import_legacy, validate_home_request, validate_legacy_import_request,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

/**
 * Build a minimal legacy Electron home for host import tests.
 */
fn build_legacy_home(root: &Path) -> std::path::PathBuf {
    let home = root.join("legacy-home");
    fs::create_dir_all(home.join("blobs")).expect("blobs");
    fs::create_dir_all(home.join("exports")).expect("exports");
    let conn = Connection::open(home.join("distill.db")).expect("db");
    conn.execute_batch(
        r#"
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
        "#,
    )
    .expect("schema");

    let text = "hello host legacy\n";
    let sha = hex::encode(Sha256::digest(text.as_bytes()));
    let payload = serde_json::json!({
        "contentRef": {
            "kind": "inline",
            "mediaType": "text/plain",
            "text": text,
            "sha256": sha,
            "byteSize": text.len()
        }
    });
    conn.execute(
        "INSERT INTO sources (kind, display_name) VALUES ('fixture', 'Fixture')",
        [],
    )
    .unwrap();
    let source_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO captures (
            source_id, capture_kind, external_session_id, source_path, raw_sha256,
            raw_payload_json, parser_version, status, captured_at
         ) VALUES (?1, 'file', 'host-legacy-1', 'a.jsonl', ?2, ?3, 'v0', 'normalized', '2026-01-01T00:00:00Z')",
        rusqlite::params![source_id, sha, payload.to_string()],
    )
    .unwrap();
    let capture_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO capture_records (
            capture_id, line_no, record_type, role, is_meta, content_text, content_json
         ) VALUES (?1, 1, 'message', 'user', 0, 'hello', '{}')",
        [capture_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (
            source_id, external_session_id, title, raw_capture_count, metadata_json
         ) VALUES (?1, 'host-legacy-1', 'Host Legacy', 1, '{}')",
        [source_id],
    )
    .unwrap();
    let session_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO messages (
            session_id, ordinal, role, text, text_hash, message_kind, metadata_json
         ) VALUES (?1, 0, 'user', 'hello', 'x', 'text', '{}')",
        [session_id],
    )
    .unwrap();
    drop(conn);
    home
}

#[test]
fn host_import_legacy_is_typed_and_validates_empty_paths() {
    let err = validate_legacy_import_request("", "/tmp/legacy").expect_err("empty home");
    assert_eq!(err.code, "validation");
    let err = validate_legacy_import_request("/tmp/home", "").expect_err("empty source");
    assert_eq!(err.code, "validation");

    let temp = TempDir::new().expect("temp");
    let legacy = build_legacy_home(temp.path());
    let dest = temp.path().join("native");
    let _ = validate_home_request(dest.to_str().unwrap()).expect("home");
    let request = validate_legacy_import_request(dest.to_str().unwrap(), legacy.to_str().unwrap())
        .expect("request");
    let report = execute_import_legacy(&request).expect("import");
    assert!(report.ok);
    assert_eq!(report.counts.sessions, 1);
    assert!(!report.source_fingerprint.is_empty());
}

#[test]
fn host_import_legacy_rejects_same_home() {
    let temp = TempDir::new().expect("temp");
    let dest = temp.path().join("native");
    fs::create_dir_all(&dest).expect("dest");
    // Open via Library through import will create native schema; first create home via import validation path.
    let request = validate_legacy_import_request(dest.to_str().unwrap(), dest.to_str().unwrap())
        .expect("request");
    let err = execute_import_legacy(&request).expect_err("same path");
    assert_eq!(err.code, "invalid_argument");
}
