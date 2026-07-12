//! Library health, open reconciliation, and explicit repair contracts for issue #21.
//!
//! Public-seam TDD over real temporary Distill homes. Sync-run stale operations are
//! reported as `operations_status = not_applicable` until issue #22.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use distill_library::{Library, RepairOptions, INLINE_CONTENT_THRESHOLD_BYTES};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

/**
 * Write a Fixture root with one file-backed Capture Candidate.
 */
fn write_fixture(root: &Path, session_id: &str, relative: &str, body: &str) -> PathBuf {
    let capture_path = root.join(relative);
    if let Some(parent) = capture_path.parent() {
        fs::create_dir_all(parent).expect("capture parent");
    }
    fs::write(&capture_path, body).expect("write capture");
    let id = Path::new(relative)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("capture");
    let manifest = format!(
        r#"{{
  "version": 1,
  "captures": [
    {{
      "id": "{id}",
      "kind": "file",
      "relative_path": "{relative}",
      "external_session_id": "{session_id}",
      "title": "Health Repair Fixture"
    }}
  ]
}}"#
    );
    fs::write(root.join("distill.fixture.json"), manifest).expect("write manifest");
    capture_path
}

/**
 * Baseline multi-message Fixture body.
 */
fn rich_body() -> String {
    concat!(
        r#"{"record_type":"session_meta","title":"Health Repair Fixture","summary":"baseline"}"#,
        "\n",
        r#"{"record_type":"message","role":"user","text":"health user"}"#,
        "\n",
        r#"{"record_type":"message","role":"assistant","text":"health assistant"}"#,
        "\n",
    )
    .to_string()
}

/**
 * Metadata-only empty projection body (valid successful generation with zero messages).
 */
fn empty_projection_body() -> String {
    concat!(
        r#"{"record_type":"session_meta","title":"Empty Projection","summary":"metadata only"}"#,
        "\n",
    )
    .to_string()
}

/**
 * Blob-backed Fixture body above the inline threshold.
 */
fn blob_body() -> String {
    let mut body = rich_body();
    body.push_str(
        &serde_json::json!({
            "record_type": "file",
            "text": "large health artifact",
            "payload": "y".repeat(INLINE_CONTENT_THRESHOLD_BYTES as usize)
        })
        .to_string(),
    );
    body.push('\n');
    body
}

/**
 * LHR-001: healthy Fixture home reports ok across every health category.
 */
#[test]
fn healthy_home_reports_all_category_status_ok() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-health",
        "captures/hello.jsonl",
        &rich_body(),
    );

    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("ingest");
    let health = library.health().expect("health");

    assert!(health.ok, "{:?}", health.issues);
    assert_eq!(health.schema_status, "ok");
    assert_eq!(health.content_status, "ok");
    assert_eq!(health.fts_status, "ok");
    assert_eq!(health.staging_status, "ok");
    assert_eq!(health.orphan_status, "ok");
    assert_eq!(health.incomplete_status, "ok");
    assert_eq!(health.operations_status, "not_applicable");
    assert!(health.issues.is_empty());
    assert_eq!(health.open_reconciliation.removed_staging_partials, 0);
}

/**
 * LHR-002: open removes only canonical `{64 lowercase hex}.partial` staging files.
 */
#[test]
fn open_reconciles_staging_partials_only() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-staging",
        "captures/hello.jsonl",
        &rich_body(),
    );

    {
        let mut library = Library::open(&home).expect("open");
        library.ingest_fixture(&fixture).expect("ingest");
    }

    let staging = home.join("staging");
    fs::create_dir_all(&staging).expect("staging");
    let partial = staging.join(format!("{}.partial", "a".repeat(64)));
    fs::write(&partial, b"disposable").expect("partial");
    let orphan_marker = home.join("blobs").join("zz").join("keep-me-not-partial");
    // Ensure reopen does not invent unrelated filesystem deletes.
    assert!(!orphan_marker.exists());

    let library = Library::open(&home).expect("reopen");
    assert_eq!(library.open_reconciliation().removed_staging_partials, 1);
    assert!(!partial.exists(), "staging partial must be removed on open");
    let health = library.health().expect("health");
    assert!(health.ok, "{:?}", health.issues);
    assert_eq!(health.open_reconciliation.removed_staging_partials, 1);
}

/**
 * LHR-002b: noncanonical staging partials remain and are reported, never silently deleted.
 */
#[test]
fn noncanonical_staging_partials_are_reported_not_deleted() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let library = Library::open(&home).expect("open");
    drop(library);

    let staging = home.join("staging");
    fs::create_dir_all(&staging).expect("staging");
    let weird = staging.join("not-a-hash.partial");
    let upper = staging.join(format!("{}.partial", "A".repeat(64)));
    fs::write(&weird, b"keep").expect("weird");
    fs::write(&upper, b"keep").expect("upper");

    let library = Library::open(&home).expect("reopen");
    assert_eq!(library.open_reconciliation().removed_staging_partials, 0);
    assert!(weird.exists());
    assert!(upper.exists());

    let health = library.health().expect("health");
    assert!(!health.ok);
    assert_eq!(health.staging_status, "failed");
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "unrecognized_staging_entry"));
}

/**
 * LHR-002c: a symlinked staging root is blocking and never traversed by health/repair/open.
 */
#[test]
fn symlinked_staging_root_never_touches_external_files() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let outside = temp.path().join("outside-staging");
    fs::create_dir_all(&outside).expect("outside");
    let external_partial = outside.join(format!("{}.partial", "a".repeat(64)));
    fs::write(&external_partial, b"do-not-delete").expect("external partial");

    let mut library = Library::open(&home).expect("open");
    fs::remove_dir(home.join("staging")).expect("remove staging");
    symlink(&outside, home.join("staging")).expect("staging symlink");

    let health = library.health().expect("health");
    assert!(!health.ok);
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "unsafe_staging_root" && issue.severity == "blocking"));

    let repaired = library
        .repair(RepairOptions::all_documented())
        .expect("repair");
    assert!(!repaired.health_after.ok);
    assert_eq!(
        fs::read(&external_partial).expect("external partial remains"),
        b"do-not-delete"
    );
    drop(library);

    let err = match Library::open(&home) {
        Ok(_) => panic!("reopen must reject unsafe layout"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("unsafe directory entry"));
    assert!(external_partial.exists());
}

/**
 * LHR-003: orphan CAS blobs are health issues and require explicit repair.
 */
#[test]
fn orphan_blob_requires_explicit_repair_and_is_idempotent() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-orphan",
        "captures/hello.jsonl",
        &blob_body(),
    );

    let mut library = Library::open(&home).expect("open");
    let report = library.ingest_fixture(&fixture).expect("ingest");
    assert_eq!(report.accepted_captures, 1);

    let orphan_dir = home.join("blobs").join("ff");
    fs::create_dir_all(&orphan_dir).expect("orphan dir");
    let orphan_name = format!("deadbeef{}", "0".repeat(54));
    assert_eq!(orphan_name.len(), 62);
    let orphan = orphan_dir.join(&orphan_name);
    fs::write(&orphan, b"orphan-bytes").expect("orphan blob");

    let health = library.health().expect("health");
    assert!(!health.ok);
    assert_eq!(health.orphan_status, "failed");
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "orphan_blob" && issue.category == "orphan"));
    assert!(
        !health
            .issues
            .iter()
            .any(|issue| issue.summary.contains(orphan.to_string_lossy().as_ref())),
        "health must not leak raw orphan paths"
    );

    // Open must not delete the orphan.
    drop(library);
    let mut library = Library::open(&home).expect("reopen");
    assert!(orphan.exists(), "open must not delete orphan CAS blobs");

    let repaired = library
        .repair(RepairOptions {
            remove_orphan_blobs: true,
            resolve_incomplete_state: false,
            rebuild_fts: false,
        })
        .expect("repair");
    let removed = repaired
        .actions
        .iter()
        .find(|action| action.name == "removed_orphan_blobs")
        .expect("removed_orphan_blobs action");
    assert_eq!(removed.count, 1);
    assert!(!orphan.exists());
    assert!(
        repaired.health_after.ok,
        "{:?}",
        repaired.health_after.issues
    );

    let again = library
        .repair(RepairOptions {
            remove_orphan_blobs: true,
            resolve_incomplete_state: false,
            rebuild_fts: false,
        })
        .expect("second repair");
    let removed_again = again
        .actions
        .iter()
        .find(|action| action.name == "removed_orphan_blobs")
        .expect("action");
    assert_eq!(removed_again.count, 0, "repair must be idempotent");
}

/**
 * LHR-003b: CAS symlinks and malicious blob_path never read/delete external targets.
 */
#[test]
fn cas_symlink_and_malicious_blob_path_never_touch_external_targets() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let outside = temp.path().join("outside-secret");
    fs::write(&outside, b"do-not-delete").expect("outside");
    let outside_dir = temp.path().join("outside-dir");
    fs::create_dir_all(&outside_dir).expect("outside dir");
    fs::write(outside_dir.join("nested"), b"nested").expect("nested");
    let external_cas_name = "d".repeat(62);
    fs::write(outside_dir.join(&external_cas_name), b"external-cas").expect("external cas");

    let library = Library::open(&home).expect("open");
    drop(library);

    let link_dir = home.join("blobs").join("aa");
    fs::create_dir_all(&link_dir).expect("link dir");
    let file_link = link_dir.join("b".repeat(62));
    let dir_link = home.join("blobs").join("cc");
    symlink(&outside, &file_link).expect("file symlink");
    symlink(&outside_dir, &dir_link).expect("dir symlink");

    let db = home.join("distill.db");
    let conn = Connection::open(&db).expect("db");
    conn.execute(
        "INSERT INTO sources (kind, display_name, data_root, metadata_json, created_at, updated_at)
         VALUES ('fixture', 'Fixture', ?1, '{}', ?2, ?2)",
        rusqlite::params![
            home.to_string_lossy().as_ref(),
            chrono::Utc::now().to_rfc3339()
        ],
    )
    .expect("source");
    let malicious_paths = [
        outside.to_string_lossy().to_string(),
        "../../outside-secret".to_string(),
        "blobs/../staging/escape".to_string(),
        format!("blobs/cc/{external_cas_name}"),
    ];
    for (idx, blob_path) in malicious_paths.iter().enumerate() {
        let (sha256, byte_size) = if blob_path.starts_with("blobs/cc/") {
            (hex::encode(Sha256::digest(b"external-cas")), 12)
        } else {
            (hex::encode(Sha256::digest(b"do-not-delete")), 13)
        };
        conn.execute(
            "INSERT INTO captures (
                source_id, source_kind, source_path, external_session_id,
                content_kind, media_type, sha256, byte_size, inline_text, blob_path,
                source_modified_at, accepted_at
             ) VALUES (
                1, 'fixture', ?1, 'malicious-session',
                'blob', 'application/octet-stream', ?2, ?3, NULL, ?4, NULL, ?5
             )",
            rusqlite::params![
                format!("captures/evil-{idx}.bin"),
                sha256,
                byte_size,
                blob_path.clone(),
                chrono::Utc::now().to_rfc3339(),
            ],
        )
        .expect("malicious capture");
    }
    drop(conn);

    let mut library = Library::open(&home).expect("reopen");
    let health = library.health().expect("health");
    assert!(!health.ok);
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "cas_unrecognized_entry"));
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "content_invalid_blob_path"));
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "content_symlink_blob"));
    assert!(
        !health
            .issues
            .iter()
            .any(|issue| issue.summary.contains(outside.to_string_lossy().as_ref())),
        "health must not leak external paths"
    );

    let _ = library
        .repair(RepairOptions::all_documented())
        .expect("repair");
    assert!(
        outside.exists(),
        "repair must never delete external symlink targets"
    );
    assert!(
        outside_dir.join("nested").exists(),
        "repair must never delete external symlink directories"
    );
    assert_eq!(
        fs::read(outside_dir.join(&external_cas_name)).expect("read external cas"),
        b"external-cas"
    );
    assert_eq!(fs::read(&outside).expect("read outside"), b"do-not-delete");
}

/**
 * LHR-004: FTS identity/content disagreement is detected (not count-only) and repaired.
 */
#[test]
fn fts_content_mismatch_is_detected_and_rebuilt() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-fts",
        "captures/hello.jsonl",
        &rich_body(),
    );

    {
        let mut library = Library::open(&home).expect("open");
        library.ingest_fixture(&fixture).expect("ingest");
    }

    let db = home.join("distill.db");
    let conn = Connection::open(&db).expect("db");
    conn.execute(
        "UPDATE projection_fts SET text = 'tampered fts text' WHERE rowid = (
            SELECT rowid FROM projection_fts LIMIT 1
         )",
        [],
    )
    .expect("tamper fts");
    drop(conn);

    let mut library = Library::open(&home).expect("reopen");
    let health = library.health().expect("health");
    assert!(!health.ok);
    assert_eq!(health.fts_status, "failed");
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "fts_projection_mismatch"));

    let repaired = library
        .repair(RepairOptions {
            remove_orphan_blobs: false,
            resolve_incomplete_state: false,
            rebuild_fts: true,
        })
        .expect("repair");
    assert!(repaired
        .actions
        .iter()
        .any(|action| action.name == "rebuilt_fts_rows" && action.count >= 1));
    assert!(
        repaired.health_after.ok,
        "{:?}",
        repaired.health_after.issues
    );
    assert_eq!(repaired.health_after.fts_status, "ok");
}

/**
 * LHR-004b: title/project_path FTS fields are compared and restored from Session.
 */
#[test]
fn fts_title_and_project_path_mismatch_is_detected_and_rebuilt() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-fts-path",
        "captures/hello.jsonl",
        &rich_body(),
    );

    {
        let mut library = Library::open(&home).expect("open");
        library.ingest_fixture(&fixture).expect("ingest");
    }

    let db = home.join("distill.db");
    let conn = Connection::open(&db).expect("db");
    conn.execute(
        "UPDATE sessions SET project_path = '/workspace/demo' WHERE id = 1",
        [],
    )
    .expect("set project_path");
    conn.execute(
        "UPDATE projection_fts
         SET title = 'tampered title', project_path = '/wrong/path'
         WHERE rowid = (SELECT rowid FROM projection_fts LIMIT 1)",
        [],
    )
    .expect("tamper title/path");
    drop(conn);

    let mut library = Library::open(&home).expect("reopen");
    let health = library.health().expect("health");
    assert!(!health.ok);
    assert_eq!(health.fts_status, "failed");
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "fts_projection_mismatch"));

    let repaired = library
        .repair(RepairOptions {
            remove_orphan_blobs: false,
            resolve_incomplete_state: false,
            rebuild_fts: true,
        })
        .expect("repair");
    assert!(
        repaired.health_after.ok,
        "{:?}",
        repaired.health_after.issues
    );

    let conn = Connection::open(&db).expect("db");
    let restored: (String, String) = conn
        .query_row(
            "SELECT title, project_path FROM projection_fts LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("fts row");
    assert_eq!(restored.0, "Health Repair Fixture");
    assert_eq!(restored.1, "/workspace/demo");
}

/**
 * LHR-005: incomplete Capture/pending Attempt states resolve via capture_failed, not fake Attempts.
 */
#[test]
fn incomplete_capture_and_pending_attempt_are_repairable() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-incomplete",
        "captures/hello.jsonl",
        &rich_body(),
    );

    {
        let mut library = Library::open(&home).expect("open");
        library.ingest_fixture(&fixture).expect("ingest");
    }

    let db = home.join("distill.db");
    let conn = Connection::open(&db).expect("db");
    // Simulate Capture accepted without Attempt.
    conn.execute(
        "INSERT INTO captures (
            source_id, source_kind, source_path, external_session_id,
            content_kind, media_type, sha256, byte_size, inline_text, blob_path,
            source_modified_at, accepted_at
         ) VALUES (
            1, 'fixture', 'captures/orphan-capture.jsonl', 'fixture-session-incomplete',
            'inline', 'application/json', ?1, 2, '{}', NULL, NULL, ?2
         )",
        [
            hex::encode(Sha256::digest(b"{}")),
            chrono::Utc::now().to_rfc3339(),
        ],
    )
    .expect("incomplete capture");
    conn.execute(
        "INSERT INTO normalization_attempts (
            capture_id, parser_id, parser_version, started_at, finished_at,
            outcome, error_class, error_message, metrics_json
         ) VALUES (1, 'fixture', '1.0.0', ?1, NULL, 'pending', NULL, NULL, '{}')",
        [chrono::Utc::now().to_rfc3339()],
    )
    .expect("pending attempt");
    drop(conn);

    let mut library = Library::open(&home).expect("reopen");
    let health = library.health().expect("health");
    assert!(!health.ok);
    assert_eq!(health.incomplete_status, "failed");
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "incomplete_capture"));
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "pending_attempt"));

    let repaired = library
        .repair(RepairOptions {
            remove_orphan_blobs: false,
            resolve_incomplete_state: true,
            rebuild_fts: false,
        })
        .expect("repair");
    assert!(repaired
        .actions
        .iter()
        .any(|action| action.name == "failed_pending_attempts" && action.count >= 1));
    assert!(repaired
        .actions
        .iter()
        .any(|action| action.name == "appended_capture_failed_recoveries" && action.count >= 1));
    assert_eq!(repaired.health_after.incomplete_status, "ok");

    let conn = Connection::open(&db).expect("db");
    let repair_attempts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM normalization_attempts WHERE parser_id = 'repair'",
            [],
            |row| row.get(0),
        )
        .expect("repair attempts");
    assert_eq!(
        repair_attempts, 0,
        "repair must not invent Normalization Attempts"
    );
    let failed_events: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM activity_events
             WHERE event_type = 'capture_failed' AND capture_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("capture_failed");
    assert!(failed_events >= 1);

    let again = library
        .repair(RepairOptions::all_documented())
        .expect("idempotent repair");
    let pending = again
        .actions
        .iter()
        .find(|action| action.name == "failed_pending_attempts")
        .expect("pending action");
    let recoveries = again
        .actions
        .iter()
        .find(|action| action.name == "appended_capture_failed_recoveries")
        .expect("recovery action");
    assert_eq!(pending.count, 0);
    assert_eq!(recoveries.count, 0);

    // Exact duplicate ingest remains inert after capture_failed recovery.
    let mut library = Library::open(&home).expect("reopen for dupe");
    let report = library.ingest_fixture(&fixture).expect("duplicate ingest");
    assert_eq!(report.accepted_captures, 0);
    assert_eq!(report.skipped_duplicates, 1);
}

/**
 * LHR-005b: metadata-only empty successful projections remain healthy (#20).
 */
#[test]
fn empty_successful_projection_remains_healthy() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-empty",
        "captures/hello.jsonl",
        &empty_projection_body(),
    );

    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("ingest");
    let detail = library
        .session_slice("fixture", "fixture-session-empty", 20, 20)
        .expect("session")
        .expect("present");
    assert!(detail.messages.is_empty());
    assert_eq!(detail.summary.successful_projection_generation, 1);
    assert!(detail.summary.title.as_deref() == Some("Empty Projection"));

    let health = library.health().expect("health");
    assert!(health.ok, "{:?}", health.issues);
    assert_eq!(health.incomplete_status, "ok");
    assert!(!health
        .issues
        .iter()
        .any(|issue| issue.code == "incomplete_projection"));
}

/**
 * LHR-005c: changed-Capture counter drift is detected and repaired while keeping last-good generation.
 */
#[test]
fn session_counter_mismatch_is_detected_and_repaired() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-counters",
        "captures/hello.jsonl",
        &rich_body(),
    );

    {
        let mut library = Library::open(&home).expect("open");
        library.ingest_fixture(&fixture).expect("ingest");
    }

    let db = home.join("distill.db");
    let conn = Connection::open(&db).expect("db");
    let generation_before: i64 = conn
        .query_row(
            "SELECT successful_projection_generation FROM sessions WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("generation");
    conn.execute(
        "INSERT INTO captures (
            source_id, source_kind, source_path, external_session_id,
            content_kind, media_type, sha256, byte_size, inline_text, blob_path,
            source_modified_at, accepted_at
         ) VALUES (
            1, 'fixture', 'captures/changed.jsonl', 'fixture-session-counters',
            'inline', 'application/json', ?1, 2, '{}', NULL, NULL, ?2
         )",
        [
            hex::encode(Sha256::digest(b"{}")),
            chrono::Utc::now().to_rfc3339(),
        ],
    )
    .expect("changed capture");
    // Leave materialized counters stale and leave the Capture unresolved.
    drop(conn);

    let mut library = Library::open(&home).expect("reopen");
    let health = library.health().expect("health");
    assert!(!health.ok);
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "session_counter_mismatch"));
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "incomplete_capture"));

    let repaired = library
        .repair(RepairOptions {
            remove_orphan_blobs: false,
            resolve_incomplete_state: true,
            rebuild_fts: false,
        })
        .expect("repair");
    assert!(repaired
        .actions
        .iter()
        .any(|action| action.name == "recomputed_session_counters" && action.count >= 1));
    assert!(
        repaired.health_after.ok,
        "{:?}",
        repaired.health_after.issues
    );

    let conn = Connection::open(&db).expect("db");
    let (captures, attempts, generation): (i64, i64, i64) = conn
        .query_row(
            "SELECT accepted_capture_count, normalization_attempt_count,
                    successful_projection_generation
             FROM sessions WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("counters");
    assert_eq!(captures, 2);
    assert_eq!(attempts, 1);
    assert_eq!(generation, generation_before);
}

/**
 * LHR-006: referenced content corruption is blocking and never deleted by repair.
 */
#[test]
fn missing_referenced_blob_is_blocking_and_not_deleted_by_repair() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-missing",
        "captures/hello.jsonl",
        &blob_body(),
    );

    let mut library = Library::open(&home).expect("open");
    let report = library.ingest_fixture(&fixture).expect("ingest");
    let capture_id = report.capture_ids[0];
    let bytes = library.replay_capture(capture_id).expect("replay");
    let sha = hex::encode(Sha256::digest(&bytes));
    let relative = format!("blobs/{}/{}", &sha[..2], &sha[2..]);
    let absolute = home.join(&relative);
    assert!(absolute.is_file());
    fs::remove_file(&absolute).expect("delete referenced blob");

    let health = library.health().expect("health");
    assert!(!health.ok);
    assert_eq!(health.content_status, "failed");
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "content_missing" && issue.severity == "blocking"));

    let repaired = library
        .repair(RepairOptions::all_documented())
        .expect("repair");
    assert!(
        !repaired.health_after.ok,
        "repair must not pretend missing referenced content is fixed"
    );
    assert_eq!(repaired.health_after.content_status, "failed");
}

/**
 * LHR-007: operations status is explicit not_applicable until #22; no invented sync codes.
 */
#[test]
fn health_reports_operations_status_not_applicable_until_sync_jobs() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let library = Library::open(&home).expect("open");
    let health = library.health().expect("health");
    assert_eq!(health.operations_status, "not_applicable");
    assert!(health
        .issues
        .iter()
        .all(|issue| issue.code != "stale_sync_operation"
            && issue.code != "stale_job"
            && issue.category != "sync"));
}
