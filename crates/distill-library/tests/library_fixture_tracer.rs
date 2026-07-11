//! Library Fixture tracer contract: one public-seam journey for issue #18.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use distill_library::{Library, LibraryError, INLINE_CONTENT_THRESHOLD_BYTES};
use tempfile::TempDir;

/**
 * Write a minimal Fixture root with one file-backed Capture Candidate.
 */
fn write_basic_fixture(root: &Path) -> PathBuf {
    let captures = root.join("captures");
    fs::create_dir_all(&captures).expect("captures dir");
    let capture_path = captures.join("hello.jsonl");
    let mut body = concat!(
        r#"{"record_type":"session_meta","title":"Fixture Hello","summary":"tracer","metadata":{"fixture":true}}"#,
        "\n",
        r#"{"record_type":"message","role":"user","text":"Hello from fixture"}"#,
        "\n",
        r#"{"record_type":"message","role":"assistant","text":"Greeting acknowledged"}"#,
        "\n",
        r#"{"record_type":"tool_use","text":"echo hi","payload":{"command":"echo"}}"#,
        "\n",
        r#"{"record_type":"reasoning","text":"consider greeting"}"#,
        "\n",
    )
    .to_string();
    body.push_str(
        &serde_json::json!({
            "record_type": "file",
            "text": "large fixture artifact",
            "payload": "x".repeat(INLINE_CONTENT_THRESHOLD_BYTES as usize)
        })
        .to_string(),
    );
    body.push('\n');
    fs::write(&capture_path, body).expect("write capture");
    let manifest = r#"{
  "version": 1,
  "captures": [
    {
      "id": "hello",
      "kind": "file",
      "relative_path": "captures/hello.jsonl",
      "external_session_id": "fixture-session-hello",
      "title": "Fixture Hello"
    }
  ]
}"#;
    fs::write(root.join("distill.fixture.json"), manifest).expect("write manifest");
    capture_path
}

#[test]
fn library_fixture_tracer_journey() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("distill-home");
    let fixture = temp.path().join("fixture-root");
    fs::create_dir_all(&fixture).expect("fixture root");
    let source_capture = write_basic_fixture(&fixture);

    let mut library = Library::open(&home).expect("open library");

    let home_mode = fs::metadata(&home).expect("home meta").permissions().mode() & 0o777;
    assert_eq!(home_mode, 0o700, "Distill home must be 0o700");
    let db_mode = fs::metadata(home.join("distill.db"))
        .expect("db meta")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(db_mode, 0o600, "database must be 0o600");

    let report = library.ingest_fixture(&fixture).expect("ingest fixture");
    assert_eq!(report.accepted_captures, 1);
    assert_eq!(report.successful_attempts, 1);
    assert_eq!(report.failed_attempts, 0);
    assert_eq!(report.capture_ids.len(), 1);

    let detail = library
        .session_slice("fixture", "fixture-session-hello", 20, 20)
        .expect("session query")
        .expect("session present");
    assert_eq!(detail.summary.source_kind, "fixture");
    assert_eq!(detail.summary.external_session_id, "fixture-session-hello");
    assert_eq!(detail.summary.accepted_capture_count, 1);
    assert_eq!(detail.summary.normalization_attempt_count, 1);
    assert_eq!(detail.summary.successful_projection_generation, 1);
    assert_eq!(detail.messages.len(), 2);
    assert_eq!(detail.messages[0].role, "user");
    assert_eq!(detail.messages[0].text, "Hello from fixture");
    assert_eq!(detail.messages[1].role, "assistant");
    assert!(detail.artifacts.len() >= 2);

    let hits = library.search("Hello", 20).expect("search");
    assert!(
        hits.iter()
            .any(|hit| hit.text.contains("Hello from fixture")),
        "search should find projected transcript text"
    );

    let activity = library.recent_activity(20).expect("activity");
    assert!(activity
        .iter()
        .any(|event| event.event_type == "capture_recorded"));
    assert!(activity
        .iter()
        .any(|event| event.event_type == "projection_replaced"));

    let capture_id = report.capture_ids[0];
    let replayed = library.replay_capture(capture_id).expect("replay");
    assert!(
        replayed.len() as u64 > INLINE_CONTENT_THRESHOLD_BYTES,
        "the tracer must exercise blob-backed content, not only inline storage"
    );
    assert!(String::from_utf8_lossy(&replayed).contains("Hello from fixture"));

    fs::remove_file(&source_capture).expect("delete source capture");
    fs::remove_dir_all(&fixture).expect("delete fixture root");
    let replayed_after_delete = library
        .replay_capture(capture_id)
        .expect("replay after source deletion");
    assert_eq!(replayed, replayed_after_delete);

    let health = library.health().expect("health");
    assert!(health.ok, "health issues: {:?}", health.issues);

    drop(library);
    let reopened = Library::open(&home).expect("reopen library");
    assert!(reopened
        .session_slice("fixture", "fixture-session-hello", 20, 20)
        .expect("session after reopen")
        .is_some());
    assert_eq!(
        reopened
            .replay_capture(capture_id)
            .expect("replay after reopen"),
        replayed
    );
    let health_reopen = reopened.health().expect("health after reopen");
    assert!(
        health_reopen.ok,
        "reopen health issues: {:?}",
        health_reopen.issues
    );
}

#[test]
fn rejects_missing_parent_traversal_before_snapshot() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");

    let manifest = r#"{
  "version": 1,
  "captures": [
    {
      "id": "missing-escape",
      "kind": "file",
      "relative_path": "../missing-outside.jsonl",
      "external_session_id": "fixture-missing-escape"
    }
  ]
}"#;
    fs::write(fixture.join("distill.fixture.json"), manifest).expect("manifest");

    let mut library = Library::open(&home).expect("open");
    let err = library
        .ingest_fixture(&fixture)
        .expect_err("parent traversal must fail before snapshot");
    assert!(
        matches!(err, LibraryError::PathOutsideConfiguredRoot { .. }),
        "expected PathOutsideConfiguredRoot, got {err:?}"
    );
}

#[test]
fn omitted_fixture_identity_uses_deterministic_synthetic_provenance() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(fixture.join("captures")).expect("captures");
    fs::write(
        fixture.join("captures/synthetic.jsonl"),
        "{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"synthetic\"}\n",
    )
    .expect("capture");
    let manifest = r#"{
  "version": 1,
  "captures": [
    {
      "id": "synthetic",
      "kind": "file",
      "relative_path": "captures/synthetic.jsonl"
    }
  ]
}"#;
    fs::write(fixture.join("distill.fixture.json"), manifest).expect("manifest");

    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("ingest");

    let detail = library
        .session_slice("fixture", "synthetic-63db78c48a553484", 20, 20)
        .expect("query")
        .expect("synthetic Session Identity");
    assert!(detail.metadata_json.contains("\"synthetic_identity\":true"));
    assert!(detail
        .metadata_json
        .contains("fixture://synthetic/captures/synthetic.jsonl"));
}

#[test]
fn rejects_path_outside_configured_fixture_root() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    let outside = temp.path().join("outside.txt");
    fs::create_dir_all(&fixture).expect("fixture");
    fs::write(&outside, "secret\n").expect("outside file");

    // Symlink escape: fixture thinks the capture is inside, but it resolves outside.
    let captures = fixture.join("captures");
    fs::create_dir_all(&captures).expect("captures");
    std::os::unix::fs::symlink(&outside, captures.join("escaped.jsonl")).expect("symlink");

    let manifest = r#"{
  "version": 1,
  "captures": [
    {
      "id": "escaped",
      "kind": "file",
      "relative_path": "captures/escaped.jsonl",
      "external_session_id": "fixture-escaped"
    }
  ]
}"#;
    fs::write(fixture.join("distill.fixture.json"), manifest).expect("manifest");

    let mut library = Library::open(&home).expect("open");
    let err = library
        .ingest_fixture(&fixture)
        .expect_err("path escape must fail");
    assert!(
        matches!(err, LibraryError::PathOutsideConfiguredRoot { .. }),
        "expected PathOutsideConfiguredRoot, got {err:?}"
    );
}

#[test]
fn rejects_capture_exceeding_size_limit() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(fixture.join("captures")).expect("captures");

    let oversized = "x".repeat((INLINE_CONTENT_THRESHOLD_BYTES as usize) + 8);
    // Keep under default 64 MiB but over a tiny custom limit.
    fs::write(
        fixture.join("captures/big.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({
                "record_type": "message",
                "role": "user",
                "text": oversized
            })
        ),
    )
    .expect("big capture");

    let manifest = r#"{
  "version": 1,
  "captures": [
    {
      "id": "big",
      "kind": "file",
      "relative_path": "captures/big.jsonl",
      "external_session_id": "fixture-big"
    }
  ]
}"#;
    fs::write(fixture.join("distill.fixture.json"), manifest).expect("manifest");

    let limit = 32_u64;
    let mut library = Library::open_with_limits(&home, limit).expect("open");
    let err = library
        .ingest_fixture(&fixture)
        .expect_err("oversized capture must fail");
    match err {
        LibraryError::CaptureTooLarge {
            byte_size,
            limit: got_limit,
        } => {
            assert!(byte_size > got_limit);
            assert_eq!(got_limit, limit);
        }
        other => panic!("expected CaptureTooLarge, got {other:?}"),
    }
}
