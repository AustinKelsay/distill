//! Codex SourceAdapter Library contracts for issue #26.

use std::fs;
use std::path::{Path, PathBuf};

use distill_library::{Library, SourceDetectRequest, SyncProgress, SyncRequest};
use tempfile::TempDir;

/**
 * Write a Codex home with live dialogue covering tool/reasoning/unknown-role rows.
 */
fn write_live_session(root: &Path) -> PathBuf {
    let relative =
        "sessions/2026/03/25/rollout-2026-03-25T10-00-00-abc12345-1111-2222-3333-abcdefabcdef.jsonl";
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    let body = concat!(
        r#"{"timestamp":"2026-03-25T10:00:00.000Z","type":"session_meta","payload":{"id":"abc12345-1111-2222-3333-abcdefabcdef","timestamp":"2026-03-25T10:00:00.000Z","cwd":"/tmp/demo","cli_version":"1.2.3","model_provider":"openai"}}"#,
        "\n",
        r#"{"timestamp":"2026-03-25T10:00:00.500Z","type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
        "\n",
        r#"{"timestamp":"2026-03-25T10:00:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n  <cwd>/tmp/demo</cwd>"}]}}"#,
        "\n",
        r#"{"timestamp":"2026-03-25T10:00:02.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello codex"}]}}"#,
        "\n",
        r#"{"timestamp":"2026-03-25T10:00:03.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"I will update the code."}]}}"#,
        "\n",
        r#"{"timestamp":"2026-03-25T10:00:04.000Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"cmd\":\"ls\"}"}}"#,
        "\n",
        r#"{"timestamp":"2026-03-25T10:00:05.000Z","type":"response_item","payload":{"type":"reasoning","summary":[{"text":"consider options"}]}}"#,
        "\n",
        r#"{"timestamp":"2026-03-25T10:00:06.000Z","type":"response_item","payload":{"type":"message","role":"system","content":[{"type":"output_text","text":"unknown role stays fact"}]}}"#,
        "\n",
    );
    fs::write(&path, body).expect("write live");
    fs::write(
        root.join("session_index.jsonl"),
        r#"{"id":"abc12345-1111-2222-3333-abcdefabcdef","thread_name":"Demo Thread","updated_at":"2026-03-25T11:00:00.000Z"}
"#,
    )
    .expect("index");
    fs::write(root.join("history.jsonl"), "{}\n").expect("history");
    path
}

/**
 * Write an archived duplicate of the live Session Identity with stale text.
 */
fn write_archived_duplicate(root: &Path) {
    let relative =
        "archived_sessions/rollout-2026-03-24T09-55-00-abc12345-1111-2222-3333-abcdefabcdef.jsonl";
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    fs::write(
        &path,
        concat!(
            r#"{"timestamp":"2026-03-24T09:55:00.000Z","type":"session_meta","payload":{"id":"abc12345-1111-2222-3333-abcdefabcdef","timestamp":"2026-03-24T09:55:00.000Z","cwd":"/tmp/archived-demo"}}"#,
            "\n",
            r#"{"timestamp":"2026-03-24T09:56:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"stale archived codex session"}]}}"#,
            "\n",
        ),
    )
    .expect("write archived");
}

/**
 * Library detect reports configured Codex root and stays generic when missing.
 */
#[test]
fn library_detects_codex_root_and_redacts_missing_root() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let codex = temp.path().join("codex-home");
    fs::create_dir_all(codex.join("sessions")).expect("sessions");
    write_live_session(&codex);

    let library = Library::open(&home).expect("open");
    let results = library
        .detect_sources(&[
            SourceDetectRequest {
                kind: "codex".into(),
                configured_root: Some(codex.display().to_string()),
            },
            SourceDetectRequest {
                kind: "codex".into(),
                configured_root: None,
            },
            SourceDetectRequest {
                kind: "opencode".into(),
                configured_root: None,
            },
        ])
        .expect("detect");

    assert_eq!(results[0].status, "ok");
    assert_eq!(results[0].display_name.as_deref(), Some("Codex"));
    assert!(results[0]
        .effective_data_root
        .as_deref()
        .expect("root")
        .contains("codex-home"));
    assert!(results[0].error_message.is_none());

    // Default preference is disabled with no override root.
    assert_eq!(results[1].status, "disabled");
    assert!(results[1].error_class.is_none());

    assert_eq!(results[2].status, "unavailable");
    assert_eq!(
        results[2].error_class.as_deref(),
        Some("adapter_not_registered")
    );

    let mut library = Library::open(&home).expect("reopen");
    library
        .set_source_preference("codex", true, None)
        .expect("enable without root");
    let missing = library
        .detect_sources(&[SourceDetectRequest {
            kind: "codex".into(),
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
        !missing_message.to_ascii_lowercase().contains("codex"),
        "diagnostics must not name the provider: {missing_message}"
    );
}

/**
 * Production Sync path ingests live-over-archive Codex history with generic progress.
 */
#[test]
fn library_sync_codex_live_over_archive_and_replays_after_source_removal() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let codex = temp.path().join("codex-home");
    fs::create_dir_all(&codex).expect("codex root");
    let live_path = write_live_session(&codex);
    write_archived_duplicate(&codex);
    let original_bytes = fs::read(&live_path).expect("original bytes");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("codex", true, Some(codex.as_path()))
        .expect("prefer codex");

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
    assert_eq!(result.run.sources[0].source_kind, "codex");
    assert_eq!(result.run.sources[0].status, "completed");
    assert!(result.run.sources[0].error_class.is_none());
    assert!(result.run.sources[0].error_message.is_none());
    assert_eq!(result.session_identities.len(), 1);
    assert_eq!(result.session_identities[0].source_kind, "codex");
    assert_eq!(
        result.session_identities[0].external_session_id,
        "abc12345-1111-2222-3333-abcdefabcdef"
    );

    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::SourceStarted { source_kind, .. } if source_kind == "codex"
    )));
    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::CandidateStarted { source_kind, .. } if source_kind == "codex"
    )));
    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::CandidateFinished { source_kind, outcome, .. }
            if source_kind == "codex" && outcome == "accepted"
    )));
    for event in &progress {
        if let SyncProgress::CandidateStarted { candidate_id, .. }
        | SyncProgress::CandidateFinished { candidate_id, .. } = event
        {
            assert!(
                candidate_id.starts_with("codex://"),
                "candidate identity should be a logical source path: {candidate_id}"
            );
            assert!(
                !candidate_id.starts_with('/'),
                "candidate identity must not leak absolute paths: {candidate_id}"
            );
        }
    }

    let detail = library
        .session_slice("codex", "abc12345-1111-2222-3333-abcdefabcdef", 20, 20)
        .expect("session query")
        .expect("session present");
    assert_eq!(detail.summary.source_kind, "codex");
    assert_eq!(detail.summary.title.as_deref(), Some("Demo Thread"));
    assert_eq!(detail.project_path.as_deref(), Some("/tmp/demo"));
    assert_eq!(detail.messages.len(), 2);
    assert_eq!(detail.messages[0].text, "hello codex");
    assert_eq!(detail.messages[1].text, "I will update the code.");
    assert!(
        detail
            .messages
            .iter()
            .all(|message| message.text != "stale archived codex session"),
        "live capture must win over archived duplicate"
    );
    assert!(detail.artifacts.len() >= 2);
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

    fs::remove_file(&live_path).expect("delete live source");
    fs::remove_dir_all(&codex).expect("delete codex root");
    let replayed_after = library
        .replay_capture(capture_id)
        .expect("replay after source removal");
    assert_eq!(replayed_after, original_bytes);

    let hits = library.search("hello codex", 10).expect("search");
    assert!(hits.iter().any(|hit| hit.text.contains("hello codex")));
}

/**
 * Codex Sync with a configured root that has no sessions completes without provider leakage.
 */
#[test]
fn library_sync_empty_codex_root_completes_generically() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let codex = temp.path().join("codex-home");
    fs::create_dir_all(codex.join("sessions")).expect("sessions");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("codex", true, Some(codex.as_path()))
        .expect("prefer");
    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.status, "completed");
    assert_eq!(result.run.accepted_captures, 0);
    assert_eq!(result.run.sources[0].source_kind, "codex");
    assert!(result.run.sources[0].error_message.is_none());
}
