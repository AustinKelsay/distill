//! Library ingest fault-injection reopen contracts for issue #21.
//!
//! Requires `--features test-faults`. Interrupts real ingest boundaries, reopens the
//! Distill home, and asserts durable state, health classification, and repair.

#![cfg(feature = "test-faults")]

use std::fs;
use std::path::{Path, PathBuf};

use distill_library::faults::{self, FaultPoint};
use distill_library::{Library, RepairOptions, INLINE_CONTENT_THRESHOLD_BYTES};
use rusqlite::Connection;
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
      "title": "Fault Fixture"
    }}
  ]
}}"#
    );
    fs::write(root.join("distill.fixture.json"), manifest).expect("write manifest");
    capture_path
}

/**
 * Inline Fixture body for Attempt/projection fault cases.
 */
fn inline_body() -> String {
    concat!(
        r#"{"record_type":"session_meta","title":"Fault Fixture","summary":"fault"}"#,
        "\n",
        r#"{"record_type":"message","role":"user","text":"fault user"}"#,
        "\n",
        r#"{"record_type":"message","role":"assistant","text":"fault assistant"}"#,
        "\n",
    )
    .to_string()
}

/**
 * Blob-backed Fixture body for staging/rename fault cases.
 */
fn blob_body() -> String {
    let mut body = inline_body();
    body.push_str(
        &serde_json::json!({
            "record_type": "file",
            "text": "large fault artifact",
            "payload": "z".repeat(INLINE_CONTENT_THRESHOLD_BYTES as usize)
        })
        .to_string(),
    );
    body.push('\n');
    body
}

/**
 * Count Capture rows in a Distill home database.
 */
fn capture_count(home: &Path) -> i64 {
    let conn = Connection::open(home.join("distill.db")).expect("db");
    conn.query_row("SELECT COUNT(*) FROM captures", [], |row| row.get(0))
        .expect("count")
}

/**
 * Count Attempts by outcome.
 */
fn attempt_count(home: &Path, outcome: &str) -> i64 {
    let conn = Connection::open(home.join("distill.db")).expect("db");
    conn.query_row(
        "SELECT COUNT(*) FROM normalization_attempts WHERE outcome = ?1",
        [outcome],
        |row| row.get(0),
    )
    .expect("count")
}

/**
 * Count Activity Events by type.
 */
fn activity_count(home: &Path, event_type: &str) -> i64 {
    let conn = Connection::open(home.join("distill.db")).expect("db");
    conn.query_row(
        "SELECT COUNT(*) FROM activity_events WHERE event_type = ?1",
        [event_type],
        |row| row.get(0),
    )
    .expect("count")
}

/**
 * Count staging partial files.
 */
fn staging_partial_count(home: &Path) -> usize {
    let staging = home.join("staging");
    if !staging.is_dir() {
        return 0;
    }
    fs::read_dir(staging)
        .expect("staging")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".partial"))
        })
        .count()
}

/**
 * Count CAS files under blobs/.
 */
fn cas_file_count(home: &Path) -> usize {
    let blobs = home.join("blobs");
    if !blobs.is_dir() {
        return 0;
    }
    let mut count = 0usize;
    fn walk(dir: &Path, count: &mut usize) {
        for entry in fs::read_dir(dir).expect("read") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.is_dir() {
                walk(&path, count);
            } else if path.is_file() {
                *count += 1;
            }
        }
    }
    walk(&blobs, &mut count);
    count
}

/**
 * FIR-001: stage write before rename leaves only a disposable partial.
 */
#[test]
fn fault_after_stage_write_before_rename() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-stage",
        "captures/hello.jsonl",
        &blob_body(),
    );

    let mut library = Library::open(&home).expect("open");
    faults::arm(FaultPoint::AfterStageWriteBeforeRename);
    let err = library
        .ingest_fixture(&fixture)
        .expect_err("fault should fire");
    assert!(err.to_string().contains("injected test fault"));
    faults::clear();
    drop(library);

    assert_eq!(capture_count(&home), 0);
    assert_eq!(cas_file_count(&home), 0);
    assert_eq!(staging_partial_count(&home), 1);

    let library = Library::open(&home).expect("reopen");
    assert_eq!(library.open_reconciliation().removed_staging_partials, 1);
    assert_eq!(staging_partial_count(&home), 0);
    let health = library.health().expect("health");
    assert!(health.ok, "{:?}", health.issues);
}

/**
 * FIR-002: rename before Capture acceptance leaves an unreferenced CAS blob.
 */
#[test]
fn fault_after_blob_rename_before_capture_accept() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-rename",
        "captures/hello.jsonl",
        &blob_body(),
    );

    let mut library = Library::open(&home).expect("open");
    faults::arm(FaultPoint::AfterBlobRenameBeforeCaptureAccept);
    let _ = library.ingest_fixture(&fixture).expect_err("fault");
    faults::clear();
    drop(library);

    assert_eq!(capture_count(&home), 0);
    assert_eq!(cas_file_count(&home), 1);
    assert_eq!(staging_partial_count(&home), 0);

    let mut library = Library::open(&home).expect("reopen");
    let health = library.health().expect("health");
    assert!(!health.ok);
    assert_eq!(health.orphan_status, "failed");
    assert!(health
        .issues
        .iter()
        .any(|issue| issue.code == "orphan_blob"));

    let repaired = library
        .repair(RepairOptions {
            remove_orphan_blobs: true,
            resolve_incomplete_state: false,
            rebuild_fts: false,
        })
        .expect("repair");
    assert!(repaired
        .actions
        .iter()
        .any(|action| action.name == "removed_orphan_blobs" && action.count == 1));
    assert!(
        repaired.health_after.ok,
        "{:?}",
        repaired.health_after.issues
    );
    assert_eq!(cas_file_count(&home), 0);
}

/**
 * FIR-003: Capture insert before capture_recorded rolls back the shared SQLite tx.
 */
#[test]
fn fault_after_capture_insert_before_activity_rolls_back() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-capture-tx",
        "captures/hello.jsonl",
        &inline_body(),
    );

    let mut library = Library::open(&home).expect("open");
    faults::arm(FaultPoint::AfterCaptureInsertBeforeActivity);
    let _ = library.ingest_fixture(&fixture).expect_err("fault");
    faults::clear();
    drop(library);

    // Capture + capture_recorded commit together; interrupting mid-tx leaves nothing.
    assert_eq!(capture_count(&home), 0);
    assert_eq!(activity_count(&home, "capture_recorded"), 0);
    let library = Library::open(&home).expect("reopen");
    assert!(library.health().expect("health").ok);
}

/**
 * FIR-004: after Capture+Activity commit before Attempt leaves incomplete Capture.
 */
#[test]
fn fault_after_capture_recorded_before_attempt() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-before-attempt",
        "captures/hello.jsonl",
        &inline_body(),
    );

    let mut library = Library::open(&home).expect("open");
    faults::arm(FaultPoint::AfterCaptureRecordedBeforeAttempt);
    let _ = library.ingest_fixture(&fixture).expect_err("fault");
    faults::clear();
    drop(library);

    assert_eq!(capture_count(&home), 1);
    assert_eq!(activity_count(&home, "capture_recorded"), 1);
    assert_eq!(attempt_count(&home, "pending"), 0);
    assert_eq!(attempt_count(&home, "succeeded"), 0);

    let mut library = Library::open(&home).expect("reopen");
    let health = library.health().expect("health");
    assert!(!health.ok);
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
        .any(|action| action.name == "appended_capture_failed_recoveries" && action.count == 1));
    assert_eq!(repaired.health_after.incomplete_status, "ok");
    assert_eq!(attempt_count(&home, "pending"), 0);
    assert_eq!(attempt_count(&home, "succeeded"), 0);
    assert_eq!(attempt_count(&home, "failed"), 0);
    assert_eq!(activity_count(&home, "capture_failed"), 1);
}

/**
 * Seed a successful projection, then interrupt a replacement Attempt mid-publish.
 */
fn seed_then_fault_replacement(point: FaultPoint) -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-replace",
        "captures/hello.jsonl",
        &inline_body(),
    );

    {
        let mut library = Library::open(&home).expect("open");
        library.ingest_fixture(&fixture).expect("seed ingest");
    }

    // Change bytes so the next ingest creates a new Capture and Attempt.
    let capture_path = fixture.join("captures/hello.jsonl");
    let mut changed = inline_body();
    changed.push_str(
        r#"{"record_type":"message","role":"user","text":"replacement user"}
"#,
    );
    fs::write(&capture_path, changed).expect("change capture");

    let mut library = Library::open(&home).expect("reopen");
    faults::arm(point);
    let _ = library.ingest_fixture(&fixture).expect_err("fault");
    faults::clear();
    drop(library);
    (temp, home)
}

/**
 * FIR-005: pending Attempt before publish remains pending; last-good projection kept.
 */
#[test]
fn fault_after_pending_attempt_before_publish() {
    let (_temp, home) = seed_then_fault_replacement(FaultPoint::AfterPendingAttemptBeforePublish);

    assert_eq!(capture_count(&home), 2);
    assert_eq!(attempt_count(&home, "pending"), 1);
    assert_eq!(attempt_count(&home, "succeeded"), 1);
    assert_eq!(activity_count(&home, "projection_replaced"), 1);

    let mut library = Library::open(&home).expect("reopen");
    let detail = library
        .session_slice("fixture", "fixture-session-replace", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(detail.summary.successful_projection_generation, 1);
    assert_eq!(detail.messages.len(), 2);
    assert!(detail.messages.iter().all(|m| m.text != "replacement user"));

    let health = library.health().expect("health");
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
        .any(|action| action.name == "failed_pending_attempts" && action.count == 1));
    assert_eq!(attempt_count(&home, "pending"), 0);
}

/**
 * Assert mid-projection SQLite faults roll back and preserve last-good projection.
 */
fn assert_mid_publish_rollback(point: FaultPoint) {
    let (_temp, home) = seed_then_fault_replacement(point);

    assert_eq!(capture_count(&home), 2);
    assert_eq!(attempt_count(&home, "pending"), 1);
    assert_eq!(attempt_count(&home, "succeeded"), 1);
    assert_eq!(activity_count(&home, "projection_replaced"), 1);

    let library = Library::open(&home).expect("reopen");
    let detail = library
        .session_slice("fixture", "fixture-session-replace", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(detail.summary.successful_projection_generation, 1);
    assert_eq!(detail.messages[0].text, "fault user");
    assert!(library
        .health()
        .expect("health")
        .issues
        .iter()
        .any(|i| i.code == "pending_attempt"));
}

/**
 * FIR-006: Fact/projection rows before FTS roll back with the publication tx.
 */
#[test]
fn fault_during_publish_after_facts_before_fts() {
    assert_mid_publish_rollback(FaultPoint::DuringPublishAfterFactsBeforeFts);
}

/**
 * FIR-007: after FTS before Attempt success/Activity rolls back the publication tx.
 */
#[test]
fn fault_during_publish_after_fts_before_attempt_success() {
    assert_mid_publish_rollback(FaultPoint::DuringPublishAfterFtsBeforeAttemptSuccess);
}

/**
 * FIR-008: after projection_replaced before commit rolls back the publication tx.
 */
#[test]
fn fault_during_publish_after_activity_before_commit() {
    assert_mid_publish_rollback(FaultPoint::DuringPublishAfterActivityBeforeCommit);
}
