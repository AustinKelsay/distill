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

#[test]
fn cli_sessions_tag_and_label_mutations_return_curation_snapshot() {
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

    let tag_add = Command::new(distill_bin())
        .args([
            "sessions",
            "tag-add",
            "--home",
            home.to_str().unwrap(),
            "--source-kind",
            "fixture",
            "--external-session-id",
            "fixture-session-cli",
            "--name",
            "  Research ",
            "--format",
            "json",
        ])
        .output()
        .expect("tag-add");
    assert_eq!(
        tag_add.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&tag_add.stderr)
    );
    let tag_value: serde_json::Value =
        serde_json::from_slice(&tag_add.stdout).expect("tag-add json");
    assert_eq!(tag_value["ok"], true);
    assert_eq!(tag_value["curation"]["changed"], true);
    assert_eq!(
        tag_value["curation"]["identity"]["external_session_id"],
        "fixture-session-cli"
    );
    assert_eq!(tag_value["curation"]["tags"][0]["name"], "research");
    assert_eq!(tag_value["curation"]["tags"][0]["origin"], "manual");

    let tag_remove_human = Command::new(distill_bin())
        .args([
            "sessions",
            "tag-remove",
            "--home",
            home.to_str().unwrap(),
            "--source-kind",
            "fixture",
            "--external-session-id",
            "fixture-session-cli",
            "--name",
            "research",
        ])
        .output()
        .expect("tag-remove");
    assert_eq!(tag_remove_human.status.code(), Some(0));
    let tag_remove_stdout = String::from_utf8_lossy(&tag_remove_human.stdout);
    assert!(tag_remove_stdout.contains("curation.changed: true"));
    assert!(tag_remove_stdout.contains("curation.tags: none"));

    let label_toggle = Command::new(distill_bin())
        .args([
            "sessions",
            "label-toggle",
            "--home",
            home.to_str().unwrap(),
            "--source-kind",
            "fixture",
            "--external-session-id",
            "fixture-session-cli",
            "--name",
            "train",
            "--format",
            "json",
        ])
        .output()
        .expect("label-toggle");
    assert_eq!(label_toggle.status.code(), Some(0));
    let label_value: serde_json::Value =
        serde_json::from_slice(&label_toggle.stdout).expect("label-toggle json");
    assert_eq!(label_value["curation"]["changed"], true);
    assert_eq!(label_value["curation"]["labels"][0]["name"], "train");
    assert_eq!(label_value["curation"]["labels"][0]["origin"], "manual");
    assert_eq!(label_value["curation"]["workflow_state"], "train_ready");
}

#[test]
fn cli_export_preview_and_publish_after_label_toggle() {
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

    let label_toggle = Command::new(distill_bin())
        .args([
            "sessions",
            "label-toggle",
            "--home",
            home.to_str().unwrap(),
            "--source-kind",
            "fixture",
            "--external-session-id",
            "fixture-session-cli",
            "--name",
            "train",
        ])
        .output()
        .expect("label-toggle");
    assert_eq!(label_toggle.status.code(), Some(0));

    let invalid = Command::new(distill_bin())
        .args([
            "export",
            "preview",
            "--home",
            home.to_str().unwrap(),
            "--dataset",
            "favorites",
            "--format",
            "json",
        ])
        .output()
        .expect("invalid dataset");
    assert_eq!(invalid.status.code(), Some(2));
    let invalid_value: serde_json::Value =
        serde_json::from_slice(&invalid.stderr).expect("invalid json");
    assert_eq!(invalid_value["error"], "usage");
    assert!(invalid_value["message"]
        .as_str()
        .unwrap_or("")
        .contains("train"));

    let preview = Command::new(distill_bin())
        .args([
            "export",
            "preview",
            "--home",
            home.to_str().unwrap(),
            "--dataset",
            "train",
            "--format",
            "json",
        ])
        .output()
        .expect("preview");
    assert_eq!(
        preview.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let preview_value: serde_json::Value =
        serde_json::from_slice(&preview.stdout).expect("preview json");
    assert_eq!(preview_value["ok"], true);
    assert_eq!(preview_value["preview"]["dataset"], "train");
    assert_eq!(
        preview_value["preview"]["format_id"],
        "distill-session-jsonl-v1"
    );
    assert_eq!(
        preview_value["preview"]["eligible"][0]["external_session_id"],
        "fixture-session-cli"
    );

    let cancelled = Command::new(distill_bin())
        .args([
            "export",
            "publish",
            "--home",
            home.to_str().unwrap(),
            "--dataset",
            "train",
            "--format",
            "json",
            "--cancel",
        ])
        .output()
        .expect("cancelled publish");
    assert_eq!(
        cancelled.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&cancelled.stderr)
    );
    let cancelled_value: serde_json::Value =
        serde_json::from_slice(&cancelled.stdout).expect("cancelled json");
    assert_eq!(cancelled_value["export"]["status"], "cancelled");
    assert!(cancelled_value["export"]["output_path"].is_null());

    let publish_human = Command::new(distill_bin())
        .args([
            "export",
            "publish",
            "--home",
            home.to_str().unwrap(),
            "--dataset",
            "train",
        ])
        .output()
        .expect("publish");
    assert_eq!(
        publish_human.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&publish_human.stderr)
    );
    let publish_stdout = String::from_utf8_lossy(&publish_human.stdout);
    assert!(publish_stdout.contains("export.status: published"));
    assert!(publish_stdout.contains("export.dataset: train"));
    assert!(publish_stdout.contains("export.record_count: 1"));
    assert!(publish_stdout.contains("export.eligible.identity: fixture:fixture-session-cli"));

    let holdout_invalid = Command::new(distill_bin())
        .args([
            "export",
            "publish",
            "--home",
            home.to_str().unwrap(),
            "--dataset",
            "all",
            "--format",
            "json",
        ])
        .output()
        .expect("invalid publish dataset");
    assert_eq!(holdout_invalid.status.code(), Some(2));
}

#[test]
fn cli_activity_and_operations_are_stable_json_and_human() {
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

    let activity_json = Command::new(distill_bin())
        .args([
            "activity",
            "--home",
            home.to_str().unwrap(),
            "--limit",
            "2",
            "--format",
            "json",
        ])
        .output()
        .expect("activity json");
    assert_eq!(activity_json.status.code(), Some(0));
    let activity_value: serde_json::Value =
        serde_json::from_slice(&activity_json.stdout).expect("activity json");
    assert_eq!(activity_value["ok"], true);
    let items = activity_value["items"].as_array().expect("items");
    assert!(!items.is_empty());
    assert!(items[0].get("id").is_some());
    assert!(items[0].get("event_type").is_some());
    assert!(items[0].get("occurred_at").is_some());
    assert!(items[0].get("payload_json").is_some());
    assert!(activity_value.get("next_cursor").is_some());

    let activity_human = Command::new(distill_bin())
        .args(["activity", "--home", home.to_str().unwrap(), "--limit", "5"])
        .output()
        .expect("activity human");
    assert_eq!(activity_human.status.code(), Some(0));
    let human = String::from_utf8_lossy(&activity_human.stdout);
    assert!(human.contains("activity.count:"));
    assert!(human.contains("activity.event:"));

    let empty_home = Command::new(distill_bin())
        .args(["activity", "--home", "", "--format", "json"])
        .output()
        .expect("empty home");
    assert_eq!(empty_home.status.code(), Some(2));

    let zero_limit = Command::new(distill_bin())
        .args([
            "activity",
            "--home",
            home.to_str().unwrap(),
            "--limit",
            "0",
            "--format",
            "json",
        ])
        .output()
        .expect("zero limit");
    assert_eq!(zero_limit.status.code(), Some(2));

    let invalid_cursor = Command::new(distill_bin())
        .args([
            "activity",
            "--home",
            home.to_str().unwrap(),
            "--cursor",
            "not-a-cursor",
            "--format",
            "json",
        ])
        .output()
        .expect("invalid Activity cursor");
    assert_eq!(invalid_cursor.status.code(), Some(1));
    let invalid_cursor_value: serde_json::Value =
        serde_json::from_slice(&invalid_cursor.stderr).expect("invalid cursor json");
    assert_eq!(invalid_cursor_value["error"], "invalid_argument");

    let ops_json = Command::new(distill_bin())
        .args([
            "operations",
            "--home",
            home.to_str().unwrap(),
            "--sync-limit",
            "10",
            "--export-limit",
            "10",
            "--format",
            "json",
        ])
        .output()
        .expect("operations json");
    assert_eq!(ops_json.status.code(), Some(0));
    let ops_value: serde_json::Value = serde_json::from_slice(&ops_json.stdout).expect("ops json");
    assert_eq!(ops_value["ok"], true);
    assert!(ops_value["operations"]["operations_status"].is_string());
    assert!(ops_value["operations"]["sync_runs"].is_array());
    assert!(ops_value["operations"]["exports"].is_array());

    let ops_human = Command::new(distill_bin())
        .args(["operations", "--home", home.to_str().unwrap()])
        .output()
        .expect("operations human");
    assert_eq!(ops_human.status.code(), Some(0));
    let ops_out = String::from_utf8_lossy(&ops_human.stdout);
    assert!(ops_out.contains("operations.status:"));
}

/**
 * Build a minimal legacy Electron home for CLI migrate tests.
 */
fn build_legacy_home_for_cli(root: &Path) -> PathBuf {
    use rusqlite::Connection;
    use sha2::{Digest, Sha256};

    let home = root.join("legacy-home");
    fs::create_dir_all(home.join("blobs")).expect("blobs");
    fs::create_dir_all(home.join("exports")).expect("exports");
    let conn = Connection::open(home.join("distill.db")).expect("db");
    conn.execute_batch(
        r#"
        CREATE TABLE sources (
          id INTEGER PRIMARY KEY,
          kind TEXT NOT NULL UNIQUE,
          display_name TEXT NOT NULL,
          data_root TEXT,
          metadata_json TEXT NOT NULL DEFAULT '{}',
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
          updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE captures (
          id INTEGER PRIMARY KEY,
          source_id INTEGER NOT NULL REFERENCES sources(id),
          capture_kind TEXT NOT NULL,
          external_session_id TEXT,
          source_path TEXT,
          source_modified_at TEXT,
          raw_sha256 TEXT NOT NULL,
          raw_blob_path TEXT,
          raw_payload_json TEXT,
          parser_version TEXT,
          status TEXT NOT NULL DEFAULT 'captured',
          captured_at TEXT NOT NULL
        );
        CREATE TABLE sessions (
          id INTEGER PRIMARY KEY,
          source_id INTEGER NOT NULL REFERENCES sources(id),
          external_session_id TEXT NOT NULL,
          title TEXT,
          project_path TEXT,
          source_url TEXT,
          summary TEXT,
          started_at TEXT,
          updated_at TEXT,
          raw_capture_count INTEGER NOT NULL DEFAULT 0,
          metadata_json TEXT NOT NULL DEFAULT '{}',
          UNIQUE (source_id, external_session_id)
        );
        CREATE TABLE capture_records (
          id INTEGER PRIMARY KEY,
          capture_id INTEGER NOT NULL REFERENCES captures(id),
          line_no INTEGER NOT NULL,
          record_type TEXT NOT NULL,
          role TEXT,
          is_meta INTEGER NOT NULL DEFAULT 0,
          content_text TEXT,
          content_json TEXT NOT NULL,
          metadata_json TEXT NOT NULL DEFAULT '{}'
        );
        CREATE TABLE messages (
          id INTEGER PRIMARY KEY,
          session_id INTEGER NOT NULL REFERENCES sessions(id),
          ordinal INTEGER NOT NULL,
          role TEXT NOT NULL,
          text TEXT NOT NULL,
          text_hash TEXT NOT NULL,
          created_at TEXT,
          message_kind TEXT NOT NULL DEFAULT 'text',
          metadata_json TEXT NOT NULL DEFAULT '{}',
          external_message_id TEXT
        );
        CREATE TABLE artifacts (
          id INTEGER PRIMARY KEY,
          session_id INTEGER REFERENCES sessions(id),
          kind TEXT NOT NULL,
          mime_type TEXT,
          metadata_json TEXT NOT NULL DEFAULT '{}'
        );
        CREATE TABLE tags (
          id INTEGER PRIMARY KEY,
          name TEXT NOT NULL UNIQUE,
          kind TEXT NOT NULL DEFAULT 'general',
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE tag_assignments (
          id INTEGER PRIMARY KEY,
          object_type TEXT NOT NULL,
          object_id INTEGER NOT NULL,
          tag_id INTEGER NOT NULL REFERENCES tags(id),
          origin TEXT NOT NULL,
          confidence REAL,
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
          UNIQUE (object_type, object_id, tag_id, origin)
        );
        CREATE TABLE labels (
          id INTEGER PRIMARY KEY,
          name TEXT NOT NULL UNIQUE,
          scope TEXT NOT NULL DEFAULT 'session',
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE label_assignments (
          id INTEGER PRIMARY KEY,
          object_type TEXT NOT NULL,
          object_id INTEGER NOT NULL,
          label_id INTEGER NOT NULL REFERENCES labels(id),
          origin TEXT NOT NULL,
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
          UNIQUE (object_type, object_id, label_id)
        );
        CREATE TABLE activity_events (
          id INTEGER PRIMARY KEY,
          event_type TEXT NOT NULL,
          object_type TEXT NOT NULL,
          object_id INTEGER,
          session_id INTEGER,
          payload_json TEXT NOT NULL DEFAULT '{}',
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE exports (
          id INTEGER PRIMARY KEY,
          export_type TEXT NOT NULL,
          label_filter TEXT,
          output_path TEXT NOT NULL,
          record_count INTEGER NOT NULL DEFAULT 0,
          metadata_json TEXT NOT NULL DEFAULT '{}',
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .expect("schema");
    let text = "cli legacy capture\n";
    let sha = hex::encode(Sha256::digest(text.as_bytes()));
    let payload = serde_json::json!({
        "contentRef": {
            "kind": "inline",
            "mediaType": "text/plain",
            "text": text,
            "sha256": sha,
            "byteSize": text.len()
        }
    });
    conn.execute(
        "INSERT INTO sources (kind, display_name) VALUES ('fixture', 'Fixture')",
        [],
    )
    .unwrap();
    let source_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO captures (
            source_id, capture_kind, external_session_id, source_path, raw_sha256,
            raw_payload_json, parser_version, status, captured_at
         ) VALUES (?1, 'file', 'cli-legacy-1', 'a.jsonl', ?2, ?3, 'v0', 'normalized', '2026-01-01T00:00:00Z')",
        rusqlite::params![source_id, sha, payload.to_string()],
    )
    .unwrap();
    let capture_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO capture_records (
            capture_id, line_no, record_type, role, is_meta, content_text, content_json
         ) VALUES (?1, 1, 'message', 'user', 0, 'hello', '{}')",
        [capture_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (
            source_id, external_session_id, title, raw_capture_count, metadata_json
         ) VALUES (?1, 'cli-legacy-1', 'CLI Legacy', 1, '{}')",
        [source_id],
    )
    .unwrap();
    let session_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO messages (
            session_id, ordinal, role, text, text_hash, message_kind, metadata_json
         ) VALUES (?1, 0, 'user', 'hello', 'x', 'text', '{}')",
        [session_id],
    )
    .unwrap();
    drop(conn);
    home
}

#[test]
fn cli_migrate_legacy_json_and_human_and_alias() {
    let temp = TempDir::new().expect("temp");
    let legacy = build_legacy_home_for_cli(temp.path());
    let home = temp.path().join("native");
    let before = fs::read(legacy.join("distill.db")).expect("db bytes");

    let json = Command::new(distill_bin())
        .args([
            "migrate",
            "--home",
            home.to_str().unwrap(),
            "--source",
            legacy.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("migrate json");
    assert_eq!(
        json.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&json.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("json");
    assert_eq!(value["ok"], true);
    assert_eq!(value["report"]["counts"]["sessions"], 1);
    assert!(
        value["report"]["source_fingerprint"]
            .as_str()
            .unwrap()
            .len()
            > 10
    );
    let report_text = String::from_utf8_lossy(&json.stdout);
    assert!(!report_text.contains(legacy.to_string_lossy().as_ref()));

    let after = fs::read(legacy.join("distill.db")).expect("db after");
    assert_eq!(before, after, "legacy db unchanged");

    let human = Command::new(distill_bin())
        .args([
            "import-legacy",
            "--home",
            home.to_str().unwrap(),
            "--source",
            legacy.to_str().unwrap(),
        ])
        .output()
        .expect("import-legacy alias");
    assert_eq!(human.status.code(), Some(0));
    let out = String::from_utf8_lossy(&human.stdout);
    assert!(out.contains("reused_prior_import: true"));
    assert!(out.contains("counts.sessions: 1"));

    let usage = Command::new(distill_bin())
        .args([
            "migrate",
            "--home",
            "",
            "--source",
            legacy.to_str().unwrap(),
        ])
        .output()
        .expect("usage");
    assert_eq!(usage.status.code(), Some(2));

    let same = Command::new(distill_bin())
        .args([
            "migrate",
            "--home",
            home.to_str().unwrap(),
            "--source",
            home.to_str().unwrap(),
        ])
        .output()
        .expect("same path");
    assert_eq!(same.status.code(), Some(1));
}
