//! CLI seam: invoke the real `distill` binary against temporary homes and Fixtures.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/**
 * Write a minimal Fixture root with one file-backed Capture Candidate.
 */
fn write_basic_fixture(root: &Path) {
    let captures = root.join("captures");
    fs::create_dir_all(&captures).expect("captures dir");
    fs::write(
        captures.join("hello.jsonl"),
        concat!(
            r#"{"record_type":"session_meta","title":"CLI Fixture","summary":"cli"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"Hello from CLI fixture"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"CLI greeting"}"#,
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
      "external_session_id": "fixture-session-cli",
      "title": "CLI Fixture"
    }
  ]
}"#,
    )
    .expect("manifest");
}

/**
 * Resolve the Cargo-built `distill` binary for integration tests.
 */
fn distill_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_distill"))
}

#[test]
fn cli_fixture_journey_human_exit_zero() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);

    let output = Command::new(distill_bin())
        .arg("--home")
        .arg(&home)
        .arg("--fixture")
        .arg(&fixture)
        .output()
        .expect("run distill");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ok: true"));
    assert!(stdout.contains("source.kind: fixture"));
    assert!(stdout.contains("sync.accepted_captures: 1"));
    assert!(stdout.contains("session.identity: fixture:fixture-session-cli"));
    assert!(stdout.contains("session.accepted_capture_count: 1"));
    assert!(stdout.contains("session.normalization_attempt_count: 1"));
    assert!(stdout.contains("session.successful_projection_generation: 1"));
    assert!(stdout.contains("health.ok: true"));
}

#[test]
fn cli_fixture_journey_json_exit_zero() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);

    let output = Command::new(distill_bin())
        .arg("--home")
        .arg(&home)
        .arg("--fixture")
        .arg(&fixture)
        .arg("--format")
        .arg("json")
        .output()
        .expect("run distill");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(value["ok"], true);
    assert_eq!(value["source"]["kind"], "fixture");
    assert_eq!(value["sync"]["accepted_captures"], 1);
    assert_eq!(
        value["session"]["summary"]["external_session_id"],
        "fixture-session-cli"
    );
    assert_eq!(value["session"]["summary"]["accepted_capture_count"], 1);
    assert_eq!(
        value["session"]["summary"]["normalization_attempt_count"],
        1
    );
    assert_eq!(
        value["session"]["summary"]["successful_projection_generation"],
        1
    );
    assert_eq!(value["health"]["ok"], true);
    assert!(value["phases"].as_array().expect("phases").len() >= 4);
}

#[test]
fn cli_missing_fixture_dir_exits_usage() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let missing = temp.path().join("missing-fixture");

    let output = Command::new(distill_bin())
        .arg("--home")
        .arg(&home)
        .arg("--fixture")
        .arg(&missing)
        .arg("--format")
        .arg("json")
        .output()
        .expect("run distill");

    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&output.stderr).expect("json stderr");
    assert_eq!(value["error"], "usage");
}

#[test]
fn cli_invalid_fixture_manifest_exits_runtime() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    // Directory exists but has no manifest — detect fails as Library/runtime error.

    let output = Command::new(distill_bin())
        .arg("--home")
        .arg(&home)
        .arg("--fixture")
        .arg(&fixture)
        .arg("--format")
        .arg("json")
        .output()
        .expect("run distill");

    assert_eq!(output.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&output.stderr).expect("json stderr");
    assert_eq!(value["error"], "source_adapter");
}

#[test]
fn cli_health_reports_typed_status_human_and_json() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);

    let journey = Command::new(distill_bin())
        .arg("--home")
        .arg(&home)
        .arg("--fixture")
        .arg(&fixture)
        .output()
        .expect("journey");
    assert_eq!(journey.status.code(), Some(0));

    let human = Command::new(distill_bin())
        .arg("health")
        .arg("--home")
        .arg(&home)
        .output()
        .expect("health human");
    assert_eq!(human.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(stdout.contains("ok: true"));
    assert!(stdout.contains("health.schema_status: ok"));
    assert!(stdout.contains("health.orphan_status: ok"));
    assert!(stdout.contains("health.incomplete_status: ok"));

    let json = Command::new(distill_bin())
        .arg("health")
        .arg("--home")
        .arg(&home)
        .arg("--format")
        .arg("json")
        .output()
        .expect("health json");
    assert_eq!(json.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("json");
    assert_eq!(value["ok"], true);
    assert_eq!(value["health"]["fts_status"], "ok");
    assert_eq!(value["health"]["staging_status"], "ok");
}

#[test]
fn cli_repair_requires_confirm_flag() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");

    let denied = Command::new(distill_bin())
        .arg("repair")
        .arg("--home")
        .arg(&home)
        .arg("--format")
        .arg("json")
        .output()
        .expect("repair without confirm");
    assert_eq!(denied.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&denied.stderr).expect("json");
    assert_eq!(value["error"], "usage");
    assert!(value["message"]
        .as_str()
        .unwrap_or("")
        .contains("--confirm"));

    let allowed = Command::new(distill_bin())
        .arg("repair")
        .arg("--home")
        .arg(&home)
        .arg("--confirm")
        .arg("--format")
        .arg("json")
        .output()
        .expect("repair with confirm");
    assert_eq!(
        allowed.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&allowed.stdout).expect("json");
    assert!(!value["repair"]["actions"].as_array().unwrap().is_empty());
}

#[test]
fn cli_sources_and_sync_commands_human_and_json() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);

    let set = Command::new(distill_bin())
        .arg("sources")
        .arg("set")
        .arg("--home")
        .arg(&home)
        .arg("--kind")
        .arg("fixture")
        .arg("--enable")
        .arg("--root")
        .arg(&fixture)
        .arg("--format")
        .arg("json")
        .output()
        .expect("sources set");
    assert_eq!(
        set.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&set.stderr)
    );

    let sync = Command::new(distill_bin())
        .arg("sync")
        .arg("start")
        .arg("--home")
        .arg(&home)
        .arg("--format")
        .arg("human")
        .output()
        .expect("sync start");
    assert_eq!(
        sync.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&sync.stderr)
    );
    let stdout = String::from_utf8_lossy(&sync.stdout);
    assert!(stdout.contains("sync.status: completed"));
    assert!(stdout.contains("sync.accepted_captures: 1"));

    let enable_codex = Command::new(distill_bin())
        .args([
            "sources",
            "set",
            "--home",
            home.to_str().unwrap(),
            "--kind",
            "codex",
            "--enable",
            "--format",
            "json",
        ])
        .output()
        .expect("enable codex");
    assert_eq!(enable_codex.status.code(), Some(0));

    let warning = Command::new(distill_bin())
        .args([
            "sync",
            "start",
            "--home",
            home.to_str().unwrap(),
            "--format",
            "human",
        ])
        .output()
        .expect("warning sync");
    assert_eq!(warning.status.code(), Some(0));
    let warning_stdout = String::from_utf8_lossy(&warning.stdout);
    assert!(warning_stdout.contains("sync.status: warning"));
    assert!(warning_stdout.contains("sync.warning:"));

    let warning_json = Command::new(distill_bin())
        .args([
            "sync",
            "start",
            "--home",
            home.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("warning json sync");
    assert_eq!(warning_json.status.code(), Some(0));
    let warning_value: serde_json::Value =
        serde_json::from_slice(&warning_json.stdout).expect("warning json");
    let run_id = warning_value["run"]["id"].as_i64().expect("run id");
    assert!(warning_value["run"]["warning_details"]
        .as_array()
        .is_some_and(|details| !details.is_empty()));

    let cancel = Command::new(distill_bin())
        .args([
            "sync",
            "cancel",
            "--home",
            home.to_str().unwrap(),
            "--id",
            &run_id.to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("cancel terminal run");
    assert_eq!(cancel.status.code(), Some(0));
    let cancel_value: serde_json::Value =
        serde_json::from_slice(&cancel.stdout).expect("cancel json");
    assert_eq!(cancel_value["run"]["status"], "warning");

    let status = Command::new(distill_bin())
        .args([
            "sync",
            "status",
            "--home",
            home.to_str().unwrap(),
            "--id",
            &run_id.to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("status");
    assert_eq!(status.status.code(), Some(0));
    let status_value: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("status json");
    assert_eq!(status_value["run"]["id"], run_id);
}

#[test]
fn cli_sessions_list_search_and_detail_are_bounded() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);

    let journey = Command::new(distill_bin())
        .args([
            "--home",
            home.to_str().unwrap(),
            "--fixture",
            fixture.to_str().unwrap(),
        ])
        .output()
        .expect("journey");
    assert_eq!(journey.status.code(), Some(0));

    let list = Command::new(distill_bin())
        .args([
            "sessions",
            "list",
            "--home",
            home.to_str().unwrap(),
            "--query",
            "Hello",
            "--lane",
            "all",
            "--limit",
            "1",
            "--format",
            "json",
        ])
        .output()
        .expect("session list");
    assert_eq!(list.status.code(), Some(0));
    let list_value: serde_json::Value = serde_json::from_slice(&list.stdout).expect("list json");
    assert_eq!(list_value["items"].as_array().expect("items").len(), 1);
    assert_eq!(
        list_value["items"][0]["external_session_id"],
        "fixture-session-cli"
    );
    assert!(list_value.get("next_cursor").is_some());

    let zero_token = Command::new(distill_bin())
        .args([
            "sessions",
            "list",
            "--home",
            home.to_str().unwrap(),
            "--query",
            "!!! ///",
            "--lane",
            "all",
            "--format",
            "json",
        ])
        .output()
        .expect("zero-token session list");
    assert_eq!(zero_token.status.code(), Some(0));
    let zero_value: serde_json::Value =
        serde_json::from_slice(&zero_token.stdout).expect("zero-token json");
    assert!(zero_value["items"].as_array().expect("items").is_empty());

    let detail = Command::new(distill_bin())
        .args([
            "sessions",
            "detail",
            "--home",
            home.to_str().unwrap(),
            "--source-kind",
            "fixture",
            "--external-session-id",
            "fixture-session-cli",
            "--message-limit",
            "1",
            "--artifact-limit",
            "1",
            "--format",
            "json",
        ])
        .output()
        .expect("session detail");
    assert_eq!(detail.status.code(), Some(0));
    let detail_value: serde_json::Value =
        serde_json::from_slice(&detail.stdout).expect("detail json");
    assert_eq!(
        detail_value["session"]["summary"]["external_session_id"],
        "fixture-session-cli"
    );
    assert_eq!(
        detail_value["session"]["messages"]
            .as_array()
            .expect("messages")
            .len(),
        1
    );
    assert!(detail_value["session"]["next_message_cursor"].is_string());
}
