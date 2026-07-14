//! Library seam: read-only legacy Electron home import (#31).
//!
//! Builds a temporary legacy SQLite home fixture, imports into a native home,
//! and proves source digest unchanged, alias rejection, representative mapping,
//! idempotent retry, and redacted report handling.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use distill_library::{
    Library, LibraryError, SessionDetailRequest, SessionListRequest, WorkflowLane,
    LEGACY_IMPORT_PARSER_ID,
};

/**
 * Apply a minimal legacy Electron schema sufficient for import mapping.
 */
fn apply_legacy_schema(conn: &Connection) {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        CREATE TABLE sources (
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
        CREATE TABLE captures (
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
        CREATE TABLE sessions (
          id INTEGER PRIMARY KEY,
          source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
          external_session_id TEXT NOT NULL,
          title TEXT,
          project_path TEXT,
          source_url TEXT,
          model TEXT,
          summary TEXT,
          started_at TEXT,
          updated_at TEXT,
          message_count INTEGER NOT NULL DEFAULT 0,
          raw_capture_count INTEGER NOT NULL DEFAULT 0,
          import_status TEXT NOT NULL DEFAULT 'ready',
          metadata_json TEXT NOT NULL DEFAULT '{}',
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
          UNIQUE (source_id, external_session_id)
        );
        CREATE TABLE capture_records (
          id INTEGER PRIMARY KEY,
          capture_id INTEGER NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
          line_no INTEGER NOT NULL,
          record_type TEXT NOT NULL,
          record_timestamp TEXT,
          role TEXT,
          is_meta INTEGER NOT NULL DEFAULT 0,
          content_text TEXT,
          content_json TEXT NOT NULL,
          metadata_json TEXT NOT NULL DEFAULT '{}',
          UNIQUE (capture_id, line_no)
        );
        CREATE TABLE messages (
          id INTEGER PRIMARY KEY,
          session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
          ordinal INTEGER NOT NULL,
          role TEXT NOT NULL,
          text TEXT NOT NULL,
          text_hash TEXT NOT NULL,
          created_at TEXT,
          message_kind TEXT NOT NULL DEFAULT 'text',
          metadata_json TEXT NOT NULL DEFAULT '{}',
          external_message_id TEXT,
          UNIQUE (session_id, ordinal)
        );
        CREATE TABLE artifacts (
          id INTEGER PRIMARY KEY,
          session_id INTEGER REFERENCES sessions(id) ON DELETE CASCADE,
          kind TEXT NOT NULL,
          mime_type TEXT,
          blob_path TEXT,
          sha256 TEXT,
          byte_size INTEGER,
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
          tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
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
          label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
          origin TEXT NOT NULL,
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
          UNIQUE (object_type, object_id, label_id)
        );
        CREATE TABLE activity_events (
          id INTEGER PRIMARY KEY,
          event_type TEXT NOT NULL,
          object_type TEXT NOT NULL,
          object_id INTEGER,
          session_id INTEGER REFERENCES sessions(id) ON DELETE CASCADE,
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
    .expect("legacy schema");
}

/**
 * Digest every regular file under a home for byte-for-byte comparison.
 */
fn home_digest(home: &Path) -> String {
    let mut entries: Vec<(String, String)> = Vec::new();
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        let Ok(read) = fs::read_dir(dir) else {
            return;
        };
        for entry in read.flatten() {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                walk(root, &path, out);
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = fs::read(&path).expect("read");
            out.push((rel, hex::encode(Sha256::digest(bytes))));
        }
    }
    walk(home, home, &mut entries);
    entries.sort();
    let mut hasher = Sha256::new();
    for (rel, digest) in entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(digest.as_bytes());
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

/**
 * Build a representative legacy Electron home with inline capture, curation, activity, and export.
 */
fn build_legacy_home(root: &Path) -> PathBuf {
    let home = root.join("legacy-home");
    fs::create_dir_all(home.join("blobs")).expect("blobs");
    fs::create_dir_all(home.join("exports")).expect("exports");
    let db_path = home.join("distill.db");
    let conn = Connection::open(&db_path).expect("open legacy db");
    apply_legacy_schema(&conn);

    let inline_text = concat!(
        r#"{"record_type":"session_meta","title":"Legacy Session"}"#,
        "\n",
        r#"{"record_type":"message","role":"user","text":"hello legacy"}"#,
        "\n",
    );
    let sha = hex::encode(Sha256::digest(inline_text.as_bytes()));
    let payload = serde_json::json!({
        "sourceKind": "fixture",
        "metadata": {},
        "contentRef": {
            "kind": "inline",
            "mediaType": "application/x-ndjson; charset=utf-8",
            "text": inline_text,
            "sha256": sha,
            "byteSize": inline_text.len()
        }
    });

    conn.execute(
        "INSERT INTO sources (kind, display_name, data_root, metadata_json)
         VALUES ('fixture', 'Fixture', '/tmp/fixture-root', '{}')",
        [],
    )
    .expect("source");
    let source_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO captures (
            source_id, capture_kind, external_session_id, source_path, raw_sha256,
            raw_blob_path, raw_payload_json, parser_version, status, captured_at
         ) VALUES (?1, 'file', 'legacy-session-1', 'captures/hello.jsonl', ?2, NULL, ?3, 'v0', 'normalized', '2026-01-01T00:00:00Z')",
        rusqlite::params![source_id, sha, payload.to_string()],
    )
    .expect("capture");
    let capture_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO capture_records (
            capture_id, line_no, record_type, role, is_meta, content_text, content_json
         ) VALUES (?1, 1, 'message', 'user', 0, 'hello legacy', '{\"text\":\"hello legacy\"}')",
        [capture_id],
    )
    .expect("record");

    conn.execute(
        "INSERT INTO sessions (
            source_id, external_session_id, title, project_path, summary,
            started_at, updated_at, message_count, raw_capture_count, metadata_json
         ) VALUES (?1, 'legacy-session-1', 'Legacy Session', '/proj', 'summary',
                   '2026-01-01T00:00:00Z', '2026-01-01T00:00:01Z', 1, 1, '{}')",
        [source_id],
    )
    .expect("session");
    let session_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO messages (
            session_id, ordinal, role, text, text_hash, created_at, message_kind, metadata_json
         ) VALUES (?1, 0, 'user', 'hello legacy', 'abc', '2026-01-01T00:00:00Z', 'text', '{}')",
        [session_id],
    )
    .expect("message");
    conn.execute(
        "INSERT INTO artifacts (session_id, kind, mime_type, metadata_json)
         VALUES (?1, 'tool_result', 'application/json', '{\"ok\":true}')",
        [session_id],
    )
    .expect("artifact");

    conn.execute(
        "INSERT INTO tags (name, kind) VALUES ('legacy-tag', 'general')",
        [],
    )
    .expect("tag");
    let tag_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO tag_assignments (object_type, object_id, tag_id, origin)
         VALUES ('session', ?1, ?2, 'manual')",
        rusqlite::params![session_id, tag_id],
    )
    .expect("tag assign");

    conn.execute(
        "INSERT INTO labels (name, scope) VALUES ('train', 'session')",
        [],
    )
    .expect("label");
    let label_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO label_assignments (object_type, object_id, label_id, origin)
         VALUES ('session', ?1, ?2, 'manual')",
        rusqlite::params![session_id, label_id],
    )
    .expect("label assign");

    conn.execute(
        "INSERT INTO activity_events (event_type, object_type, object_id, session_id, payload_json)
         VALUES ('tag_added', 'session', ?1, ?1, ?2)",
        rusqlite::params![
            session_id,
            serde_json::json!({
                "tag": "legacy-tag",
                "sourcePath": "/secret/path/to/capture.jsonl",
                "sql": "SELECT * FROM captures"
            })
            .to_string()
        ],
    )
    .expect("activity");

    let export_body = "{\"session\":\"legacy-session-1\"}\n";
    let export_path = home.join("exports").join("train-sessions.jsonl");
    fs::write(&export_path, export_body).expect("export file");
    conn.execute(
        "INSERT INTO exports (export_type, label_filter, output_path, record_count, metadata_json)
         VALUES ('jsonl', 'train', ?1, 1, '{\"dataset\":\"train\"}')",
        [export_path.to_string_lossy().to_string()],
    )
    .expect("export");

    // Unsafe/missing blob capture that should be skipped, not followed outside home.
    conn.execute(
        "INSERT INTO captures (
            source_id, capture_kind, external_session_id, source_path, raw_sha256,
            raw_blob_path, raw_payload_json, parser_version, status, captured_at
         ) VALUES (?1, 'file', 'legacy-missing', 'captures/missing.jsonl', 'deadbeef',
                   '../../etc/passwd', NULL, 'v0', 'captured', '2026-01-01T00:00:00Z')",
        [source_id],
    )
    .expect("unsafe capture");

    drop(conn);
    home
}

#[test]
fn legacy_import_maps_representative_data_and_leaves_source_unchanged() {
    let temp = TempDir::new().expect("temp");
    let legacy = build_legacy_home(temp.path());
    let before = home_digest(&legacy);
    let dest = temp.path().join("native-home");
    let mut library = Library::open(&dest).expect("open native");

    let report = library
        .import_legacy_electron_home(&legacy)
        .expect("import");
    assert!(report.ok);
    assert!(!report.reused_prior_import);
    assert_eq!(report.counts.sources, 1);
    assert_eq!(report.counts.captures, 1);
    assert!(report.counts.captures_skipped >= 1);
    assert_eq!(report.counts.sessions, 1);
    assert_eq!(report.counts.messages, 1);
    assert_eq!(report.counts.artifacts, 1);
    assert!(report.counts.tag_assignments >= 1);
    assert!(report.counts.label_assignments >= 1);
    assert!(report.counts.exports >= 1);
    assert!(report.skips.iter().any(|s| s.category == "capture_content"));
    assert!(report
        .skips
        .iter()
        .any(|s| s.reason == "artifact_links_unmapped"));
    let report_json = serde_json::to_string(&report).expect("json");
    assert!(!report_json.contains("/secret/"));
    assert!(!report_json.contains("SELECT "));
    assert!(!report_json.contains("passwd"));
    assert!(!report_json.contains(legacy.to_string_lossy().as_ref()));

    let after = home_digest(&legacy);
    assert_eq!(
        before, after,
        "legacy home must remain byte-for-byte unchanged"
    );

    let page = library
        .list_sessions(SessionListRequest {
            query: Some("hello legacy".into()),
            lane: WorkflowLane::All,
            limit: 20,
            cursor: None,
        })
        .expect("search");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].external_session_id, "legacy-session-1");

    let detail = library
        .session_detail(SessionDetailRequest {
            source_kind: "fixture".into(),
            external_session_id: "legacy-session-1".into(),
            message_limit: 50,
            artifact_limit: 50,
            message_cursor: None,
            artifact_cursor: None,
        })
        .expect("detail")
        .expect("present");
    assert_eq!(detail.summary.successful_projection_generation, 1);
    assert!(detail.tags.iter().any(|tag| tag.name == "legacy-tag"));
    assert!(detail.labels.iter().any(|label| label.name == "train"));

    let activity = library.recent_activity(50).expect("activity");
    let capture_id = activity
        .into_iter()
        .find_map(|e| e.capture_id)
        .expect("capture activity");
    assert_eq!(
        library.replay_capture(capture_id).expect("capture bytes"),
        concat!(
            "{\"record_type\":\"session_meta\",\"title\":\"Legacy Session\"}",
            "\n",
            "{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"hello legacy\"}",
            "\n"
        )
        .as_bytes()
    );
    let attempt_page = library.capture_attempts(capture_id).expect("attempts");
    assert!(attempt_page
        .iter()
        .any(|a| a.parser_id == LEGACY_IMPORT_PARSER_ID));

    let activity = library
        .list_activity(distill_library::ActivityListRequest {
            limit: 50,
            cursor: None,
        })
        .expect("activity page");
    let tag_event = activity
        .items
        .iter()
        .find(|e| e.event_type == "tag_added")
        .expect("tag_added imported");
    let payload = tag_event.payload_json.to_string();
    assert!(!payload.contains("/secret/"));
    assert!(!payload.contains("SELECT "));

    let export_bytes = fs::read_dir(dest.join("exports"))
        .expect("destination exports")
        .flatten()
        .map(|entry| fs::read(entry.path()).expect("export bytes"))
        .find(|bytes| {
            bytes
                == br#"{"session":"legacy-session-1"}
"#
        })
        .expect("copied export bytes");
    assert_eq!(
        export_bytes,
        br#"{"session":"legacy-session-1"}
"#
    );
}

#[test]
fn legacy_import_reads_wal_home_without_mutating_source_files() {
    let temp = TempDir::new().expect("temp");
    let legacy = build_legacy_home(temp.path());
    let writer = Connection::open(legacy.join("distill.db")).expect("writer");
    writer
        .execute_batch(
            "PRAGMA journal_mode = WAL; UPDATE sources SET display_name = 'WAL Fixture';",
        )
        .expect("wal write");
    assert!(legacy.join("distill.db-wal").exists());
    let before = home_digest(&legacy);
    let dest = temp.path().join("native-home");
    let mut library = Library::open(&dest).expect("open native");
    let report = library
        .import_legacy_electron_home(&legacy)
        .expect("wal import");
    assert!(report.ok);
    assert_eq!(before, home_digest(&legacy));
    drop(writer);
}

#[test]
fn legacy_import_is_idempotent_for_same_fingerprint() {
    let temp = TempDir::new().expect("temp");
    let legacy = build_legacy_home(temp.path());
    let dest = temp.path().join("native-home");
    let mut library = Library::open(&dest).expect("open");
    let first = library.import_legacy_electron_home(&legacy).expect("first");
    let second = library
        .import_legacy_electron_home(&legacy)
        .expect("second");
    assert!(second.reused_prior_import);
    assert_eq!(first.source_fingerprint, second.source_fingerprint);
    assert_eq!(first.counts.captures, second.counts.captures);

    let page = library
        .list_sessions(SessionListRequest {
            query: None,
            lane: WorkflowLane::All,
            limit: 50,
            cursor: None,
        })
        .expect("list");
    assert_eq!(page.items.len(), 1);
}

#[test]
fn legacy_import_rehydrates_existing_attempt_maps_when_marker_is_missing() {
    let temp = TempDir::new().expect("temp");
    let legacy = build_legacy_home(temp.path());
    let dest = temp.path().join("native-home");
    let mut library = Library::open(&dest).expect("open");
    library.import_legacy_electron_home(&legacy).expect("first");
    drop(library);

    let conn = Connection::open(dest.join("distill.db")).expect("native db");
    conn.execute("DELETE FROM legacy_import_markers", [])
        .expect("marker");
    let before_facts: i64 = conn
        .query_row("SELECT COUNT(*) FROM capture_facts", [], |row| row.get(0))
        .expect("facts");
    drop(conn);

    let mut library = Library::open(&dest).expect("reopen");
    let report = library
        .import_legacy_electron_home(&legacy)
        .expect("reimport");
    assert!(report.ok);
    let conn = Connection::open(dest.join("distill.db")).expect("native db");
    let after_facts: i64 = conn
        .query_row("SELECT COUNT(*) FROM capture_facts", [], |row| row.get(0))
        .expect("facts");
    assert_eq!(before_facts, after_facts);
}

#[test]
fn unsupported_source_skips_captures_and_preserves_preexisting_cas() {
    let temp = TempDir::new().expect("temp");
    let legacy = build_legacy_home(temp.path());
    let bytes = vec![b'z'; 70 * 1024];
    let sha = hex::encode(Sha256::digest(&bytes));
    let blob_rel = format!("{}/{}", &sha[..2], &sha[2..]);
    let blob_path = legacy.join("blobs").join(&blob_rel);
    fs::create_dir_all(blob_path.parent().expect("blob parent")).expect("blob parent");
    fs::write(&blob_path, &bytes).expect("blob");
    let conn = Connection::open(legacy.join("distill.db")).expect("legacy db");
    let source_id: i64 = conn
        .query_row("SELECT id FROM sources LIMIT 1", [], |row| row.get(0))
        .expect("source");
    conn.execute(
        "UPDATE sources SET kind = 'unsupported' WHERE id = ?1",
        [source_id],
    )
    .expect("source kind");
    conn.execute(
        "INSERT INTO captures (
            source_id, capture_kind, external_session_id, source_path, raw_sha256,
            raw_blob_path, raw_payload_json, parser_version, status, captured_at
         ) VALUES (?1, 'file', 'unsupported-session', 'large.jsonl', ?2, ?3, NULL,
                   'v0', 'captured', '2026-01-01T00:00:00Z')",
        rusqlite::params![source_id, sha, blob_rel],
    )
    .expect("capture");
    drop(conn);

    let dest = temp.path().join("native-home");
    let library = Library::open(&dest).expect("native");
    let preexisting = dest.join("blobs").join(&sha[..2]).join(&sha[2..]);
    fs::create_dir_all(preexisting.parent().expect("parent")).expect("parent");
    fs::write(&preexisting, &bytes).expect("preexisting orphan");
    drop(library);
    let mut library = Library::open(&dest).expect("reopen");
    let report = library
        .import_legacy_electron_home(&legacy)
        .expect("unsupported import");
    assert_eq!(report.counts.captures, 0);
    assert!(report.counts.captures_skipped >= 2);
    assert!(report.skips.iter().any(|skip| {
        skip.reason == "source_not_imported" || skip.reason == "unsupported_source_kind"
    }));
    assert!(
        preexisting.exists(),
        "pre-existing CAS must survive cleanup"
    );
}

#[test]
fn legacy_import_rejects_alias_and_ancestor_paths() {
    let temp = TempDir::new().expect("temp");
    let legacy = build_legacy_home(temp.path());
    let dest = temp.path().join("native-home");
    let mut library = Library::open(&dest).expect("open dest");

    let err = library
        .import_legacy_electron_home(&dest)
        .expect_err("same path as destination");
    assert!(matches!(err, LibraryError::InvalidArgument(_)));

    let nested = dest.join("nested-legacy");
    fs::create_dir_all(&nested).expect("nested");
    fs::copy(legacy.join("distill.db"), nested.join("distill.db")).expect("copy db");
    let err = library
        .import_legacy_electron_home(&nested)
        .expect_err("descendant source");
    assert!(matches!(err, LibraryError::InvalidArgument(_)));
}

#[test]
fn legacy_import_rejects_missing_source() {
    let temp = TempDir::new().expect("temp");
    let dest = temp.path().join("native-home");
    let mut library = Library::open(&dest).expect("open");
    let err = library
        .import_legacy_electron_home(temp.path().join("missing"))
        .expect_err("missing");
    assert!(matches!(
        err,
        LibraryError::NotFound(_) | LibraryError::InvalidArgument(_)
    ));
}

#[test]
fn interrupted_import_cleans_uncommitted_blob_files() {
    let temp = TempDir::new().expect("temp");
    let legacy = temp.path().join("legacy-home");
    fs::create_dir_all(legacy.join("blobs")).expect("blobs");
    let bytes = vec![b'x'; 70 * 1024];
    let sha = hex::encode(Sha256::digest(&bytes));
    let blob_rel = format!("{}/{}", &sha[..2], &sha[2..]);
    let blob_path = legacy.join("blobs").join(&blob_rel);
    fs::create_dir_all(blob_path.parent().expect("blob parent")).expect("blob parent");
    fs::write(&blob_path, &bytes).expect("blob");
    let conn = Connection::open(legacy.join("distill.db")).expect("legacy db");
    conn.execute_batch(
        "CREATE TABLE captures (
            id INTEGER PRIMARY KEY,
            source_id INTEGER NOT NULL,
            source_path TEXT,
            external_session_id TEXT,
            source_modified_at TEXT,
            raw_sha256 TEXT NOT NULL,
            raw_blob_path TEXT,
            raw_payload_json TEXT,
            status TEXT NOT NULL
         );",
    )
    .expect("captures schema");
    conn.execute(
        "INSERT INTO captures (
            source_id, source_path, external_session_id, raw_sha256,
            raw_blob_path, status
         ) VALUES (1, 'large.jsonl', 'broken', ?1, ?2, 'captured')",
        rusqlite::params![sha, blob_rel],
    )
    .expect("capture");
    drop(conn);

    let dest = temp.path().join("native-home");
    let mut library = Library::open(&dest).expect("native");
    assert!(library.import_legacy_electron_home(&legacy).is_err());
    assert!(
        !dest.join("blobs").join(&sha[..2]).join(&sha[2..]).exists(),
        "a failed transaction must not leave an unreferenced CAS blob"
    );
}
