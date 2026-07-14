//! CLI seam: independent `sources detect` over `Library::detect_sources`.
//!
//! Proves sibling-failure isolation, redacted caller diagnostics, usage exits,
//! and read-only behavior (no Activity / Sync Run mutation).

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
            r#"{"record_type":"session_meta","title":"CLI Detect Fixture","summary":"detect"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"Hello from detect fixture"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"Detect greeting"}"#,
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
      "external_session_id": "fixture-session-detect",
      "title": "CLI Detect Fixture"
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

/**
 * Run the real `distill` binary and parse JSON stdout on success.
 */
fn distill_ok_json(args: &[&str]) -> serde_json::Value {
    let output = Command::new(distill_bin())
        .args(args)
        .output()
        .expect("run distill");
    assert_eq!(
        output.status.code(),
        Some(0),
        "args={args:?} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("json stdout")
}

/**
 * Run the real `distill` binary and parse JSON stderr on a non-zero exit.
 */
fn distill_err_json(args: &[&str], expected_code: i32) -> serde_json::Value {
    let output = Command::new(distill_bin())
        .args(args)
        .output()
        .expect("run distill");
    assert_eq!(
        output.status.code(),
        Some(expected_code),
        "args={args:?} stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stderr).expect("json stderr")
}

/**
 * TCC-008: mixed Fixture ok/unhealthy plus disabled/unavailable siblings isolate;
 * diagnostics stay redacted; human/JSON remain stable; exit `0` for typed batches.
 */
#[test]
fn cli_sources_detect_json_isolates_and_redacts() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let good = temp.path().join("good root");
    let bad = temp.path().join("bad root with spaces and secret token");
    fs::create_dir_all(&good).expect("good");
    fs::create_dir_all(&bad).expect("bad");
    write_basic_fixture(&good);

    let home_s = home.to_str().unwrap();
    let bad_req = format!("fixture={}", bad.display());
    let good_req = format!("fixture={}", good.display());
    let missing_req = format!(
        "codex={}",
        temp.path().join("missing root with secret token").display()
    );

    let json = distill_ok_json(&[
        "sources",
        "detect",
        "--home",
        home_s,
        "--request",
        &bad_req,
        "--request",
        &good_req,
        "--request",
        "droid",
        "--request",
        &missing_req,
        "--format",
        "json",
    ]);

    assert_eq!(json["ok"], true);
    let results = json["results"].as_array().expect("results");
    assert_eq!(results.len(), 4);
    assert_eq!(results[0]["status"], "unhealthy");
    assert_eq!(results[1]["status"], "ok");
    assert_eq!(results[2]["status"], "disabled");
    assert_eq!(results[2]["display_name"], "Droid");
    assert_eq!(results[3]["status"], "unhealthy");
    assert_eq!(results[3]["error_class"], "invalid_configured_root");

    let bad_message = results[0]["error_message"].as_str().unwrap_or("");
    assert!(!bad_message.contains("secret"));
    assert!(!bad_message.contains("spaces"));
    assert!(!bad_message.contains('/'));
    let codex_message = results[3]["error_message"].as_str().unwrap_or("");
    assert!(!codex_message.contains("secret"));
    let serialized = serde_json::to_string(&json).expect("serialize safe output");
    assert!(!serialized.contains("good root"));
    assert!(!serialized.contains("bad root"));
    assert!(!serialized.contains("missing root"));

    let human = Command::new(distill_bin())
        .args([
            "sources",
            "detect",
            "--home",
            home_s,
            "--request",
            &good_req,
            "--request",
            &missing_req,
        ])
        .output()
        .expect("human");
    assert_eq!(human.status.code(), Some(0));
    let out = String::from_utf8_lossy(&human.stdout);
    assert!(out.contains("detect.count: 2"));
    assert!(out.contains("detect.fixture status=ok"));
    assert!(out.contains("detect.codex status=unhealthy"));
    assert!(!out.contains("secret"));
    assert!(!out.contains("good root"));
}

/**
 * TCC-009: usage validation exits `2`; detection does not mutate Activity or Sync Runs.
 */
#[test]
fn cli_sources_detect_usage_and_no_mutation() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let good = temp.path().join("good");
    fs::create_dir_all(&good).expect("good");
    write_basic_fixture(&good);
    let home_s = home.to_str().unwrap();
    let good_req = format!("fixture={}", good.display());

    let empty_root = distill_err_json(
        &[
            "sources",
            "detect",
            "--home",
            home_s,
            "--request",
            "fixture=",
            "--format",
            "json",
        ],
        2,
    );
    assert_eq!(empty_root["error"], "usage");

    let empty_request = distill_err_json(
        &[
            "sources",
            "detect",
            "--home",
            home_s,
            "--request",
            "   ",
            "--format",
            "json",
        ],
        2,
    );
    assert_eq!(empty_request["error"], "usage");

    let before_activity = distill_ok_json(&[
        "activity", "--home", home_s, "--limit", "50", "--format", "json",
    ]);
    assert_eq!(before_activity["items"].as_array().expect("items").len(), 0);

    let detect = distill_ok_json(&[
        "sources",
        "detect",
        "--home",
        home_s,
        "--request",
        &good_req,
        "--request",
        "not-a-real-kind",
        "--format",
        "json",
    ]);
    assert_eq!(detect["results"][0]["status"], "ok");
    assert_eq!(detect["results"][1]["status"], "unhealthy");
    assert_eq!(detect["results"][1]["error_class"], "unknown_source_kind");

    let after_activity = distill_ok_json(&[
        "activity", "--home", home_s, "--limit", "50", "--format", "json",
    ]);
    assert_eq!(
        after_activity["items"].as_array().expect("items").len(),
        0,
        "detect must not append Activity"
    );

    let sync_status = Command::new(distill_bin())
        .args(["sync", "status", "--home", home_s, "--format", "json"])
        .output()
        .expect("sync status");
    if sync_status.status.success() {
        let value: serde_json::Value =
            serde_json::from_slice(&sync_status.stdout).expect("sync json");
        assert!(
            value["run"].is_null() || value.get("run").is_none(),
            "detect must not create Sync Runs: {value}"
        );
    } else {
        assert_eq!(sync_status.status.code(), Some(1));
    }
}
