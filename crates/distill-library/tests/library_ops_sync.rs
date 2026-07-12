//! Library Sync Run, Source preference, detection, and operations contracts for issue #22.
//!
//! Public-seam TDD over real temporary Distill homes. Overlap tests use two Library
//! instances against the same home. Stale leases are proven by aging durable
//! `lease_expires_at` / `heartbeat_at` columns in the real temp SQLite DB, then
//! reopening with the normal public `Library::open` seam (no injectable clock).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
#[cfg(feature = "test-leases")]
use distill_library::test_support::{
    enforce_output_bounds_for_test, run_bounded_command, ProviderProcessLimits,
};
use distill_library::{
    Library, LibraryError, SourceDetectRequest, SyncProgress, SyncRequest, SYNC_LEASE_STALE_AFTER,
};
use rusqlite::Connection;
use tempfile::TempDir;

#[cfg(feature = "test-leases")]
static LEASE_TIMING_TEST_LOCK: Mutex<()> = Mutex::new(());

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
      "title": "Sync Fixture"
    }}
  ]
}}"#
    );
    fs::write(root.join("distill.fixture.json"), manifest).expect("write manifest");
    capture_path
}

/**
 * Write a Fixture root with two Capture Candidates for cancellation checkpoints.
 */
fn write_two_candidate_fixture(root: &Path) {
    fs::create_dir_all(root.join("sessions")).expect("sessions");
    fs::write(
        root.join("sessions/one.jsonl"),
        concat!(
            r#"{"record_type":"session_meta","title":"One","summary":"first"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"one user"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"one assistant"}"#,
            "\n",
        ),
    )
    .expect("one");
    fs::write(
        root.join("sessions/two.jsonl"),
        concat!(
            r#"{"record_type":"session_meta","title":"Two","summary":"second"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"two user"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"two assistant"}"#,
            "\n",
        ),
    )
    .expect("two");
    fs::write(
        root.join("distill.fixture.json"),
        r#"{
  "version": 1,
  "captures": [
    {
      "id": "one",
      "kind": "file",
      "relative_path": "sessions/one.jsonl",
      "external_session_id": "fixture-session-one",
      "title": "One"
    },
    {
      "id": "two",
      "kind": "file",
      "relative_path": "sessions/two.jsonl",
      "external_session_id": "fixture-session-two",
      "title": "Two"
    }
  ]
}"#,
    )
    .expect("manifest");
}

/**
 * Write one valid candidate plus one invalid JSONL parse candidate.
 */
fn write_partial_parse_fixture(root: &Path) {
    fs::create_dir_all(root.join("sessions")).expect("sessions");
    fs::write(root.join("sessions/good.jsonl"), rich_body()).expect("good");
    fs::write(root.join("sessions/bad.jsonl"), "{not-jsonl\n").expect("bad");
    fs::write(
        root.join("distill.fixture.json"),
        r#"{
  "version": 1,
  "captures": [
    {
      "id": "good",
      "kind": "file",
      "relative_path": "sessions/good.jsonl",
      "external_session_id": "fixture-session-good",
      "title": "Good"
    },
    {
      "id": "bad",
      "kind": "file",
      "relative_path": "sessions/bad.jsonl",
      "external_session_id": "fixture-session-bad",
      "title": "Bad"
    }
  ]
}"#,
    )
    .expect("manifest");
}

fn rich_body() -> String {
    concat!(
        r#"{"record_type":"session_meta","title":"Sync Fixture","summary":"baseline"}"#,
        "\n",
        r#"{"record_type":"message","role":"user","text":"sync user"}"#,
        "\n",
        r#"{"record_type":"message","role":"assistant","text":"sync assistant"}"#,
        "\n",
    )
    .to_string()
}

/**
 * Age every active Sync Run lease in the real Distill SQLite database.
 *
 * Used instead of an injectable Library clock so production never exposes clock
 * authority that could falsely fail a live run.
 */
fn age_active_leases(home: &Path) {
    let past = (Utc::now() - ChronoDuration::seconds(120)).to_rfc3339();
    let conn = Connection::open(home.join("distill.db")).expect("open db");
    conn.execute(
        "UPDATE sync_runs
         SET heartbeat_at = ?1, lease_expires_at = ?1
         WHERE status IN ('queued', 'running')",
        [&past],
    )
    .expect("age leases");
}

/**
 * Count Capture rows for assertions about post-stale acceptance.
 */
fn capture_count(home: &Path) -> i64 {
    let conn = Connection::open(home.join("distill.db")).expect("open db");
    conn.query_row("SELECT COUNT(*) FROM captures", [], |row| row.get(0))
        .expect("count")
}

/**
 * Count Activity Events of one type.
 */
fn activity_count(home: &Path, event_type: &str) -> i64 {
    let conn = Connection::open(home.join("distill.db")).expect("open db");
    conn.query_row(
        "SELECT COUNT(*) FROM activity_events WHERE event_type = ?1",
        [event_type],
        |row| row.get(0),
    )
    .expect("count")
}

/** Count durable Sync Run rows. */
fn sync_run_count(home: &Path) -> i64 {
    let conn = Connection::open(home.join("distill.db")).expect("open db");
    conn.query_row("SELECT COUNT(*) FROM sync_runs", [], |row| row.get(0))
        .expect("count")
}

/**
 * OSR-001: migration 0002 applies; preferences persist enabled + canonical root.
 */
#[test]
fn source_preferences_persist_enabled_and_canonical_root() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-prefs",
        "sessions/a.jsonl",
        &rich_body(),
    );

    let mut library = Library::open(&home).expect("open");
    let pref = library
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("set pref");
    assert!(pref.enabled);
    let canonical = fs::canonicalize(&fixture).expect("canon");
    assert_eq!(
        pref.configured_root.as_deref(),
        Some(canonical.to_str().unwrap())
    );

    let listed = library.list_sources().expect("list");
    let fixture_pref = listed
        .iter()
        .find(|p| p.kind == "fixture")
        .expect("fixture");
    assert!(fixture_pref.enabled);
    assert_eq!(
        fixture_pref.configured_root.as_deref(),
        Some(canonical.to_str().unwrap())
    );

    drop(library);
    let reopened = Library::open(&home).expect("reopen");
    let reopened_pref = reopened
        .list_sources()
        .expect("reopened prefs")
        .into_iter()
        .find(|pref| pref.kind == "fixture")
        .expect("fixture after reopen");
    assert!(reopened_pref.enabled);
    assert_eq!(
        reopened_pref.configured_root.as_deref(),
        Some(canonical.to_str().unwrap())
    );
}

/**
 * OSR-002: detection returns independent typed results; one failure never aborts siblings.
 * Diagnostics remain generic — no path/payload fragments even when roots contain spaces.
 */
#[test]
fn detection_isolates_failures_across_independent_fixture_requests() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let good = temp.path().join("good root");
    let bad = temp.path().join("bad root with spaces and secret token");
    fs::create_dir_all(&good).expect("good");
    fs::create_dir_all(&bad).expect("bad");
    write_fixture(
        &good,
        "fixture-session-good",
        "sessions/g.jsonl",
        &rich_body(),
    );
    // bad has no distill.fixture.json; path itself carries sensitive fragments.

    let library = Library::open(&home).expect("open");
    let results = library
        .detect_sources(&[
            SourceDetectRequest {
                kind: "fixture".into(),
                configured_root: Some(bad.display().to_string()),
            },
            SourceDetectRequest {
                kind: "fixture".into(),
                configured_root: Some(good.display().to_string()),
            },
            SourceDetectRequest {
                kind: "claude_code".into(),
                configured_root: None,
            },
            SourceDetectRequest {
                kind: "codex".into(),
                configured_root: Some(
                    temp.path()
                        .join("missing root with secret token")
                        .display()
                        .to_string(),
                ),
            },
        ])
        .expect("detect");

    assert_eq!(results.len(), 4);
    assert_eq!(results[0].status, "unhealthy");
    let bad_message = results[0].error_message.as_deref().unwrap_or("");
    assert!(
        !bad_message.contains("secret"),
        "diagnostics must not leak path fragments: {bad_message}"
    );
    assert!(
        !bad_message.contains("spaces"),
        "diagnostics must not leak path fragments: {bad_message}"
    );
    assert!(
        !bad_message.contains('/'),
        "diagnostics must not leak path separators: {bad_message}"
    );
    assert_eq!(results[1].status, "ok");
    assert!(results[1].executable.is_none());
    assert_eq!(results[2].status, "unavailable");
    assert_eq!(
        results[2].error_class.as_deref(),
        Some("adapter_not_registered")
    );
    assert_eq!(results[3].status, "unhealthy");
    assert_eq!(
        results[3].error_class.as_deref(),
        Some("invalid_configured_root")
    );
    assert!(results[3].effective_data_root.is_none());
    assert!(!results[3]
        .error_message
        .as_deref()
        .unwrap_or_default()
        .contains("secret"));
}

/**
 * OSR-003: sync_already_running from a second Library has no durable side effects.
 */
#[test]
fn overlapping_sync_returns_sync_already_running_without_side_effects() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_two_candidate_fixture(&fixture);

    let mut owner = Library::open(&home).expect("owner");
    owner
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("pref");

    let gate = Arc::new(Mutex::new(false));
    let gate_for_thread = Arc::clone(&gate);
    let home_for_thread = home.clone();
    let started = Arc::new(Mutex::new(false));
    let started_flag = Arc::clone(&started);

    let worker = thread::spawn(move || {
        let mut library = Library::open(&home_for_thread).expect("worker open");
        library
            .set_source_preference("fixture", true, Some(fixture.as_path()))
            .ok();
        let _ = library.start_sync(SyncRequest::default(), |progress| {
            if matches!(progress, SyncProgress::CandidateStarted { .. }) {
                *started_flag.lock().unwrap() = true;
                while !*gate_for_thread.lock().unwrap() {
                    thread::sleep(Duration::from_millis(5));
                }
            }
        });
    });

    while !*started.lock().unwrap() {
        thread::sleep(Duration::from_millis(5));
    }

    let activity_before = owner.recent_activity(200).expect("activity before").len();
    let runs_before = sync_run_count(&home);
    let err = owner
        .start_sync(SyncRequest::default(), |_| {})
        .expect_err("overlap must fail");
    assert!(matches!(err, LibraryError::SyncAlreadyRunning));
    assert_eq!(err.code(), "sync_already_running");
    let activity_after = owner.recent_activity(200).expect("activity after").len();
    let runs_after = sync_run_count(&home);
    assert_eq!(
        activity_before, activity_after,
        "overlap must create no Activity side effects"
    );
    assert_eq!(
        runs_before, runs_after,
        "overlap must create no Sync Run row"
    );

    *gate.lock().unwrap() = true;
    worker.join().expect("join");
}

/**
 * OSR-004: cancel from a second Library honors the next candidate checkpoint.
 */
#[test]
fn cancel_from_second_library_honors_candidate_checkpoint() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_two_candidate_fixture(&fixture);

    let mut owner = Library::open(&home).expect("owner");
    owner
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("pref");

    let home_for_cancel = home.clone();
    let cancel_armed = Arc::new(Mutex::new(false));
    let cancel_armed_flag = Arc::clone(&cancel_armed);

    let result = owner
        .start_sync(SyncRequest::default(), |progress| {
            if let SyncProgress::CandidateStarted {
                sync_run_id,
                candidate_id,
                ..
            } = &progress
            {
                if candidate_id.contains("one") && !*cancel_armed_flag.lock().unwrap() {
                    *cancel_armed_flag.lock().unwrap() = true;
                    let mut other = Library::open(&home_for_cancel).expect("cancel library");
                    other
                        .request_sync_cancel(*sync_run_id)
                        .expect("request cancel");
                }
            }
        })
        .expect("sync");

    assert_eq!(result.run.status, "cancelled");
    assert_eq!(result.run.accepted_captures, 1);
    let activity = owner.recent_activity(50).expect("activity");
    assert!(activity.iter().any(|e| e.event_type == "sync_failed"));
    assert!(activity.iter().any(|e| e.event_type == "sync_queued"));
    assert!(activity.iter().any(|e| e.event_type == "sync_started"));
    let conn = Connection::open(home.join("distill.db")).expect("db");
    let payload: String = conn
        .query_row(
            "SELECT payload_json FROM activity_events
             WHERE event_type = 'sync_failed' ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("cancel payload");
    assert!(payload.contains("\"reason\":\"cancelled\""));
}

/**
 * OSR-005: completed Fixture sync emits sync_completed and projects sessions.
 */
#[test]
fn successful_sync_completes_with_activity_and_projection() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-sync",
        "sessions/a.jsonl",
        &rich_body(),
    );

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("pref");
    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.status, "completed");
    assert!(result.run.accepted_captures >= 1);
    assert!(!result.session_identities.is_empty());

    let health = library.health().expect("health");
    assert_eq!(health.operations_status, "ok");
    let activity = library.recent_activity(50).expect("activity");
    assert!(activity.iter().any(|e| e.event_type == "sync_completed"));
}

/**
 * OSR-006 / LHR-007: live leases report active; stale leases fail idempotently on reopen.
 *
 * Strengthened: after stale repair, the old worker is unblocked/joined and must
 * leave the row failed, emit only one terminal `sync_failed`, and accept no
 * post-stale Capture.
 */
#[test]
fn operations_status_active_and_stale_lease_reopen() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_two_candidate_fixture(&fixture);

    let mut owner = Library::open(&home).expect("open");
    owner
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("pref");

    let hold = Arc::new(Mutex::new(false));
    let hold_t = Arc::clone(&hold);
    let home_t = home.clone();
    let started = Arc::new(Mutex::new(false));
    let started_t = Arc::clone(&started);
    let worker_result = Arc::new(Mutex::new(None));
    let worker_result_t = Arc::clone(&worker_result);
    let worker = thread::spawn(move || {
        let mut library = Library::open(&home_t).expect("worker");
        let outcome = library.start_sync(SyncRequest::default(), |progress| {
            if matches!(progress, SyncProgress::CandidateStarted { .. }) {
                *started_t.lock().unwrap() = true;
                while !*hold_t.lock().unwrap() {
                    thread::sleep(Duration::from_millis(5));
                }
            }
        });
        *worker_result_t.lock().unwrap() = Some(outcome);
    });

    while !*started.lock().unwrap() {
        thread::sleep(Duration::from_millis(5));
    }

    let observer = Library::open(&home).expect("observer");
    let health_active = observer.health().expect("health active");
    assert_eq!(health_active.operations_status, "active");
    assert!(health_active
        .issues
        .iter()
        .all(|issue| issue.code != "stale_sync_operation"));

    let captures_before_stale = capture_count(&home);
    age_active_leases(&home);

    let reopened = Library::open(&home).expect("reopen");
    let health_after = reopened.health().expect("health after");
    assert_eq!(health_after.operations_status, "ok");
    let failed = activity_count(&home, "sync_failed");
    assert_eq!(
        failed, 1,
        "stale reopen must append exactly one sync_failed"
    );

    let again = Library::open(&home).expect("again");
    assert_eq!(
        activity_count(&home, "sync_failed"),
        failed,
        "idempotent reopen must not duplicate sync_failed"
    );
    let _ = again;

    *hold.lock().unwrap() = true;
    worker.join().expect("join");
    let worker_outcome = worker_result
        .lock()
        .unwrap()
        .take()
        .expect("worker outcome");
    assert!(
        matches!(worker_outcome, Err(LibraryError::SyncLeaseLost)),
        "stale-failed worker must observe sync_lease_lost, got {worker_outcome:?}"
    );

    let final_status: String = {
        let conn = Connection::open(home.join("distill.db")).expect("db");
        conn.query_row(
            "SELECT status FROM sync_runs ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("status")
    };
    assert_eq!(final_status, "failed");
    assert_eq!(activity_count(&home, "sync_failed"), 1);
    assert_eq!(
        capture_count(&home),
        captures_before_stale,
        "no post-stale Capture may be accepted"
    );
    // Production stale threshold remains the documented constant.
    assert_eq!(SYNC_LEASE_STALE_AFTER, Duration::from_secs(60));
}

/**
 * Health classifies an active run with malformed lease data instead of failing
 * the entire health report or silently treating the run as healthy.
 */
#[test]
fn health_reports_invalid_active_lease_timestamp() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let library = Library::open(&home).expect("open");
    let conn = Connection::open(home.join("distill.db")).expect("db");
    conn.execute(
        "INSERT INTO sync_runs (
            status, requested_at, cancel_requested, owner_id, heartbeat_at,
            lease_expires_at, metrics_json, warning_details_json
         ) VALUES ('running', '2026-01-01T00:00:00Z', 0, 'owner-test',
                   '2026-01-01T00:00:00Z', 'not-a-timestamp', '{}', '[]')",
        [],
    )
    .expect("insert malformed lease");

    drop(library);
    let reopened = Library::open(&home).expect("reopen malformed lease");
    let health = reopened.health().expect("health remains typed");
    assert_eq!(health.operations_status, "failed");
    assert!(health.issues.iter().any(|issue| {
        issue.code == "invalid_lease_timestamp"
            && issue.severity == "repairable"
            && issue.category == "sync"
    }));
}

/**
 * OSR-007: configured-root rejects empty and traversal escapes; symlink roots canonicalize.
 */
#[test]
fn configured_root_rejects_empty_and_traversal() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let mut library = Library::open(&home).expect("open");

    let err = library
        .set_source_preference("fixture", true, Some(Path::new("")))
        .expect_err("empty");
    assert_eq!(err.code(), "invalid_configured_root");

    let err = library
        .set_source_preference(
            "fixture",
            true,
            Some(temp.path().join("missing/../outside").as_path()),
        )
        .expect_err("missing escape");
    assert_eq!(err.code(), "invalid_configured_root");

    let real = temp.path().join("real-root");
    fs::create_dir_all(&real).expect("real");
    write_fixture(
        &real,
        "fixture-session-link",
        "sessions/a.jsonl",
        &rich_body(),
    );
    let link = temp.path().join("link-root");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        let pref = library
            .set_source_preference("fixture", true, Some(link.as_path()))
            .expect("symlink root");
        let canonical = fs::canonicalize(&real).expect("canon");
        assert_eq!(
            pref.configured_root.as_deref(),
            Some(canonical.to_str().unwrap())
        );
    }
}

/**
 * OSR-008: provider subprocess output bounds are enforced without leaking payloads.
 */
#[cfg(feature = "test-leases")]
#[test]
fn provider_process_output_bounds_are_enforced() {
    let limits = ProviderProcessLimits {
        max_duration: Duration::from_secs(1),
        max_stdout_bytes: 16,
        max_stderr_bytes: 16,
    };
    let err = enforce_output_bounds_for_test(&[0_u8; 64], b"ok", limits).expect_err("stdout");
    assert_eq!(err.code(), "provider_process_bound_exceeded");
    let message = err.to_string();
    assert!(!message.contains('\0'));
}

/**
 * Unix-only duration bound using a blocking child helper (no flaky multi-second sleeps).
 */
#[cfg(all(unix, feature = "test-leases"))]
#[test]
fn provider_process_duration_bound_unix() {
    use std::process::Command;

    let limits = ProviderProcessLimits {
        max_duration: Duration::from_millis(50),
        max_stdout_bytes: 1024,
        max_stderr_bytes: 1024,
    };
    let mut command = Command::new("/bin/sleep");
    command.arg("5");
    let err = run_bounded_command(command, limits, None).expect_err("timeout");
    assert_eq!(err.code(), "provider_process_bound_exceeded");
    assert!(err.to_string().contains("duration"));
}

/**
 * OSR-009: unknown or empty/disabled SyncRequest selections reject with zero side effects.
 */
#[test]
fn sync_request_rejects_unknown_or_disabled_selection_without_side_effects() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("fixture", false, None)
        .expect("disable fixture");

    let activity_before = library.recent_activity(200).expect("before").len();

    let err = library
        .start_sync(
            SyncRequest {
                source_kinds: vec!["not-a-source".into()],
            },
            |_| {},
        )
        .expect_err("unknown");
    assert_eq!(err.code(), "invalid_argument");

    let err = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect_err("none enabled");
    assert_eq!(err.code(), "sync_no_enabled_sources");

    let err = library
        .start_sync(
            SyncRequest {
                source_kinds: vec!["fixture".into()],
            },
            |_| {},
        )
        .expect_err("disabled selection");
    assert_eq!(err.code(), "sync_no_enabled_sources");

    assert_eq!(
        library.recent_activity(200).expect("after").len(),
        activity_before
    );
    let conn = Connection::open(home.join("distill.db")).expect("db");
    let runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM sync_runs", [], |row| row.get(0))
        .expect("runs");
    assert_eq!(runs, 0);
}

/**
 * OSR-010: one good Fixture Source plus one enabled Codex Source without a
 * configured root terminates as warning.
 */
#[test]
fn sync_partial_source_success_terminates_as_warning() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-warn-src",
        "sessions/a.jsonl",
        &rich_body(),
    );

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("fixture");
    library
        .set_source_preference("codex", true, None)
        .expect("codex");

    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.status, "warning");
    assert!(!result.run.warning_details.is_empty());
    assert!(result.run.accepted_captures >= 1);
    assert_eq!(result.run.sources.len(), 2);
    assert!(result
        .run
        .sources
        .iter()
        .any(|s| s.source_kind == "fixture" && s.status == "completed"));
    assert!(result
        .run
        .sources
        .iter()
        .any(|s| s.source_kind == "codex" && s.status == "failed"));

    let activity = library.recent_activity(50).expect("activity");
    assert!(activity.iter().any(|e| e.event_type == "sync_completed"));
    assert!(!activity.iter().any(|e| e.event_type == "sync_failed"));
}

/**
 * OSR-011: one valid + one invalid Fixture candidate persists both outcomes as warning.
 */
#[test]
fn sync_partial_candidate_success_terminates_as_warning() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_partial_parse_fixture(&fixture);

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("pref");
    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");

    assert_eq!(result.run.status, "warning");
    assert_eq!(result.run.accepted_captures, 2);
    assert_eq!(result.run.successful_attempts, 1);
    assert_eq!(result.run.failed_attempts, 1);
    assert_eq!(result.run.sources.len(), 1);
    assert_eq!(result.run.sources[0].status, "warning");
    assert!(!result.run.warning_details.is_empty());

    let activity = library.recent_activity(50).expect("activity");
    assert!(activity.iter().any(|e| e.event_type == "sync_completed"));
    assert!(!activity.iter().any(|e| e.event_type == "sync_failed"));
    let conn = Connection::open(home.join("distill.db")).expect("db");
    let payload: String = conn
        .query_row(
            "SELECT payload_json FROM activity_events
             WHERE event_type = 'sync_completed'
             ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("payload");
    assert!(
        payload.contains("warning"),
        "sync_completed payload must record warning status: {payload}"
    );
}

/**
 * OSR-012: bad Fixture manifest + enabled Codex Source without a configured
 * root fails with no progress.
 */
#[test]
fn sync_all_sources_failing_terminates_as_failed() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    fs::write(
        fixture.join("distill.fixture.json"),
        r#"{"version":1,"captures":"broken"}"#,
    )
    .expect("bad manifest");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("fixture");
    library
        .set_source_preference("codex", true, None)
        .expect("codex");

    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.status, "failed");
    assert_eq!(result.run.accepted_captures, 0);
    assert_eq!(result.run.sources.len(), 2);
    assert!(result.run.sources.iter().all(|s| s.status == "failed"));

    let activity = library.recent_activity(50).expect("activity");
    assert!(activity.iter().any(|e| e.event_type == "sync_failed"));
    assert!(!activity.iter().any(|e| e.event_type == "sync_completed"));
}

/**
 * OSR-013: cancel on a terminal run is idempotent; sync_status(None) is typed NotFound.
 */
#[test]
fn cancel_on_terminal_is_idempotent_and_status_none_is_not_found() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let mut library = Library::open(&home).expect("open");

    let err = library.sync_status(None).expect_err("no runs");
    assert!(matches!(err, LibraryError::NotFound(_)));
    assert_eq!(err.code(), "not_found");

    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-session-cancel-terminal",
        "sessions/a.jsonl",
        &rich_body(),
    );
    library
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("pref");
    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.status, "completed");

    library
        .request_sync_cancel(result.run.id)
        .expect("idempotent cancel on terminal");
    let status = library
        .sync_status(Some(result.run.id))
        .expect("status after cancel");
    assert_eq!(status.status, "completed");
    assert!(!status.cancel_requested);
}

/**
 * OSR-014: heartbeat starts before progress delivery and keeps a live run active.
 *
 * Uses the test-only lease timing seam (`test-leases`) so the proof is deterministic
 * without multi-minute sleeps and without exposing clock mutation publicly.
 */
#[cfg(feature = "test-leases")]
#[test]
fn heartbeat_keeps_live_long_checkpoint_active_past_stale_interval() {
    use distill_library::test_leases::{
        reset_lease_timing_for_test, set_heartbeat_interval_for_test,
        set_lease_stale_after_for_test,
    };

    let _timing_lock = LEASE_TIMING_TEST_LOCK.lock().expect("timing lock");
    struct ResetLeaseTiming;
    impl Drop for ResetLeaseTiming {
        fn drop(&mut self) {
            reset_lease_timing_for_test();
        }
    }
    let _reset = ResetLeaseTiming;

    set_lease_stale_after_for_test(Duration::from_millis(80));
    set_heartbeat_interval_for_test(Duration::from_millis(15));

    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_two_candidate_fixture(&fixture);

    let mut owner = Library::open(&home).expect("open");
    owner
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("pref");

    let home_t = home.clone();
    let started = Arc::new(Mutex::new(false));
    let started_t = Arc::clone(&started);
    let worker = thread::spawn(move || {
        let mut library = Library::open(&home_t).expect("worker");
        let _ = library.start_sync(SyncRequest::default(), |progress| {
            if matches!(progress, SyncProgress::RunStarted { .. }) {
                *started_t.lock().unwrap() = true;
                // Hold the first post-running callback beyond the stale interval.
                // The heartbeat must already own renewal before delivery begins.
                thread::sleep(Duration::from_millis(250));
            }
        });
    });

    while !*started.lock().unwrap() {
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_millis(200));

    let observer = Library::open(&home).expect("observer");
    let health = observer.health().expect("health");
    assert_eq!(
        health.operations_status, "active",
        "live long checkpoint must remain active via heartbeat"
    );
    assert!(health
        .issues
        .iter()
        .all(|issue| issue.code != "stale_sync_operation"));

    worker.join().expect("join");
}

/**
 * OSR-015: large stdin does not deadlock; timeout/output paths clean up without leaking argv.
 */
#[cfg(all(unix, feature = "test-leases"))]
#[test]
fn provider_process_large_stdin_and_cleanup_do_not_deadlock() {
    use std::process::Command;

    let limits = ProviderProcessLimits {
        max_duration: Duration::from_secs(2),
        max_stdout_bytes: 64,
        max_stderr_bytes: 64,
    };
    // cat reads stdin and writes stdout; large stdin previously could deadlock if
    // written synchronously before reader threads started.
    let large = vec![b'x'; 256 * 1024];
    let err = run_bounded_command(Command::new("/bin/cat"), limits, Some(&large)).expect_err("cap");
    assert_eq!(err.code(), "provider_process_bound_exceeded");
    let message = err.to_string();
    assert!(!message.contains("cat"));
    assert!(!message.contains("xxxx"));
}
