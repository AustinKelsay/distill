//! Pi SourceAdapter Library contracts (LPI-001..LPI-004).
//!
//! Pi is a file-backed Source: detection requires the `pi` executable plus a
//! configured sessions root; discovery/snapshot/parse/sync never invoke a provider
//! subprocess. Exact JSONL bytes remain the snapshot truth.

use std::fs;
use std::path::{Path, PathBuf};

use distill_library::{Library, SourceDetectRequest, SyncProgress, SyncRequest};
use tempfile::TempDir;

/**
 * Write a Pi session with a `session` header, user/assistant dialogue, a tool_use
 * structured block, a compaction, and a label.
 */
fn write_mixed_session(root: &Path) -> PathBuf {
    let path = root
        .join("--home-user-project--")
        .join("20260601_100000_pi-ses-001.jsonl");
    fs::create_dir_all(root.join("--home-user-project--")).expect("parent");
    let body = concat!(
        r#"{"type":"session","version":3,"id":"pi-ses-001","timestamp":"2026-06-01T10:00:00.000Z","cwd":"/tmp/demo"}"#,
        "\n",
        r#"{"type":"message","id":"msg_1","parentId":null,"timestamp":"2026-06-01T10:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"hello pi"}]}}"#,
        "\n",
        r#"{"type":"message","id":"msg_2","parentId":"msg_1","timestamp":"2026-06-01T10:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"hello! how can i help?"},{"type":"tool_use","name":"Read","input":{"file_path":"/tmp/demo/src/main.ts"}}]}}"#,
        "\n",
        r#"{"type":"message","id":"msg_3","parentId":"msg_2","timestamp":"2026-06-01T10:00:03.000Z","message":{"role":"user","content":[{"type":"text","text":"fix the import"}]}}"#,
        "\n",
        r#"{"type":"compaction","id":"cmp_1","parentId":"msg_3","timestamp":"2026-06-01T10:05:00.000Z","tokensBefore":500,"tokensAfter":200}"#,
        "\n",
        r#"{"type":"label","id":"lbl_1","parentId":"msg_3","timestamp":"2026-06-01T10:06:00.000Z","label":"important","targetId":"msg_3"}"#,
        "\n",
    );
    fs::write(&path, body).expect("write session");
    path
}

/**
 * LPI-001 — Pi detection reports configured roots, extracts session-header ids, and
 * keeps absent roots typed. The configured-root test requires the `pi` executable on
 * PATH (CI provides a hermetic stub); absent-root classification is executable-free.
 */
#[test]
fn library_detects_pi_root_and_absent_root() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let sessions = temp.path().join("pi-sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    write_mixed_session(&sessions);

    let missing = temp.path().join("missing pi root with secret");

    let library = Library::open(&home).expect("open");

    let results = library
        .detect_sources(&[SourceDetectRequest {
            kind: "pi".into(),
            configured_root: Some(sessions.display().to_string()),
        }])
        .expect("detect");
    assert_eq!(results[0].status, "ok");
    assert_eq!(results[0].display_name.as_deref(), Some("Pi"));
    assert!(results[0]
        .effective_data_root
        .as_deref()
        .expect("root")
        .contains("pi-sessions"));
    assert!(results[0].error_message.is_none());

    let absent = library
        .detect_sources(&[SourceDetectRequest {
            kind: "pi".into(),
            configured_root: Some(missing.display().to_string()),
        }])
        .expect("detect absent");
    assert_eq!(absent[0].status, "unhealthy");
    assert!(absent[0].error_class.is_some());
    let message = absent[0].error_message.as_deref().unwrap_or_default();
    assert!(!message.contains('/'));
    assert!(!message.contains("secret"));
    assert!(!message.to_ascii_lowercase().contains("pi"));
}

/**
 * LPI-002 — Pi snapshot preserves exact JSONL bytes; replay survives session and root
 * deletion using Distill-owned content only.
 */
#[test]
fn library_sync_pi_snapshot_and_replay_after_source_removal() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let sessions = temp.path().join("pi-sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    let session_path = write_mixed_session(&sessions);
    let original_bytes = fs::read(&session_path).expect("original bytes");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("pi", true, Some(sessions.as_path()))
        .expect("prefer pi");

    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");

    assert_eq!(result.run.status, "completed");
    assert_eq!(result.run.accepted_captures, 1);
    assert_eq!(result.run.successful_attempts, 1);
    assert_eq!(result.run.sources.len(), 1);
    assert_eq!(result.run.sources[0].source_kind, "pi");
    assert_eq!(result.run.sources[0].status, "completed");
    assert_eq!(
        result.session_identities[0].external_session_id,
        "pi-ses-001"
    );

    let activity = library.recent_activity(50).expect("activity");
    let capture_id = activity
        .iter()
        .find(|event| event.event_type == "capture_recorded")
        .and_then(|event| event.capture_id)
        .expect("capture");
    let replayed = library.replay_capture(capture_id).expect("replay");
    assert_eq!(replayed, original_bytes);

    fs::remove_file(&session_path).expect("delete session source");
    fs::remove_dir_all(&sessions).expect("delete sessions root");
    let replayed_after = library
        .replay_capture(capture_id)
        .expect("replay after source removal");
    assert_eq!(replayed_after, original_bytes);

    let hits = library.search("fix the import", 10).expect("search");
    assert!(hits.iter().any(|hit| hit.text.contains("fix the import")));
}

/**
 * LPI-003 — Pi header metadata, dialogue, compaction/label facts, and structured
 * blocks map to canonical messages/artifacts without leaking tool payloads into
 * transcript text.
 */
#[test]
fn library_sync_pi_mixed_blocks_and_metadata() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let sessions = temp.path().join("pi-sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    write_mixed_session(&sessions);

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("pi", true, Some(sessions.as_path()))
        .expect("prefer pi");

    let mut progress = Vec::new();
    let result = library
        .start_sync(SyncRequest::default(), |event| progress.push(event))
        .expect("sync");

    assert_eq!(result.run.status, "completed");
    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::SourceStarted { source_kind, .. } if source_kind == "pi"
    )));
    for event in &progress {
        if let SyncProgress::CandidateStarted { candidate_id, .. }
        | SyncProgress::CandidateFinished { candidate_id, .. } = event
        {
            assert!(
                candidate_id.starts_with("pi://session/"),
                "candidate identity should be a logical source path: {candidate_id}"
            );
            assert!(
                !candidate_id.starts_with('/'),
                "candidate identity must not leak absolute paths: {candidate_id}"
            );
            assert!(
                !candidate_id.contains("pi-sessions"),
                "candidate identity must not leak filesystem roots: {candidate_id}"
            );
        }
    }

    let detail = library
        .session_slice("pi", "pi-ses-001", 20, 20)
        .expect("session query")
        .expect("session present");
    assert_eq!(detail.summary.source_kind, "pi");
    assert_eq!(detail.summary.title.as_deref(), Some("hello pi"));
    assert_eq!(detail.project_path.as_deref(), Some("/tmp/demo"));
    assert_eq!(detail.messages.len(), 3);
    assert_eq!(detail.messages[0].role, "user");
    assert_eq!(detail.messages[0].text, "hello pi");
    assert_eq!(detail.messages[1].role, "assistant");
    assert_eq!(detail.messages[1].text, "hello! how can i help?");
    assert_eq!(detail.messages[2].role, "user");
    assert_eq!(detail.messages[2].text, "fix the import");
    assert!(
        detail
            .messages
            .iter()
            .all(|message| !message.text.contains("Read") && !message.text.contains("/tmp/demo")),
        "structured tool payload must not become transcript text"
    );
    assert!(detail
        .artifacts
        .iter()
        .any(|artifact| artifact.artifact_type == "tool_call"));
    assert!(detail
        .artifacts
        .iter()
        .all(|artifact| artifact.capture_fact_id.is_some()));

    let metadata: serde_json::Value =
        serde_json::from_str(&detail.metadata_json).expect("metadata json");
    assert_eq!(
        metadata
            .get("session_version")
            .and_then(|value| value.as_i64()),
        Some(3)
    );
    assert_eq!(
        metadata
            .pointer("/external_session_id_provenance/kind")
            .and_then(|value| value.as_str()),
        Some("source")
    );
}

/**
 * LPI-004 — malformed JSON and malformed UTF-8 Pi files remain typed and redacted,
 * storing exact bytes so replay still succeeds from Distill-owned content.
 */
#[test]
fn library_sync_pi_malformed_jsonl_and_utf8_are_typed() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let sessions = temp.path().join("pi-sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    let bad_json = sessions.join("bad.jsonl");
    fs::write(&bad_json, "{not-json\n").expect("bad json");
    // A non-UTF-8 capture must be blob-backed (>64KB inline threshold) to be stored.
    let mut invalid_bytes = vec![0xff; 70_000];
    invalid_bytes.push(b'\n');
    let bad_utf8 = sessions.join("bad-utf8.jsonl");
    fs::write(&bad_utf8, &invalid_bytes).expect("bad utf8");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("pi", true, Some(sessions.as_path()))
        .expect("prefer pi");

    let mut progress = Vec::new();
    let result = library
        .start_sync(SyncRequest::default(), |event| progress.push(event))
        .expect("sync");
    assert_eq!(result.run.status, "warning");
    assert_eq!(result.run.status, "warning");
    let activity = library.recent_activity(100).expect("activity");
    eprintln!("DBG activity={:#?}", activity);
    let capture_ids: Vec<i64> = activity
        .iter()
        .filter(|event| event.event_type == "capture_recorded")
        .filter_map(|event| event.capture_id)
        .collect();
    eprintln!("DBG capture_ids={:#?}", capture_ids);
    assert_eq!(result.run.accepted_captures, 2);
    assert_eq!(result.run.failed_attempts, 2);
    assert_eq!(result.run.sources[0].source_kind, "pi");
    assert_eq!(result.run.sources[0].status, "warning");
    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::CandidateFinished { outcome, candidate_id, .. }
            if outcome == "failed"
                && candidate_id.starts_with("pi://session/")
                && !candidate_id.contains("pi-sessions")
    )));

    let activity = library.recent_activity(100).expect("activity");
    let mut captured_bytes: Vec<Vec<u8>> = activity
        .iter()
        .filter(|event| event.event_type == "capture_recorded")
        .filter_map(|event| event.capture_id)
        .map(|capture_id| library.replay_capture(capture_id).expect("replay"))
        .collect();
    assert_eq!(captured_bytes.len(), 2);
    captured_bytes.sort();
    let mut expected = vec![b"{not-json\n".to_vec(), invalid_bytes.clone()];
    expected.sort();
    assert_eq!(captured_bytes, expected);
}

/**
 * LPI-004 — headerless Pi files fall back to the deterministic filename-stem
 * identity and stay stable across a repeated sync.
 */
#[test]
fn library_sync_pi_headerless_uses_deterministic_stem_identity() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("distill-home");
    let sessions = temp.path().join("pi-sessions");
    let path = sessions.join("orphan-session.jsonl");
    fs::create_dir_all(&sessions).expect("sessions");
    fs::write(
        &path,
        concat!(
            r#"{"type":"message","id":"m1","timestamp":"2026-06-01T10:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"orphan session"}]}}"#,
            "\n",
        ),
    )
    .expect("session");

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("pi", true, Some(sessions.as_path()))
        .expect("prefer pi");

    let first = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("first sync");
    assert_eq!(first.run.status, "completed");
    assert_eq!(
        first.session_identities[0].external_session_id,
        "orphan-session"
    );

    let detail = library
        .session_slice("pi", "orphan-session", 10, 10)
        .expect("query")
        .expect("present");
    let metadata: serde_json::Value =
        serde_json::from_str(&detail.metadata_json).expect("metadata json");
    assert_eq!(
        metadata
            .pointer("/external_session_id_provenance/strategy")
            .and_then(|value| value.as_str()),
        Some("filename_stem")
    );

    // A repeat sync is inert: duplicate bytes create no new Capture and keep the identity.
    let second = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("second sync");
    assert_eq!(second.run.status, "completed");
    assert_eq!(second.run.accepted_captures, 0);
    let still = library
        .session_slice("pi", "orphan-session", 10, 10)
        .expect("re-query")
        .expect("still present");
    assert_eq!(still.messages[0].text, "orphan session");
}
