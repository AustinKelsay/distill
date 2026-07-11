//! Tauri host seam: command runner and typed event/error translation.
//!
//! These tests exercise the host boundary without claiming renderer consistency.

use std::fs;
use std::path::Path;

use distill_desktop_lib::{execute_fixture_journey, validate_fixture_journey_request, HostError};
use distill_library::FixtureJourneyPhase;
use tempfile::TempDir;

/**
 * Write a minimal Fixture root for host boundary tests.
 */
fn write_basic_fixture(root: &Path) {
    let captures = root.join("captures");
    fs::create_dir_all(&captures).expect("captures");
    fs::write(
        captures.join("hello.jsonl"),
        concat!(
            r#"{"record_type":"session_meta","title":"Host Fixture","summary":"host"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"Hello from host fixture"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"Host greeting"}"#,
            "\n",
        ),
    )
    .expect("capture");
    fs::write(
        root.join("distill.fixture.json"),
        r#"{
  "version": 1,
  "captures": [
    {
      "id": "hello",
      "kind": "file",
      "relative_path": "captures/hello.jsonl",
      "external_session_id": "fixture-session-host",
      "title": "Host Fixture"
    }
  ]
}"#,
    )
    .expect("manifest");
}

#[test]
fn validates_empty_home_as_typed_host_error() {
    let err = validate_fixture_journey_request("  ", "/tmp").expect_err("empty home");
    assert_eq!(err.code, "validation");
    assert!(err.message.contains("home"));
}

#[test]
fn validates_missing_fixture_directory() {
    let temp = TempDir::new().expect("temp");
    let missing = temp.path().join("nope");
    let err = validate_fixture_journey_request(
        temp.path().join("home").to_str().unwrap(),
        missing.to_str().unwrap(),
    )
    .expect_err("missing fixture");
    assert_eq!(err.code, "validation");
}

#[test]
fn host_runner_emits_progress_and_typed_results() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);

    let request =
        validate_fixture_journey_request(home.to_str().unwrap(), fixture.to_str().unwrap())
            .expect("validate");

    let mut phases = Vec::new();
    let result = execute_fixture_journey(&request, |phase| phases.push(phase)).expect("run");

    assert_eq!(
        phases,
        [
            FixtureJourneyPhase::DetectingSource,
            FixtureJourneyPhase::SyncingCaptures,
            FixtureJourneyPhase::LoadingSession,
            FixtureJourneyPhase::CheckingHealth,
        ]
    );
    assert_eq!(result.source.kind, "fixture");
    assert_eq!(result.sync.accepted_captures, 1);
    let session = result.session.expect("session");
    assert_eq!(session.summary.external_session_id, "fixture-session-host");
    assert!(result.health.ok);
}

#[test]
fn host_translates_library_detect_failure() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");

    let request =
        validate_fixture_journey_request(home.to_str().unwrap(), fixture.to_str().unwrap())
            .expect("validate");

    let err: HostError = execute_fixture_journey(&request, |_| {}).expect_err("detect fail");
    assert_eq!(err.code, "source_adapter");
    assert!(!err.message.is_empty());
}
