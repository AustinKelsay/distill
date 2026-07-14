//! Tauri host seam: command runner and typed event/error translation.
//!
//! These tests exercise the host boundary without claiming renderer consistency.

use std::fs;
use std::path::Path;

use distill_desktop_lib::{
    execute_fixture_journey, execute_health, execute_repair, validate_fixture_journey_request,
    validate_home_request, HostError,
};
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

/** Write two candidates so host cancellation can be observed at the next checkpoint. */
fn write_two_candidate_fixture(root: &Path) {
    let captures = root.join("captures");
    fs::create_dir_all(&captures).expect("captures");
    for (name, text) in [("one", "one"), ("two", "two")] {
        fs::write(
            captures.join(format!("{name}.jsonl")),
            format!("{{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"{text}\"}}\n"),
        )
        .expect("capture");
    }
    fs::write(
        root.join("distill.fixture.json"),
        r#"{
  "version": 1,
  "captures": [
    {"id": "one", "kind": "file", "relative_path": "captures/one.jsonl", "external_session_id": "host-cancel-one"},
    {"id": "two", "kind": "file", "relative_path": "captures/two.jsonl", "external_session_id": "host-cancel-two"}
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

#[test]
fn host_health_and_repair_require_home_and_confirm() {
    let err = validate_home_request("  ").expect_err("empty home");
    assert_eq!(err.code, "validation");

    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);

    let journey =
        validate_fixture_journey_request(home.to_str().unwrap(), fixture.to_str().unwrap())
            .expect("validate journey");
    execute_fixture_journey(&journey, |_| {}).expect("journey");

    let request = validate_home_request(home.to_str().unwrap()).expect("validate home");
    let health = execute_health(&request).expect("health");
    assert!(health.ok);
    assert_eq!(health.staging_status, "ok");
    assert_eq!(health.orphan_status, "ok");
    assert_eq!(health.incomplete_status, "ok");

    let denied = execute_repair(&request, false).expect_err("confirm required");
    assert_eq!(denied.code, "validation");

    let repaired = execute_repair(&request, true).expect("repair");
    assert!(repaired.health_after.ok);
    assert!(repaired
        .actions
        .iter()
        .any(|action| action.name == "removed_staging_partials"));
}

#[test]
fn host_sync_start_and_status() {
    use distill_desktop_lib::{
        execute_set_source_preference, execute_sync_start, execute_sync_status,
        validate_source_preference_request, validate_sync_start_request,
    };

    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);

    let pref = validate_source_preference_request(
        home.to_str().unwrap(),
        "fixture",
        true,
        Some(fixture.to_str().unwrap()),
    )
    .expect("pref");
    execute_set_source_preference(&pref).expect("set");

    let request =
        validate_sync_start_request(home.to_str().unwrap(), vec!["fixture".into()]).expect("req");
    let mut progress = Vec::new();
    let result = execute_sync_start(&request, |event| progress.push(event)).expect("sync");
    assert_eq!(result.run.status, "completed");
    assert!(!progress.is_empty());

    let home_req = validate_home_request(home.to_str().unwrap()).expect("home");
    let status = execute_sync_status(&home_req, Some(result.run.id)).expect("status");
    assert_eq!(status.id, result.run.id);
    assert_eq!(status.status, "completed");
}

#[test]
fn host_sync_cancel_requests_next_candidate_checkpoint() {
    use distill_desktop_lib::{
        execute_set_source_preference, execute_sync_cancel, execute_sync_start,
        validate_source_preference_request, validate_sync_id_request, validate_sync_start_request,
    };
    use distill_library::SyncProgress;

    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_two_candidate_fixture(&fixture);

    let pref = validate_source_preference_request(
        home.to_str().unwrap(),
        "fixture",
        true,
        Some(fixture.to_str().unwrap()),
    )
    .expect("pref");
    execute_set_source_preference(&pref).expect("set");

    let request =
        validate_sync_start_request(home.to_str().unwrap(), vec!["fixture".into()]).expect("req");
    let mut cancel_response = None;
    let result = execute_sync_start(&request, |event| {
        if let SyncProgress::CandidateStarted { sync_run_id, .. } = event {
            if cancel_response.is_none() {
                let cancel_request =
                    validate_sync_id_request(home.to_str().unwrap(), sync_run_id).expect("id");
                cancel_response = Some(execute_sync_cancel(&cancel_request).expect("cancel"));
            }
        }
    })
    .expect("sync");

    assert_eq!(result.run.status, "cancelled");
    assert_eq!(cancel_response.expect("cancel response").status, "running");
}

#[test]
fn host_sessions_list_and_detail_are_typed_and_bounded() {
    use distill_desktop_lib::{
        execute_fixture_journey, execute_list_sessions, execute_session_detail,
    };
    use distill_library::{SessionDetailRequest, SessionListRequest, WorkflowLane};

    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);
    let journey =
        validate_fixture_journey_request(home.to_str().unwrap(), fixture.to_str().unwrap())
            .expect("request");
    execute_fixture_journey(&journey, |_| {}).expect("journey");
    let home_request = validate_home_request(home.to_str().unwrap()).expect("home");

    let page = execute_list_sessions(
        &home_request,
        SessionListRequest {
            query: Some("Hello".into()),
            lane: WorkflowLane::All,
            limit: 1,
            cursor: None,
        },
    )
    .expect("session page");
    assert_eq!(page.items.len(), 1);
    let detail = execute_session_detail(
        &home_request,
        SessionDetailRequest {
            source_kind: "fixture".into(),
            external_session_id: "fixture-session-host".into(),
            message_limit: 1,
            artifact_limit: 1,
            message_cursor: None,
            artifact_cursor: None,
        },
    )
    .expect("detail")
    .expect("session detail");
    assert_eq!(detail.messages.len(), 1);
    assert_eq!(detail.summary.external_session_id, "fixture-session-host");
    assert!(detail.next_message_cursor.is_some());
}

#[test]
fn host_session_curation_mutations_return_typed_snapshot() {
    use distill_desktop_lib::{
        execute_add_session_tag, execute_fixture_journey, execute_remove_session_tag,
        execute_toggle_session_label, validate_fixture_journey_request,
        validate_session_curation_request,
    };
    use distill_library::SessionCurationRequest;

    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);
    let journey =
        validate_fixture_journey_request(home.to_str().unwrap(), fixture.to_str().unwrap())
            .expect("request");
    execute_fixture_journey(&journey, |_| {}).expect("journey");

    let (home_request, tag_request) = validate_session_curation_request(
        home.to_str().unwrap(),
        SessionCurationRequest {
            source_kind: "fixture".into(),
            external_session_id: "fixture-session-host".into(),
            name: "  Topic ".into(),
        },
    )
    .expect("tag request");
    let tagged = execute_add_session_tag(&home_request, tag_request).expect("add tag");
    assert!(tagged.changed);
    assert_eq!(tagged.tags.len(), 1);
    assert_eq!(tagged.tags[0].name, "topic");
    assert_eq!(tagged.tags[0].origin, "manual");

    let removed = execute_remove_session_tag(
        &home_request,
        SessionCurationRequest {
            source_kind: "fixture".into(),
            external_session_id: "fixture-session-host".into(),
            name: "topic".into(),
        },
    )
    .expect("remove tag");
    assert!(removed.changed);
    assert!(removed.tags.is_empty());

    let (_home_request, label_request) = validate_session_curation_request(
        home.to_str().unwrap(),
        SessionCurationRequest {
            source_kind: "fixture".into(),
            external_session_id: "fixture-session-host".into(),
            name: "favorite".into(),
        },
    )
    .expect("label request");
    let labeled = execute_toggle_session_label(&home_request, label_request).expect("toggle label");
    assert!(labeled.changed);
    assert!(labeled.labels.iter().any(|label| label.name == "favorite"));
    assert_eq!(
        labeled.workflow_state,
        distill_library::WorkflowState::Favorite
    );

    let empty = validate_session_curation_request(
        home.to_str().unwrap(),
        SessionCurationRequest {
            source_kind: "  ".into(),
            external_session_id: "fixture-session-host".into(),
            name: "x".into(),
        },
    );
    assert!(empty.is_err());
}

#[test]
fn host_export_preview_publish_and_invalid_dataset() {
    use distill_desktop_lib::{
        execute_fixture_journey, execute_preview_export, execute_publish_export,
        execute_toggle_session_label, validate_export_request, validate_fixture_journey_request,
        validate_session_curation_request,
    };
    use distill_library::{ExportProgress, ExportStatus, SessionCurationRequest};

    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);
    let journey =
        validate_fixture_journey_request(home.to_str().unwrap(), fixture.to_str().unwrap())
            .expect("request");
    execute_fixture_journey(&journey, |_| {}).expect("journey");

    let invalid = validate_export_request(home.to_str().unwrap(), "favorites");
    assert!(invalid.is_err());
    let invalid_err = invalid.expect_err("invalid dataset");
    assert_eq!(invalid_err.code, "validation");
    assert!(invalid_err.message.contains("train"));

    let (home_request, label_request) = validate_session_curation_request(
        home.to_str().unwrap(),
        SessionCurationRequest {
            source_kind: "fixture".into(),
            external_session_id: "fixture-session-host".into(),
            name: "train".into(),
        },
    )
    .expect("label request");
    let labeled = execute_toggle_session_label(&home_request, label_request).expect("toggle label");
    assert!(labeled.changed);

    let preview_request =
        validate_export_request(home.to_str().unwrap(), "train").expect("preview request");
    let preview = execute_preview_export(&preview_request).expect("preview");
    assert_eq!(preview.dataset.as_str(), "train");
    assert_eq!(preview.format_id, "distill-session-jsonl-v1");
    assert_eq!(preview.eligible.len(), 1);
    assert_eq!(
        preview.eligible[0].external_session_id,
        "fixture-session-host"
    );

    let publish_request =
        validate_export_request(home.to_str().unwrap(), "train").expect("publish request");
    let mut progress = Vec::new();
    let result =
        execute_publish_export(&publish_request, |event| progress.push(event)).expect("publish");
    assert_eq!(result.status, ExportStatus::Published);
    assert_eq!(result.record_count, 1);
    assert!(result.output_path.is_some());
    assert!(progress.iter().any(|event| matches!(
        event,
        ExportProgress::Preparing { .. }
            | ExportProgress::Published { .. }
            | ExportProgress::Committed { .. }
    )));
    assert!(progress
        .iter()
        .any(|event| matches!(event, ExportProgress::Published { .. })));

    let holdout_empty =
        validate_export_request(home.to_str().unwrap(), "holdout").expect("holdout request");
    let holdout_preview = execute_preview_export(&holdout_empty).expect("holdout preview");
    assert!(holdout_preview.eligible.is_empty());
}

#[test]
fn host_export_cancellation_returns_cancelled_result_without_publication() {
    use distill_desktop_lib::{
        execute_export_cancel, execute_fixture_journey, execute_prepare_export_cancellation,
        execute_publish_export_cancellable, execute_toggle_session_label, validate_export_request,
        validate_fixture_journey_request, validate_session_curation_request,
    };
    use distill_library::{ExportProgress, ExportStatus, SessionCurationRequest};

    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);
    let journey =
        validate_fixture_journey_request(home.to_str().unwrap(), fixture.to_str().unwrap())
            .expect("request");
    execute_fixture_journey(&journey, |_| {}).expect("journey");

    let (home_request, label_request) = validate_session_curation_request(
        home.to_str().unwrap(),
        SessionCurationRequest {
            source_kind: "fixture".into(),
            external_session_id: "fixture-session-host".into(),
            name: "train".into(),
        },
    )
    .expect("label request");
    execute_toggle_session_label(&home_request, label_request).expect("label");

    let request = validate_export_request(home.to_str().unwrap(), "train").expect("export request");
    assert!(execute_prepare_export_cancellation(&request).expect("prepare cancellation"));
    assert!(execute_export_cancel(&request).expect("early cancel request"));
    let mut progress = Vec::new();
    let result = execute_publish_export_cancellable(&request, |event| {
        progress.push(event);
    })
    .expect("cancelled export is a typed result");

    assert_eq!(result.status, ExportStatus::Cancelled);
    assert!(result.output_path.is_none());
    assert!(progress
        .iter()
        .any(|event| matches!(event, ExportProgress::Preparing { .. })));
    assert_eq!(result.error_class.as_deref(), Some("cancelled"));
}
