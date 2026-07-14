//! Claude Code SourceAdapter Library contracts for issue #27.

use std::fs;
use std::path::{Path, PathBuf};

use distill_library::{Library, SourceDetectRequest, SyncProgress, SyncRequest};
use tempfile::TempDir;

/**
 * Write a Claude home with mixed text/image/tool/thinking/unknown-role/meta rows.
 */
fn write_mixed_session(root: &Path) -> PathBuf {
    let relative = "projects/demo-project/123e4567-e89b-12d3-a456-426614174000.jsonl";
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    let body = concat!(
        r#"{"type":"user","uuid":"u1","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-03-25T11:00:00.000Z","cwd":"/tmp/demo-project","message":{"role":"user","content":[{"type":"text","text":"Please review the screenshot and fix the layout."},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB"}}]}}"#,
        "\n",
        r#"{"type":"assistant","uuid":"a1","parentUuid":"u1","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-03-25T11:00:02.000Z","cwd":"/tmp/demo-project","gitBranch":"feature/layout","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"I will tighten the layout."},{"type":"tool_use","name":"Read","input":{"file_path":"/tmp/demo-project/src/app.ts"}},{"type":"tool_result","content":"ok"},{"type":"file","file":{"path":"/tmp/demo-project/src/app.ts"}},{"type":"custom_block","payload":{"x":1}}]}}"#,
        "\n",
        r#"{"type":"user","uuid":"u2","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-03-25T11:00:03.000Z","message":{"role":"system","content":[{"type":"text","text":"unknown role stays fact"}]}}"#,
        "\n",
        r#"{"type":"queue-operation","uuid":"q1","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-03-25T11:00:04.000Z","operation":"enqueue"}"#,
        "\n",
    );
    fs::write(&path, body).expect("write session");
    fs::write(
        root.join("history.jsonl"),
        r#"{"display":"Claude mixed content fixture","sessionId":"123e4567-e89b-12d3-a456-426614174000"}
"#,
    )
    .expect("history");
    fs::write(root.join("settings.json"), "{}\n").expect("settings");
    path
}

/**
 * Library detect reports configured Claude root and stays generic when missing.
 */
#[test]
fn library_detects_claude_root_and_redacts_missing_root() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let claude = temp.path().join("claude-home");
    fs::create_dir_all(claude.join("projects")).expect("projects");
    write_mixed_session(&claude);

    let library = Library::open(&home).expect("open");
    let results = library
        .detect_sources(&[
            SourceDetectRequest {
                kind: "claude_code".into(),
                configured_root: Some(claude.display().to_string()),
            },
            SourceDetectRequest {
                kind: "claude_code".into(),
                configured_root: None,
            },
            SourceDetectRequest {
                kind: "droid".into(),
                configured_root: None,
            },
        ])
        .expect("detect");

    assert!(
        results[0].status == "ok" || results[0].status == "unavailable",
        "configured Claude root should detect as ok or executable-unavailable: {:?}",
        results[0]
    );
    assert_eq!(results[0].display_name.as_deref(), Some("Claude Code"));
    assert!(results[0]
        .effective_data_root
        .as_deref()
        .expect("root")
        .contains("claude-home"));
    if results[0].status == "unavailable" {
        assert_eq!(
            results[0].error_class.as_deref(),
            Some("executable_not_found")
        );
        let message = results[0].error_message.as_deref().unwrap_or("");
        assert!(!message.to_ascii_lowercase().contains("claude"));
        assert!(!message.contains('/'));
    } else {
        assert!(results[0].error_message.is_none());
    }

    // Default preference is disabled with no override root.
    assert_eq!(results[1].status, "disabled");
    assert!(results[1].error_class.is_none());

    assert_eq!(results[2].status, "disabled");
    assert!(results[2].error_class.is_none());

    let mut library = Library::open(&home).expect("reopen");
    library
        .set_source_preference("claude_code", true, None)
        .expect("enable without root");
    let missing = library
        .detect_sources(&[SourceDetectRequest {
            kind: "claude_code".into(),
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
        !missing_message.to_ascii_lowercase().contains("claude"),
        "diagnostics must not name the provider: {missing_message}"
    );
}

/**
 * Production Sync path ingests Claude history with generic progress and exact replay.
 */
#[test]
fn library_sync_claude_mixed_blocks_and_replays_after_source_removal() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let claude = temp.path().join("claude-home");
    fs::create_dir_all(&claude).expect("claude root");
    let session_path = write_mixed_session(&claude);
    let original_bytes = fs::read(&session_path).expect("original bytes");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("claude_code", true, Some(claude.as_path()))
        .expect("prefer claude");

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
    assert_eq!(result.run.sources[0].source_kind, "claude_code");
    assert_eq!(result.run.sources[0].status, "completed");
    assert!(result.run.sources[0].error_class.is_none());
    assert!(result.run.sources[0].error_message.is_none());
    assert_eq!(result.session_identities.len(), 1);
    assert_eq!(result.session_identities[0].source_kind, "claude_code");
    assert_eq!(
        result.session_identities[0].external_session_id,
        "123e4567-e89b-12d3-a456-426614174000"
    );

    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::SourceStarted { source_kind, .. } if source_kind == "claude_code"
    )));
    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::CandidateStarted { source_kind, .. } if source_kind == "claude_code"
    )));
    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::CandidateFinished { source_kind, outcome, .. }
            if source_kind == "claude_code" && outcome == "accepted"
    )));
    for event in &progress {
        if let SyncProgress::CandidateStarted { candidate_id, .. }
        | SyncProgress::CandidateFinished { candidate_id, .. } = event
        {
            assert!(
                candidate_id.starts_with("claude://"),
                "candidate identity should be a logical source path: {candidate_id}"
            );
            assert!(
                !candidate_id.starts_with('/'),
                "candidate identity must not leak absolute paths: {candidate_id}"
            );
        }
    }

    let detail = library
        .session_slice(
            "claude_code",
            "123e4567-e89b-12d3-a456-426614174000",
            20,
            20,
        )
        .expect("session query")
        .expect("session present");
    assert_eq!(detail.summary.source_kind, "claude_code");
    assert_eq!(
        detail.summary.title.as_deref(),
        Some("Claude mixed content fixture")
    );
    assert_eq!(detail.project_path.as_deref(), Some("/tmp/demo-project"));
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
        .any(|artifact| artifact.artifact_type == "tool_call"
            || artifact.artifact_type == "tool_use"));
    assert!(detail
        .artifacts
        .iter()
        .any(|artifact| artifact.artifact_type == "tool_result"));
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
    fs::remove_dir_all(&claude).expect("delete claude root");
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
 * Claude Sync with a configured root that has no sessions completes without provider leakage.
 */
#[test]
fn library_sync_empty_claude_root_completes_generically() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let claude = temp.path().join("claude-home");
    fs::create_dir_all(claude.join("projects")).expect("projects");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("claude_code", true, Some(claude.as_path()))
        .expect("prefer");
    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.status, "completed");
    assert_eq!(result.run.accepted_captures, 0);
    assert_eq!(result.run.sources[0].source_kind, "claude_code");
    assert!(result.run.sources[0].error_message.is_none());
}

#[test]
fn library_sync_unreadable_projects_root_is_typed_and_redacted() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let claude = temp.path().join("claude-home");
    fs::create_dir_all(&claude).expect("claude root");
    fs::write(claude.join("projects"), "not-a-directory").expect("projects file");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("claude_code", true, Some(claude.as_path()))
        .expect("prefer");
    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.status, "failed");
    assert_eq!(result.run.sources.len(), 1);
    assert_eq!(result.run.sources[0].source_kind, "claude_code");
    assert_eq!(
        result.run.sources[0].error_class.as_deref(),
        Some("source_adapter")
    );
    assert_eq!(
        result.run.sources[0].error_message.as_deref(),
        Some("source sync failed")
    );
    assert!(!result.run.sources[0]
        .error_message
        .as_deref()
        .unwrap_or_default()
        .contains("projects"));
}
