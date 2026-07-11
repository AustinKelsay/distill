//! Library attempt-retry contracts for issue #20.
//!
//! Public-seam TDD over real temporary Distill homes, SQLite, CAS, and Fixture paths.

use std::fs;
use std::path::{Path, PathBuf};

use distill_library::{Library, LibraryError};
use tempfile::TempDir;

/**
 * Write a Fixture root with one file-backed Capture Candidate and optional body.
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
      "title": "Attempt Retry Fixture"
    }}
  ]
}}"#
    );
    fs::write(root.join("distill.fixture.json"), manifest).expect("write manifest");
    capture_path
}

/**
 * Baseline multi-message Fixture body used by several scenarios.
 */
fn rich_body() -> String {
    concat!(
        r#"{"record_type":"session_meta","title":"Attempt Retry Fixture","summary":"baseline"}"#,
        "\n",
        r#"{"record_type":"message","role":"user","text":"first user"}"#,
        "\n",
        r#"{"record_type":"message","role":"assistant","text":"first assistant"}"#,
        "\n",
        r#"{"record_type":"message","role":"user","text":"second user"}"#,
        "\n",
        r#"{"record_type":"tool_use","text":"echo tool"}"#,
        "\n",
    )
    .to_string()
}

#[test]
fn exact_duplicate_ingest_is_inert() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-dup",
        "captures/hello.jsonl",
        &rich_body(),
    );

    let mut library = Library::open(&home).expect("open");
    let first = library.ingest_fixture(&fixture).expect("first ingest");
    assert_eq!(first.accepted_captures, 1);
    assert_eq!(first.successful_attempts, 1);
    assert_eq!(first.skipped_duplicates, 0);

    let before = library
        .session_slice("fixture", "fixture-session-dup", 20, 20)
        .expect("session")
        .expect("present");
    let activity_before = library.recent_activity(50).expect("activity");
    let projection_events_before = activity_before
        .iter()
        .filter(|event| event.event_type == "projection_replaced")
        .count();
    let capture_events_before = activity_before
        .iter()
        .filter(|event| event.event_type == "capture_recorded")
        .count();
    let attempts_before = library
        .capture_attempts(first.capture_ids[0])
        .expect("attempts");
    assert_eq!(attempts_before.len(), 1);

    let second = library.ingest_fixture(&fixture).expect("duplicate ingest");
    assert_eq!(second.accepted_captures, 0);
    assert_eq!(second.skipped_duplicates, 1);
    assert_eq!(second.successful_attempts, 0);
    assert_eq!(second.failed_attempts, 0);

    let after = library
        .session_slice("fixture", "fixture-session-dup", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(
        after.summary.accepted_capture_count,
        before.summary.accepted_capture_count
    );
    assert_eq!(
        after.summary.normalization_attempt_count,
        before.summary.normalization_attempt_count
    );
    assert_eq!(
        after.summary.successful_projection_generation,
        before.summary.successful_projection_generation
    );
    assert_eq!(after.messages.len(), before.messages.len());
    assert_eq!(after.messages[0].text, before.messages[0].text);

    let attempts_after = library
        .capture_attempts(first.capture_ids[0])
        .expect("attempts after");
    assert_eq!(attempts_after.len(), 1);

    let activity_after = library.recent_activity(50).expect("activity after");
    assert_eq!(
        activity_after
            .iter()
            .filter(|event| event.event_type == "projection_replaced")
            .count(),
        projection_events_before
    );
    assert_eq!(
        activity_after
            .iter()
            .filter(|event| event.event_type == "capture_recorded")
            .count(),
        capture_events_before
    );

    let hits = library.search("first user", 20).expect("search");
    assert!(hits.iter().any(|hit| hit.text.contains("first user")));
}

#[test]
fn changed_bytes_create_new_capture_and_replace_projection() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    let capture_path = write_fixture(
        &fixture,
        "fixture-session-change",
        "captures/hello.jsonl",
        &rich_body(),
    );

    let mut library = Library::open(&home).expect("open");
    let first = library.ingest_fixture(&fixture).expect("first");
    assert_eq!(first.accepted_captures, 1);
    let first_capture = first.capture_ids[0];

    let changed = concat!(
        r#"{"record_type":"session_meta","title":"Changed Fixture","summary":"changed"}"#,
        "\n",
        r#"{"record_type":"message","role":"user","text":"only changed user"}"#,
        "\n",
        r#"{"record_type":"message","role":"assistant","text":"only changed assistant"}"#,
        "\n",
    );
    fs::write(&capture_path, changed).expect("rewrite capture");

    let second = library.ingest_fixture(&fixture).expect("changed ingest");
    assert_eq!(second.accepted_captures, 1);
    assert_eq!(second.successful_attempts, 1);
    assert_eq!(second.skipped_duplicates, 0);
    assert_ne!(second.capture_ids[0], first_capture);

    let detail = library
        .session_slice("fixture", "fixture-session-change", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(detail.summary.accepted_capture_count, 2);
    assert_eq!(detail.summary.normalization_attempt_count, 2);
    assert_eq!(detail.summary.successful_projection_generation, 2);
    assert_eq!(detail.messages.len(), 2);
    assert_eq!(detail.messages[0].text, "only changed user");
    assert!(!detail
        .messages
        .iter()
        .any(|message| message.text.contains("first user")));

    let stale = library.search("first user", 20).expect("stale search");
    assert!(
        stale.is_empty(),
        "superseded projection text must leave FTS"
    );
    let current = library.search("only changed user", 20).expect("search");
    assert!(current
        .iter()
        .any(|hit| hit.text.contains("only changed user")));
}

#[test]
fn parse_failure_records_safe_attempt_and_preserves_last_good() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    let capture_path = write_fixture(
        &fixture,
        "fixture-session-parse-fail",
        "captures/hello.jsonl",
        &rich_body(),
    );

    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("good ingest");
    let before = library
        .session_slice("fixture", "fixture-session-parse-fail", 20, 20)
        .expect("session")
        .expect("present");
    let generation_before = before.summary.successful_projection_generation;
    let messages_before = before.messages.clone();

    fs::write(
        &capture_path,
        concat!(
            r#"{"record_type":"session_meta","title":"Broken"}"#,
            "\n",
            "{not-json\n",
        ),
    )
    .expect("broken capture");

    let failed = library.ingest_fixture(&fixture).expect("failed ingest");
    assert_eq!(failed.accepted_captures, 1);
    assert_eq!(failed.failed_attempts, 1);
    assert_eq!(failed.successful_attempts, 0);
    let failed_capture = failed.capture_ids[0];

    let after = library
        .session_slice("fixture", "fixture-session-parse-fail", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(
        after.summary.successful_projection_generation,
        generation_before
    );
    assert_eq!(after.messages, messages_before);
    assert_eq!(after.summary.accepted_capture_count, 2);
    assert_eq!(after.summary.normalization_attempt_count, 2);

    let attempts = library
        .capture_attempts(failed_capture)
        .expect("failed attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].outcome, "failed");
    assert_eq!(attempts[0].error_class.as_deref(), Some("parse_failed"));
    assert_eq!(
        attempts[0].error_message.as_deref(),
        Some("parser rejected Capture content")
    );
    assert!(!attempts[0]
        .error_message
        .as_deref()
        .is_some_and(|message| message.contains("not-json")));
    assert_eq!(attempts[0].fact_count, 0);

    let hits = library.search("first user", 20).expect("search");
    assert!(hits.iter().any(|hit| hit.text.contains("first user")));
}

#[test]
fn projection_failure_records_safe_attempt_and_preserves_last_good() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    let capture_path = write_fixture(
        &fixture,
        "fixture-session-proj-fail",
        "captures/hello.jsonl",
        &rich_body(),
    );

    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("good ingest");
    let before = library
        .session_slice("fixture", "fixture-session-proj-fail", 20, 20)
        .expect("session")
        .expect("present");

    fs::write(
        &capture_path,
        concat!(
            r#"{"record_type":"session_meta","title":"Projection Bomb"}"#,
            "\n",
            r#"{"record_type":"force_projection_fail","text":"boom"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"should not publish"}"#,
            "\n",
        ),
    )
    .expect("bomb capture");

    let failed = library
        .ingest_fixture(&fixture)
        .expect("projection fail ingest");
    assert_eq!(failed.accepted_captures, 1);
    assert_eq!(failed.failed_attempts, 1);
    assert_eq!(failed.successful_attempts, 0);

    let after = library
        .session_slice("fixture", "fixture-session-proj-fail", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(
        after.summary.successful_projection_generation,
        before.summary.successful_projection_generation
    );
    assert_eq!(after.messages[0].text, before.messages[0].text);
    assert!(!after
        .messages
        .iter()
        .any(|message| message.text.contains("should not publish")));

    let attempts = library
        .capture_attempts(failed.capture_ids[0])
        .expect("attempts");
    assert_eq!(attempts[0].outcome, "failed");
    assert_eq!(
        attempts[0].error_class.as_deref(),
        Some("projection_failed")
    );
    assert_eq!(
        attempts[0].error_message.as_deref(),
        Some("projection constraints rejected the Attempt output")
    );
    assert_eq!(attempts[0].fact_count, 0);

    let hits = library.search("should not publish", 20).expect("search");
    assert!(hits.is_empty());
}

#[test]
fn newer_registered_fixture_parser_renormalizes_same_capture() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-retry",
        "captures/hello.jsonl",
        concat!(
            r#"{"record_type":"session_meta","title":"Retry Fixture"}"#,
            "\n",
            r#"{"record_type":"require_parser_min","version":"2.0.0"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"needs newer parser"}"#,
            "\n",
        ),
    );

    let mut library = Library::open(&home).expect("open");
    let first = library.ingest_fixture(&fixture).expect("v1 ingest");
    assert_eq!(first.accepted_captures, 1);
    assert_eq!(first.failed_attempts, 1);
    assert_eq!(first.successful_attempts, 0);
    let capture_id = first.capture_ids[0];
    assert!(library
        .session_slice("fixture", "fixture-session-retry", 20, 20)
        .expect("query")
        .is_none());

    let v1_attempts = library.capture_attempts(capture_id).expect("v1 attempts");
    assert_eq!(v1_attempts.len(), 1);
    assert_eq!(v1_attempts[0].outcome, "failed");
    assert_eq!(v1_attempts[0].parser_version, "1.0.0");
    assert_eq!(v1_attempts[0].fact_count, 0);

    library
        .set_registered_fixture_parser_version("2.0.0")
        .expect("register newer parser");
    let retry = library
        .renormalize_capture(capture_id)
        .expect("renormalize");
    assert_eq!(retry.capture_id, capture_id);
    assert_eq!(retry.outcome, "succeeded");
    assert_eq!(retry.parser_id, "fixture");
    assert_eq!(retry.parser_version, "2.0.0");

    let detail = library
        .session_slice("fixture", "fixture-session-retry", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(detail.summary.accepted_capture_count, 1);
    assert_eq!(detail.summary.normalization_attempt_count, 2);
    assert_eq!(detail.summary.successful_projection_generation, 1);
    assert_eq!(detail.messages[0].text, "needs newer parser");

    let attempts = library.capture_attempts(capture_id).expect("attempts");
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].outcome, "failed");
    assert_eq!(attempts[0].fact_count, 0);
    assert_eq!(attempts[1].outcome, "succeeded");
    assert!(attempts[1].fact_count >= 1);
    assert_eq!(attempts[0].id, v1_attempts[0].id);

    // Re-ingest exact same bytes remains inert even after successful retry.
    let duplicate = library
        .ingest_fixture(&fixture)
        .expect("duplicate after retry");
    assert_eq!(duplicate.accepted_captures, 0);
    assert_eq!(duplicate.skipped_duplicates, 1);
    assert_eq!(
        library
            .capture_attempts(capture_id)
            .expect("attempts stable")
            .len(),
        2
    );
}

#[test]
fn newer_parser_appends_facts_without_mutating_prior_success() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-fact-history",
        "captures/hello.jsonl",
        &rich_body(),
    );

    let mut library = Library::open(&home).expect("open");
    let first = library.ingest_fixture(&fixture).expect("v1 ingest");
    let capture_id = first.capture_ids[0];
    let before = library.capture_attempts(capture_id).expect("v1 attempts");
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].outcome, "succeeded");
    assert!(before[0].fact_count > 0);
    let original_attempt = before[0].clone();

    library
        .set_registered_fixture_parser_version("2.0.0")
        .expect("register v2");
    let retry = library
        .renormalize_capture(capture_id)
        .expect("renormalize");
    assert_eq!(retry.capture_id, capture_id);
    assert_eq!(retry.outcome, "succeeded");

    let after = library.capture_attempts(capture_id).expect("v2 attempts");
    assert_eq!(after.len(), 2);
    assert_eq!(after[0], original_attempt);
    assert_eq!(after[1].parser_version, "2.0.0");
    assert_eq!(after[1].fact_count, original_attempt.fact_count);

    let detail = library
        .session_slice("fixture", "fixture-session-fact-history", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(detail.summary.accepted_capture_count, 1);
    assert_eq!(detail.summary.normalization_attempt_count, 2);
    assert_eq!(detail.summary.successful_projection_generation, 2);
}

#[test]
fn successful_shorter_and_empty_projections_fully_replace() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    let capture_path = write_fixture(
        &fixture,
        "fixture-session-replace",
        "captures/hello.jsonl",
        &rich_body(),
    );

    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("rich ingest");
    let rich = library
        .session_slice("fixture", "fixture-session-replace", 20, 20)
        .expect("session")
        .expect("present");
    assert!(rich.messages.len() >= 3);
    assert!(!rich.artifacts.is_empty());

    fs::write(
        &capture_path,
        concat!(
            r#"{"record_type":"session_meta","title":"Shorter"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"only one"}"#,
            "\n",
        ),
    )
    .expect("shorter");
    library.ingest_fixture(&fixture).expect("shorter ingest");
    let shorter = library
        .session_slice("fixture", "fixture-session-replace", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(shorter.messages.len(), 1);
    assert_eq!(shorter.messages[0].text, "only one");
    assert!(shorter.artifacts.is_empty());
    assert_eq!(shorter.summary.successful_projection_generation, 2);
    assert!(library.search("first user", 20).expect("search").is_empty());

    fs::write(
        &capture_path,
        concat!(
            r#"{"record_type":"session_meta","title":"Empty Projection","summary":"cleared"}"#,
            "\n",
        ),
    )
    .expect("empty");
    library.ingest_fixture(&fixture).expect("empty ingest");
    let empty = library
        .session_slice("fixture", "fixture-session-replace", 20, 20)
        .expect("session")
        .expect("present");
    assert!(empty.messages.is_empty());
    assert!(empty.artifacts.is_empty());
    assert_eq!(empty.summary.title.as_deref(), Some("Empty Projection"));
    assert_eq!(empty.summary.successful_projection_generation, 3);
    assert_eq!(empty.summary.accepted_capture_count, 3);
    assert!(library.search("only one", 20).expect("search").is_empty());
}

#[test]
fn renormalize_rejects_unknown_capture_and_keeps_parser_id_internal() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let mut library = Library::open(&home).expect("open");

    let err = library
        .renormalize_capture(999_999)
        .expect_err("missing capture");
    assert!(matches!(err, LibraryError::NotFound(_)));

    // Registered Fixture parser version updates keep id fixed to `fixture`.
    library
        .set_registered_fixture_parser_version("2.1.0")
        .expect("set version");
    let err = library
        .set_registered_fixture_parser_version("2.1.0")
        .expect_err("same version");
    assert!(matches!(err, LibraryError::InvalidArgument(_)));
    let err = library
        .set_registered_fixture_parser_version("2.0.0")
        .expect_err("older version");
    assert!(matches!(err, LibraryError::InvalidArgument(_)));
    let err = library
        .set_registered_fixture_parser_version("")
        .expect_err("empty version");
    assert!(matches!(err, LibraryError::InvalidArgument(_)));
    let err = library
        .set_registered_fixture_parser_version("not-a-version")
        .expect_err("malformed version");
    assert!(matches!(err, LibraryError::InvalidArgument(_)));
}
