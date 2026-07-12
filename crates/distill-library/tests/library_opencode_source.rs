//! OpenCode SourceAdapter Library contracts for issue #28.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use distill_library::{Library, SourceDetectRequest, SyncProgress, SyncRequest};
use tempfile::TempDir;

/// Serialize PATH / env-bound OpenCode harness mutations across integration tests.
static HARNESS_LOCK: Mutex<()> = Mutex::new(());

/**
 * Mixed OpenCode export covering dialogue, tools, reasoning, files, and unknown parts.
 */
fn mixed_export_json() -> String {
    r#"{"info":{"id":"ses_1","slug":"tidy-wizard","projectID":"global","directory":"/tmp/opencode-demo","title":"New session - 2026-03-26T19:15:49.354Z","version":"1.3.3","time":{"created":1774543194067,"updated":1774543475213}},"messages":[{"info":{"id":"msg_user","role":"user","time":{"created":1774543194080},"model":{"providerID":"ollama","modelID":"nemotron-cascade-2:30b"}},"parts":[{"id":"part_user_text","type":"text","text":"Do I have a project for VisiBible in GTDspace?"}]},{"info":{"id":"msg_assistant","role":"assistant","parentID":"msg_user","time":{"created":1774543194090},"providerID":"ollama","modelID":"nemotron-cascade-2:30b"},"parts":[{"id":"part_step_start","type":"step-start"},{"id":"part_reasoning","type":"reasoning","text":"We should search GTDspace first."},{"id":"part_tool","type":"tool","tool":"gtdspace_workspace_search","callID":"call_1","state":{"status":"completed","title":"Search workspace","output":"{\"matches\":[]}","attachments":[{"type":"file","mime":"text/plain","filename":"report.txt","url":"file:///tmp/demo/report.txt"}]}},{"id":"part_file","type":"file","mime":"text/plain","filename":"input.txt","url":"file:///tmp/demo/input.txt","source":{"type":"file","path":"/tmp/demo/input.txt"}},{"id":"part_agent","type":"agent","name":"planner"},{"id":"part_text","type":"text","text":"Yes. Your GTDSpace includes a project named Visibible."},{"id":"part_step_finish","type":"step-finish","reason":"stop","tokens":{"input":12,"output":34,"reasoning":0,"cache":{"read":0,"write":0}}}]}]}"#
    .to_string()
}

/**
 * Session discovery JSON returned by the fake `opencode db ... --format json` command.
 */
fn sessions_json() -> String {
    r#"[{"id":"ses_1","title":"New session - 2026-03-26T19:15:49.354Z","directory":"/tmp/opencode-demo","version":"1.3.3","time_created":1774543194067,"time_updated":1774543475213,"time_archived":null,"share_url":"https://opencode.ai/share/ses_1"}]"#
        .to_string()
}

/**
 * Install a hermetic fake `opencode` under `{root}/bin` driven by fixture files.
 */
fn install_fake_opencode(root: &Path, mode: &str) -> PathBuf {
    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin");
    let export_dir = root.join("exports");
    fs::create_dir_all(&export_dir).expect("exports");
    let sessions_path = root.join("sessions.json");
    let script = bin_dir.join("opencode");

    match mode {
        "empty" => {
            fs::write(&sessions_path, "[]\n").expect("sessions");
        }
        "mixed" => {
            fs::write(&sessions_path, sessions_json()).expect("sessions");
            let export_body = format!("Exporting session: ses_1\n{}", mixed_export_json());
            fs::write(export_dir.join("ses_1.json"), export_body).expect("export");
        }
        "malformed" => {
            fs::write(&sessions_path, sessions_json()).expect("sessions");
            fs::write(
                export_dir.join("ses_1.json"),
                "Exporting session: ses_1\nnot-json\n",
            )
            .expect("export");
        }
        "fail-db" => {
            // Script exits non-zero for db queries.
        }
        "timeout" => {
            fs::write(&sessions_path, sessions_json()).expect("sessions");
            fs::write(export_dir.join("ses_1.json"), mixed_export_json()).expect("export");
        }
        "overflow" => {
            fs::write(&sessions_path, sessions_json()).expect("sessions");
            let huge = format!(
                "Exporting session: ses_1\n{{\"info\":{{\"id\":\"ses_1\"}},\"messages\":[],\"pad\":\"{}\"}}",
                "x".repeat(64 * 1024)
            );
            fs::write(export_dir.join("ses_1.json"), huge).expect("export");
        }
        other => panic!("unknown fake mode: {other}"),
    }

    let script_body = format!(
        r#"#!/bin/sh
set -eu
ROOT="{root}"
MODE="{mode}"
case "${{1:-}}" in
  db)
    if [ "${{2:-}}" = "path" ]; then
      printf '%s\n' "$ROOT/opencode.db"
      exit 0
    fi
    if [ "$MODE" = "fail-db" ]; then
      printf 'permission denied: /secret/opencode.db\n' >&2
      exit 1
    fi
    if [ "$MODE" = "timeout" ] && [ "${{3:-}}" != "" ]; then
      : # discovery stays fast
    fi
    cat "$ROOT/sessions.json"
    exit 0
    ;;
  export)
    SESSION="${{2:-}}"
    if [ "$MODE" = "timeout" ]; then
      sleep 5
    fi
    if [ "$MODE" = "fail-db" ]; then
      printf 'export failed for /secret/%s\n' "$SESSION" >&2
      exit 1
    fi
    cat "$ROOT/exports/$SESSION.json"
    exit 0
    ;;
esac
printf 'unsupported fake opencode command\n' >&2
exit 1
"#,
        root = root.display(),
        mode = mode,
    );
    fs::write(&script, script_body).expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).expect("chmod");
    }
    script
}

/**
 * Library detect reports configured OpenCode root and stays generic when missing.
 */
#[test]
fn library_detects_opencode_root_and_redacts_missing_root() {
    let _guard = HARNESS_LOCK.lock().expect("lock");
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let opencode = temp.path().join("opencode-home");
    fs::create_dir_all(&opencode).expect("root");
    install_fake_opencode(&opencode, "empty");

    let library = Library::open(&home).expect("open");
    let results = library
        .detect_sources(&[
            SourceDetectRequest {
                kind: "opencode".into(),
                configured_root: Some(opencode.display().to_string()),
            },
            SourceDetectRequest {
                kind: "opencode".into(),
                configured_root: None,
            },
            SourceDetectRequest {
                kind: "droid".into(),
                configured_root: None,
            },
        ])
        .expect("detect");

    assert_eq!(results[0].status, "ok");
    assert_eq!(results[0].display_name.as_deref(), Some("OpenCode"));
    assert!(results[0]
        .effective_data_root
        .as_deref()
        .expect("root")
        .contains("opencode-home"));
    assert!(results[0].error_message.is_none());

    assert_eq!(results[1].status, "disabled");
    assert!(results[1].error_class.is_none());

    assert_eq!(results[2].status, "unavailable");
    assert_eq!(
        results[2].error_class.as_deref(),
        Some("adapter_not_registered")
    );

    let mut library = Library::open(&home).expect("reopen");
    library
        .set_source_preference("opencode", true, None)
        .expect("enable without root");
    let missing = library
        .detect_sources(&[SourceDetectRequest {
            kind: "opencode".into(),
            configured_root: None,
        }])
        .expect("detect enabled");
    assert_eq!(missing[0].status, "missing");
    assert_eq!(
        missing[0].error_class.as_deref(),
        Some("configured_root_required")
    );
    let missing_message = missing[0].error_message.as_deref().unwrap_or("");
    assert!(
        !missing_message.contains('/'),
        "missing-root diagnostics must stay generic: {missing_message}"
    );
    assert!(
        !missing_message.to_ascii_lowercase().contains("opencode"),
        "diagnostics must not name the provider: {missing_message}"
    );
}

/**
 * Empty OpenCode root with a healthy fake CLI completes Sync without provider leakage.
 */
#[test]
fn library_sync_empty_opencode_root_completes_generically() {
    let _guard = HARNESS_LOCK.lock().expect("lock");
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let opencode = temp.path().join("opencode-home");
    fs::create_dir_all(&opencode).expect("root");
    install_fake_opencode(&opencode, "empty");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("opencode", true, Some(opencode.as_path()))
        .expect("prefer");
    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.status, "completed");
    assert_eq!(result.run.accepted_captures, 0);
    assert_eq!(result.run.sources[0].source_kind, "opencode");
    assert!(result.run.sources[0].error_message.is_none());
}

/**
 * Production Sync path ingests mixed OpenCode export bytes and replays after source removal.
 */
#[test]
fn library_sync_opencode_mixed_blocks_and_replays_after_source_removal() {
    let _guard = HARNESS_LOCK.lock().expect("lock");
    clear_test_limits();
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let opencode = temp.path().join("opencode-home");
    fs::create_dir_all(&opencode).expect("root");
    install_fake_opencode(&opencode, "mixed");
    let export_path = opencode.join("exports/ses_1.json");
    let original_bytes = fs::read(&export_path).expect("original bytes");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("opencode", true, Some(opencode.as_path()))
        .expect("prefer opencode");

    let mut progress = Vec::new();
    let result = library
        .start_sync(SyncRequest::default(), |event| {
            progress.push(event);
        })
        .expect("sync");

    assert_eq!(result.run.status, "completed");
    assert_eq!(result.run.accepted_captures, 1);
    assert_eq!(result.run.successful_attempts, 1);
    assert_eq!(result.run.sources.len(), 1);
    assert_eq!(result.run.sources[0].source_kind, "opencode");
    assert_eq!(result.run.sources[0].status, "completed");
    assert!(result.run.sources[0].error_class.is_none());
    assert_eq!(result.session_identities.len(), 1);
    assert_eq!(result.session_identities[0].source_kind, "opencode");
    assert_eq!(result.session_identities[0].external_session_id, "ses_1");

    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::SourceStarted { source_kind, .. } if source_kind == "opencode"
    )));
    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::CandidateStarted { source_kind, .. } if source_kind == "opencode"
    )));
    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::CandidateFinished { source_kind, outcome, .. }
            if source_kind == "opencode" && outcome == "accepted"
    )));
    for event in &progress {
        if let SyncProgress::CandidateStarted { candidate_id, .. }
        | SyncProgress::CandidateFinished { candidate_id, .. } = event
        {
            assert_eq!(candidate_id, "opencode://session/ses_1");
            assert!(
                !candidate_id.starts_with('/'),
                "candidate identity must not leak absolute paths: {candidate_id}"
            );
        }
    }

    let detail = library
        .session_slice("opencode", "ses_1", 20, 20)
        .expect("session query")
        .expect("session present");
    assert_eq!(detail.summary.source_kind, "opencode");
    assert_eq!(
        detail.summary.title.as_deref(),
        Some("Do I have a project for VisiBible in GTDspace?")
    );
    assert_eq!(detail.project_path.as_deref(), Some("/tmp/opencode-demo"));
    assert!(detail.started_at.is_some());
    assert!(detail.updated_at.is_some());
    assert!(detail.metadata_json.contains("nemotron-cascade-2:30b"));
    assert!(detail
        .messages
        .iter()
        .any(|message| message.text.contains("Visibible")));
    assert!(detail.artifacts.len() >= 3);
    assert!(detail
        .artifacts
        .iter()
        .any(|artifact| artifact.artifact_type == "tool_call"));
    assert!(detail
        .artifacts
        .iter()
        .any(|artifact| artifact.artifact_type == "tool_result"));
    assert!(detail
        .artifacts
        .iter()
        .any(|artifact| artifact.artifact_type == "file"));
    assert!(detail
        .artifacts
        .iter()
        .any(|artifact| artifact.artifact_type == "raw_json"));

    let activity = library.recent_activity(50).expect("activity");
    assert!(activity
        .iter()
        .any(|event| event.event_type == "capture_recorded"));
    let capture_id = activity
        .iter()
        .find(|event| event.event_type == "capture_recorded")
        .and_then(|event| event.capture_id)
        .expect("capture id");
    let replayed = library.replay_capture(capture_id).expect("replay");
    assert_eq!(replayed, original_bytes);
    assert!(
        std::str::from_utf8(&replayed)
            .expect("utf8")
            .starts_with("Exporting session: ses_1\n{"),
        "replay must preserve leading non-JSON export line"
    );

    fs::remove_dir_all(&opencode).expect("delete opencode root");
    let replayed_after = library
        .replay_capture(capture_id)
        .expect("replay after source removal");
    assert_eq!(replayed_after, original_bytes);

    let hits = library.search("Visibible", 10).expect("search");
    assert!(hits.iter().any(|hit| hit.text.contains("Visibible")));
}

/**
 * Missing executable and command failures stay redacted on detect/sync seams.
 */
#[test]
fn library_opencode_missing_executable_and_command_failure_are_redacted() {
    let _guard = HARNESS_LOCK.lock().expect("lock");
    clear_test_limits();
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let missing_root = temp.path().join("opencode-missing");
    fs::create_dir_all(&missing_root).expect("root");

    // Isolate PATH so a machine-local opencode cannot satisfy detection.
    let empty_path = temp.path().join("empty-path");
    fs::create_dir_all(&empty_path).expect("empty path");
    let previous_path = std::env::var_os("PATH");
    std::env::set_var("PATH", &empty_path);

    let library = Library::open(&home).expect("open");
    let missing = library
        .detect_sources(&[SourceDetectRequest {
            kind: "opencode".into(),
            configured_root: Some(missing_root.display().to_string()),
        }])
        .expect("detect");
    assert_eq!(missing[0].status, "unavailable");
    assert_eq!(
        missing[0].error_class.as_deref(),
        Some("executable_not_found")
    );
    let message = missing[0].error_message.as_deref().unwrap_or("");
    assert!(!message.contains('/'));
    assert!(!message.to_ascii_lowercase().contains("opencode"));
    assert!(!message.contains("secret"));

    if let Some(path) = previous_path {
        std::env::set_var("PATH", path);
    } else {
        std::env::remove_var("PATH");
    }

    let fail_root = temp.path().join("opencode-fail");
    fs::create_dir_all(&fail_root).expect("fail root");
    install_fake_opencode(&fail_root, "fail-db");
    let mut library = Library::open(&home).expect("reopen");
    library
        .set_source_preference("opencode", true, Some(fail_root.as_path()))
        .expect("prefer");
    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.sources[0].source_kind, "opencode");
    assert_eq!(result.run.sources[0].status, "failed");
    assert_eq!(
        result.run.sources[0].error_class.as_deref(),
        Some("source_adapter")
    );
    let sync_message = result.run.sources[0].error_message.as_deref().unwrap_or("");
    assert!(
        !sync_message.contains('/'),
        "sync diagnostics must not leak paths: {sync_message}"
    );
    assert!(
        !sync_message.contains("secret"),
        "sync diagnostics must not leak provider stderr: {sync_message}"
    );
}

/**
 * Timeout, overflow, and malformed export failures classify without path leakage.
 */
#[test]
fn library_opencode_timeout_overflow_and_malformed_export_classify() {
    let _guard = HARNESS_LOCK.lock().expect("lock");
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");

    // Timeout on export.
    let timeout_root = temp.path().join("opencode-timeout");
    fs::create_dir_all(&timeout_root).expect("timeout root");
    install_fake_opencode(&timeout_root, "timeout");
    // Keep discovery fast but still expire the export `sleep 5` path.
    std::env::set_var("DISTILL_TEST_OPENCODE_TIMEOUT_MS", "500");
    std::env::remove_var("DISTILL_TEST_OPENCODE_MAX_STDOUT_BYTES");
    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("opencode", true, Some(timeout_root.as_path()))
        .expect("prefer timeout");
    let timeout_result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("timeout sync");
    assert_eq!(timeout_result.run.sources[0].status, "failed");
    assert_eq!(
        timeout_result.run.sources[0].error_class.as_deref(),
        Some("source_adapter")
    );
    let timeout_message = timeout_result.run.sources[0]
        .error_message
        .as_deref()
        .unwrap_or("");
    assert_eq!(timeout_message, "source sync failed");
    assert!(!timeout_message.contains('/'));

    // Overflow on export stdout.
    let overflow_root = temp.path().join("opencode-overflow");
    fs::create_dir_all(&overflow_root).expect("overflow root");
    install_fake_opencode(&overflow_root, "overflow");
    std::env::set_var("DISTILL_TEST_OPENCODE_TIMEOUT_MS", "5000");
    std::env::set_var("DISTILL_TEST_OPENCODE_MAX_STDOUT_BYTES", "1024");
    let mut library = Library::open(&home).expect("reopen overflow");
    library
        .set_source_preference("opencode", true, Some(overflow_root.as_path()))
        .expect("prefer overflow");
    let overflow_result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("overflow sync");
    assert_eq!(overflow_result.run.sources[0].status, "failed");
    assert_eq!(
        overflow_result.run.sources[0].error_class.as_deref(),
        Some("source_adapter")
    );
    let overflow_message = overflow_result.run.sources[0]
        .error_message
        .as_deref()
        .unwrap_or("");
    assert_eq!(overflow_message, "source sync failed");
    assert!(!overflow_message.contains('/'));
    assert!(!overflow_message.contains("pad"));

    // Malformed export payload.
    clear_test_limits();
    let malformed_root = temp.path().join("opencode-malformed");
    fs::create_dir_all(&malformed_root).expect("malformed root");
    install_fake_opencode(&malformed_root, "malformed");
    let mut library = Library::open(&home).expect("reopen malformed");
    library
        .set_source_preference("opencode", true, Some(malformed_root.as_path()))
        .expect("prefer malformed");
    let malformed_result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("malformed sync");
    assert_eq!(malformed_result.run.sources[0].status, "failed");
    assert_eq!(
        malformed_result.run.sources[0].error_class.as_deref(),
        Some("source_adapter")
    );
    let malformed_message = malformed_result.run.sources[0]
        .error_message
        .as_deref()
        .unwrap_or("");
    assert_eq!(malformed_message, "source sync failed");
    assert!(!malformed_message.contains('/'));
    assert!(!malformed_message.contains("not-json"));

    clear_test_limits();
}

fn clear_test_limits() {
    std::env::remove_var("DISTILL_TEST_OPENCODE_TIMEOUT_MS");
    std::env::remove_var("DISTILL_TEST_OPENCODE_MAX_STDOUT_BYTES");
}
