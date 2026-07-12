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
