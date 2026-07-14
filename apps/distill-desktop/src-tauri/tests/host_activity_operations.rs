//! Tauri host Activity and Operations diagnostics seam for issue #30.

use std::fs;
use std::path::Path;

use distill_desktop_lib::{
    execute_fixture_journey, execute_list_activity, execute_list_operations,
    validate_fixture_journey_request, validate_home_request,
};
use distill_library::{ActivityListRequest, OperationsRequest};
use tempfile::TempDir;

/**
 * Write a minimal Fixture root for host Activity/Operations tests.
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
fn host_activity_and_operations_pages_are_typed() {
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

    let activity = execute_list_activity(
        &home_request,
        ActivityListRequest {
            limit: 5,
            cursor: None,
        },
    )
    .expect("activity");
    assert!(!activity.items.is_empty());
    assert!(!activity.items[0].event_type.is_empty());
    assert!(activity.items[0].payload_json.is_object());

    let operations = execute_list_operations(
        &home_request,
        OperationsRequest {
            sync_limit: 5,
            export_limit: 5,
            sync_cursor: None,
            export_cursor: None,
        },
    )
    .expect("operations");
    assert!(matches!(
        operations.operations_status.as_str(),
        "ok" | "active" | "failed"
    ));
    assert!(operations
        .sync_runs
        .iter()
        .all(|run| !run.status.trim().is_empty()));
}

#[test]
fn host_rejects_empty_home_for_activity_and_operations() {
    let err = validate_home_request("  ").expect_err("empty home");
    assert_eq!(err.code, "validation");
}
