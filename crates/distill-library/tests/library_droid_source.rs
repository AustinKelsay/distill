//! Factory Droid SourceAdapter Library contracts for issue #29.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use distill_library::{Library, SourceDetectRequest, SyncProgress, SyncRequest};
use tempfile::TempDir;

/// Serialize HOME mutations across Droid default-root detection tests.
static HOME_LOCK: Mutex<()> = Mutex::new(());

/**
 * Write a mixed Droid session covering dialogue, tools, thinking, image, file, and unknown role.
 */
fn write_mixed_session(root: &Path) -> PathBuf {
    let session_id = "123e4567-e89b-12d3-a456-426614174000";
    let path = root.join("ws-demo").join(format!("{session_id}.jsonl"));
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    let body = concat!(
        r#"{"type":"session_start","id":"123e4567-e89b-12d3-a456-426614174000","title":"Droid mixed content fixture","owner":"plebdev","cwd":"/tmp/droid-demo"}"#,
        "\n",
        r#"{"type":"message","id":"u1","timestamp":"2026-04-12T18:17:28.000Z","message":{"role":"user","content":[{"type":"text","text":"Please review the screenshot and fix the layout."},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB"}}]}}"#,
        "\n",
        r#"{"type":"message","id":"a1","timestamp":"2026-04-12T18:17:29.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"I will tighten the layout."},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/tmp/droid-demo/src/app.ts"}},{"type":"tool_result","tool_use_id":"t1","content":"ok"},{"type":"file","file":{"path":"/tmp/droid-demo/src/app.ts"}},{"type":"custom_block","payload":{"x":1}}]}}"#,
        "\n",
        r#"{"type":"message","id":"u2","timestamp":"not-a-timestamp","message":{"role":"system","content":[{"type":"text","text":"unknown role stays fact"}]}}"#,
        "\n",
        r#"{"type":"todo_state","id":"todo1","todos":[]}"#,
        "\n",
    );
    fs::write(&path, body).expect("write session");
    fs::write(
        root.join("ws-demo")
            .join(format!("{session_id}.settings.json")),
        r#"{"model":"claude-sonnet-4-6","archivedAt":"2026-04-12T18:20:00.000Z"}
"#,
    )
    .expect("sidecar");
    path
}

/**
 * Library detect reports configured and default Droid roots with redacted absent diagnostics.
 */
#[test]
fn library_detects_default_and_override_droid_roots() {
    let _guard = HOME_LOCK.lock().expect("lock");
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let fake_home = temp.path().join("user-home");
    let default_sessions = fake_home.join(".factory").join("sessions");
    fs::create_dir_all(&default_sessions).expect("default sessions");
    write_mixed_session(&default_sessions);

    let override_root = temp.path().join("override-sessions");
    fs::create_dir_all(&override_root).expect("override");
    write_mixed_session(&override_root);

    let previous_home = std::env::var_os("HOME");
    std::env::set_var("HOME", &fake_home);

    let library = Library::open(&home).expect("open");
    let results = library
        .detect_sources(&[
            SourceDetectRequest {
                kind: "droid".into(),
                configured_root: Some(override_root.display().to_string()),
            },
            SourceDetectRequest {
                kind: "droid".into(),
                configured_root: None,
            },
        ])
        .expect("detect");

    assert_eq!(results[0].status, "ok");
    assert_eq!(results[0].display_name.as_deref(), Some("Droid"));
    assert!(results[0]
        .effective_data_root
        .as_deref()
        .expect("root")
        .contains("override-sessions"));
    assert!(results[0].error_message.is_none());

    // Default preference is disabled with no override root.
    assert_eq!(results[1].status, "disabled");
    assert!(results[1].error_class.is_none());

    let mut library = Library::open(&home).expect("reopen");
    library
        .set_source_preference("droid", true, None)
        .expect("enable default");
    let default_ok = library
        .detect_sources(&[SourceDetectRequest {
            kind: "droid".into(),
            configured_root: None,
        }])
        .expect("detect default");
    assert_eq!(default_ok[0].status, "ok");
    assert!(default_ok[0]
        .effective_data_root
        .as_deref()
        .expect("default root")
        .contains(".factory/sessions"));
    assert!(!default_ok[0]
        .error_message
        .as_deref()
        .unwrap_or_default()
        .contains('/'));

    fs::remove_dir_all(&default_sessions).expect("remove default");
    let absent = library
        .detect_sources(&[SourceDetectRequest {
            kind: "droid".into(),
            configured_root: None,
        }])
        .expect("detect absent");
    assert_eq!(absent[0].status, "missing");
    assert_eq!(absent[0].error_class.as_deref(), Some("root_absent"));
    let absent_message = absent[0].error_message.as_deref().unwrap_or("");
    assert!(
        !absent_message.contains('/'),
        "absent-root diagnostics must stay generic: {absent_message}"
    );
    assert!(
        !absent_message.to_ascii_lowercase().contains("droid"),
        "diagnostics must not name the provider: {absent_message}"
    );
    assert!(
        !absent_message.contains("factory"),
        "diagnostics must not leak default path fragments: {absent_message}"
    );

    match previous_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
}

/**
 * Unreadable configured Droid root stays typed and redacted.
 */
#[test]
fn library_detect_unreadable_droid_root_is_typed_and_redacted() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let missing = temp
        .path()
        .join("missing root with secret token")
        .join("sessions");

    let library = Library::open(&home).expect("open");
    let results = library
        .detect_sources(&[SourceDetectRequest {
            kind: "droid".into(),
            configured_root: Some(missing.display().to_string()),
        }])
        .expect("detect");
    assert_eq!(results[0].status, "unhealthy");
    assert_eq!(
        results[0].error_class.as_deref(),
        Some("invalid_configured_root")
    );
    let message = results[0].error_message.as_deref().unwrap_or("");
    assert!(!message.contains("secret"));
    assert!(!message.contains('/'));
    assert!(!message.to_ascii_lowercase().contains("droid"));
}

/**
 * Production Sync path ingests Droid history with generic progress and exact replay.
 */
#[test]
fn library_sync_droid_mixed_blocks_and_replays_after_source_removal() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let sessions = temp.path().join("factory-sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    let session_path = write_mixed_session(&sessions);
    let original_bytes = fs::read(&session_path).expect("original bytes");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("droid", true, Some(sessions.as_path()))
        .expect("prefer droid");

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
    assert_eq!(result.run.sources[0].source_kind, "droid");
    assert_eq!(result.run.sources[0].status, "completed");
    assert!(result.run.sources[0].error_class.is_none());
    assert!(result.run.sources[0].error_message.is_none());
    assert_eq!(result.session_identities.len(), 1);
    assert_eq!(result.session_identities[0].source_kind, "droid");
    assert_eq!(
        result.session_identities[0].external_session_id,
        "123e4567-e89b-12d3-a456-426614174000"
    );

    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::SourceStarted { source_kind, .. } if source_kind == "droid"
    )));
    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::CandidateStarted { source_kind, .. } if source_kind == "droid"
    )));
    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::CandidateFinished { source_kind, outcome, .. }
            if source_kind == "droid" && outcome == "accepted"
    )));
    for event in &progress {
        if let SyncProgress::CandidateStarted { candidate_id, .. }
        | SyncProgress::CandidateFinished { candidate_id, .. } = event
        {
            assert!(
                candidate_id.starts_with("droid://session/"),
                "candidate identity should be a logical source path: {candidate_id}"
            );
            assert!(
                !candidate_id.starts_with('/'),
                "candidate identity must not leak absolute paths: {candidate_id}"
            );
            assert!(
                !candidate_id.contains("factory-sessions"),
                "candidate identity must not leak filesystem roots: {candidate_id}"
            );
        }
    }

    let detail = library
        .session_slice("droid", "123e4567-e89b-12d3-a456-426614174000", 20, 20)
        .expect("session query")
        .expect("session present");
    assert_eq!(detail.summary.source_kind, "droid");
    assert_eq!(
        detail.summary.title.as_deref(),
        Some("Droid mixed content fixture")
    );
    assert_eq!(detail.project_path.as_deref(), Some("/tmp/droid-demo"));
    assert_eq!(detail.messages.len(), 2);
    assert_eq!(
        detail.messages[0].text,
        "Please review the screenshot and fix the layout."
    );
    assert_eq!(detail.messages[1].text, "I will tighten the layout.");
    assert!(detail.messages.iter().all(|message| {
        !message.text.contains("hidden") && !message.text.contains("unknown role")
    }));
    assert!(detail.artifacts.len() >= 4);
    assert!(detail
        .artifacts
        .iter()
        .any(|artifact| artifact.artifact_type == "image"));
    assert!(detail
        .artifacts
        .iter()
        .any(|artifact| artifact.artifact_type == "thinking"));
    assert!(detail
        .artifacts
        .iter()
        .any(|artifact| artifact.artifact_type == "tool_call"
            || artifact.artifact_type == "tool_use"));
    assert!(detail
        .artifacts
        .iter()
        .any(|artifact| artifact.artifact_type == "tool_result"));
    assert!(detail.artifacts.iter().any(
        |artifact| artifact.artifact_type == "file" || artifact.artifact_type == "custom_block"
    ));
    assert!(detail
        .artifacts
        .iter()
        .all(|artifact| artifact.capture_fact_id.is_some()));

    let activity = library.recent_activity(50).expect("activity");
    assert!(activity
        .iter()
        .any(|event| event.event_type == "capture_recorded"));
    assert!(activity
        .iter()
        .any(|event| event.event_type == "sync_completed"));

    let capture_ids: Vec<i64> = activity
        .iter()
        .filter(|event| event.event_type == "capture_recorded")
        .filter_map(|event| event.capture_id)
        .collect();
    assert_eq!(capture_ids.len(), 1);
    let capture_id = capture_ids[0];
    let replayed = library.replay_capture(capture_id).expect("replay");
    assert_eq!(replayed, original_bytes);

    fs::remove_file(&session_path).expect("delete session source");
    fs::remove_dir_all(&sessions).expect("delete sessions root");
    let replayed_after = library
        .replay_capture(capture_id)
        .expect("replay after source removal");
    assert_eq!(replayed_after, original_bytes);

    let hits = library
        .search("Please review the screenshot", 10)
        .expect("search");
    assert!(hits
        .iter()
        .any(|hit| hit.text.contains("Please review the screenshot")));
}

/**
 * Sidecar model/archive metadata and stem/synthetic identity edges stay deterministic.
 */
#[test]
fn library_sync_sidecar_model_archive_and_synthetic_identity() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let sessions = temp.path().join("factory-sessions");
    let path = sessions.join("ws").join("stem-only.jsonl");
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    fs::write(
        &path,
        concat!(
            r#"{"type":"message","id":"m1","timestamp":"2026-04-12T18:17:28.000Z","message":{"role":"user","content":"hello from stem"}}"#,
            "\n",
        ),
    )
    .expect("session");
    fs::write(
        sessions.join("ws").join("stem-only.settings.json"),
        r#"{"model":"gpt-5.4","archivedAt":"2026-04-12T19:00:00.000Z","title":"Sidecar title"}
"#,
    )
    .expect("sidecar");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("droid", true, Some(sessions.as_path()))
        .expect("prefer");
    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.status, "completed");
    assert_eq!(
        result.session_identities[0].external_session_id,
        "stem-only"
    );

    let detail = library
        .session_slice("droid", "stem-only", 10, 10)
        .expect("query")
        .expect("present");
    let metadata: serde_json::Value =
        serde_json::from_str(&detail.metadata_json).expect("metadata json");
    assert_eq!(detail.summary.title.as_deref(), Some("Sidecar title"));
    assert_eq!(
        metadata.get("model").and_then(|value| value.as_str()),
        Some("gpt-5.4")
    );
    assert_eq!(
        metadata.get("archived").and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(detail.messages[0].text, "hello from stem");
}

/**
 * Malformed Droid JSONL accepts exact bytes, fails parse typed, and stays redacted.
 */
#[test]
fn library_sync_malformed_droid_jsonl_is_typed_and_redacted() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let sessions = temp.path().join("factory-sessions");
    let path = sessions.join("ws").join("bad.jsonl");
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    let original = "{not-json\n";
    fs::write(&path, original).expect("bad session");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("droid", true, Some(sessions.as_path()))
        .expect("prefer");
    let mut progress = Vec::new();
    let result = library
        .start_sync(SyncRequest::default(), |event| progress.push(event))
        .expect("sync");
    assert_eq!(result.run.status, "warning");
    assert_eq!(result.run.accepted_captures, 1);
    assert_eq!(result.run.failed_attempts, 1);
    assert_eq!(result.run.sources[0].source_kind, "droid");
    assert_eq!(result.run.sources[0].status, "warning");
    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::CandidateFinished { outcome, candidate_id, .. }
            if outcome == "failed"
                && candidate_id.starts_with("droid://session/")
                && !candidate_id.contains("factory-sessions")
    )));

    let activity = library.recent_activity(50).expect("activity");
    let capture_id = activity
        .iter()
        .find(|event| event.event_type == "capture_recorded")
        .and_then(|event| event.capture_id)
        .expect("capture");
    let replayed = library.replay_capture(capture_id).expect("replay");
    assert_eq!(replayed, original.as_bytes());
    assert!(activity
        .iter()
        .any(|event| event.event_type == "sync_completed"));
}

/**
 * Empty configured Droid root completes generically with Sync progress redaction intact.
 */
#[test]
fn library_sync_empty_droid_root_completes_generically() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let sessions = temp.path().join("factory-sessions");
    fs::create_dir_all(&sessions).expect("sessions");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("droid", true, Some(sessions.as_path()))
        .expect("prefer");
    let mut progress = Vec::new();
    let result = library
        .start_sync(SyncRequest::default(), |event| progress.push(event))
        .expect("sync");
    assert_eq!(result.run.status, "completed");
    assert_eq!(result.run.accepted_captures, 0);
    assert_eq!(result.run.sources[0].source_kind, "droid");
    assert!(result.run.sources[0].error_message.is_none());
    for event in &progress {
        if let SyncProgress::CandidateStarted { candidate_id, .. }
        | SyncProgress::CandidateFinished { candidate_id, .. } = event
        {
            assert!(!candidate_id.starts_with('/'));
            assert!(!candidate_id.contains(&sessions.display().to_string()));
        }
    }
}
