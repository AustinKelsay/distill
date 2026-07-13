//! Tauri host multi-Source product-loop seam for issue #44.
//!
//! Hermetic coverage for Codex, Claude Code, OpenCode, and Droid through public
//! host/Library seams only. No repository or SQL shortcuts.

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use distill_desktop_lib::{
    execute_add_session_tag, execute_list_activity, execute_list_operations, execute_list_sessions,
    execute_preview_export, execute_publish_export, execute_session_detail,
    execute_set_source_preference, execute_sync_start, execute_toggle_session_label,
    validate_export_request, validate_home_request, validate_session_curation_request,
    validate_source_preference_request, validate_sync_start_request,
};
use distill_library::{
    ActivityListRequest, ExportStatus, OperationsRequest, SessionCurationRequest,
    SessionDetailRequest, SessionListRequest, WorkflowLane,
};
use tempfile::TempDir;

/// Serialize PATH / env-bound OpenCode harness mutations across tests in this file.
static OPENCODE_HARNESS_LOCK: Mutex<()> = Mutex::new(());

/**
 * Write a minimal Fixture root for mixed-failure isolation tests.
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

/**
 * Write a minimal Codex live session under a synthetic Codex home.
 */
fn write_host_codex_root(root: &Path) -> (String, String) {
    let session_id = "abc12345-1111-2222-3333-abcdefabcdef";
    let relative =
        "sessions/2026/07/12/rollout-2026-07-12T12-00-00-abc12345-1111-2222-3333-abcdefabcdef.jsonl";
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("codex parent");
    fs::write(
        &path,
        concat!(
            r#"{"timestamp":"2026-07-12T12:00:00.000Z","type":"session_meta","payload":{"id":"abc12345-1111-2222-3333-abcdefabcdef","timestamp":"2026-07-12T12:00:00.000Z","cwd":"/tmp/host-codex"}}"#,
            "\n",
            r#"{"timestamp":"2026-07-12T12:00:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello host codex"}]}}"#,
            "\n",
            r#"{"timestamp":"2026-07-12T12:00:02.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"codex host reply"}]}}"#,
            "\n",
        ),
    )
    .expect("codex session");
    fs::write(
        root.join("session_index.jsonl"),
        r#"{"id":"abc12345-1111-2222-3333-abcdefabcdef","thread_name":"Host Codex","updated_at":"2026-07-12T12:01:00.000Z"}
"#,
    )
    .expect("codex index");
    (session_id.to_string(), "hello host codex".to_string())
}

/**
 * Write a minimal Claude Code project session under a synthetic Claude home.
 */
fn write_host_claude_root(root: &Path) -> (String, String) {
    let session_id = "123e4567-e89b-12d3-a456-426614174000";
    let path = root
        .join("projects")
        .join("host-demo")
        .join(format!("{session_id}.jsonl"));
    fs::create_dir_all(path.parent().expect("parent")).expect("claude parent");
    fs::write(
        &path,
        concat!(
            r#"{"type":"user","uuid":"u1","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-07-12T12:10:00.000Z","cwd":"/tmp/host-claude","message":{"role":"user","content":[{"type":"text","text":"hello host claude"}]}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a1","parentUuid":"u1","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-07-12T12:10:01.000Z","cwd":"/tmp/host-claude","message":{"role":"assistant","content":[{"type":"text","text":"claude host reply"}]}}"#,
            "\n",
        ),
    )
    .expect("claude session");
    fs::write(
        root.join("history.jsonl"),
        r#"{"display":"Host Claude","sessionId":"123e4567-e89b-12d3-a456-426614174000"}
"#,
    )
    .expect("claude history");
    (session_id.to_string(), "hello host claude".to_string())
}

/**
 * Write a minimal Droid session under a synthetic Factory sessions root.
 */
fn write_host_droid_root(root: &Path) -> (String, String) {
    let session_id = "123e4567-e89b-12d3-a456-426614174000";
    let path = root.join("ws-host").join(format!("{session_id}.jsonl"));
    fs::create_dir_all(path.parent().expect("parent")).expect("droid parent");
    fs::write(
        &path,
        concat!(
            r#"{"type":"session_start","id":"123e4567-e89b-12d3-a456-426614174000","title":"Host Droid","owner":"host","cwd":"/tmp/host-droid"}"#,
            "\n",
            r#"{"type":"message","id":"u1","timestamp":"2026-07-12T12:20:00.000Z","message":{"role":"user","content":[{"type":"text","text":"hello host droid"}]}}"#,
            "\n",
            r#"{"type":"message","id":"a1","timestamp":"2026-07-12T12:20:01.000Z","message":{"role":"assistant","content":[{"type":"text","text":"droid host reply"}]}}"#,
            "\n",
        ),
    )
    .expect("droid session");
    (session_id.to_string(), "hello host droid".to_string())
}

/**
 * Install a hermetic fake `opencode` under `{root}/bin` with one virtual session.
 */
fn install_host_fake_opencode(root: &Path) -> (String, String) {
    let bin_dir = root.join("bin");
    let export_dir = root.join("exports");
    fs::create_dir_all(&bin_dir).expect("opencode bin");
    fs::create_dir_all(&export_dir).expect("opencode exports");
    fs::write(
        root.join("sessions.json"),
        r#"[{"id":"ses_host","title":"Host OpenCode","directory":"/tmp/host-opencode","version":"1.0.0","time_created":1774543194067,"time_updated":1774543475213,"time_archived":null}]
"#,
    )
    .expect("sessions json");
    let export_body = concat!(
        "Exporting session: ses_host\n",
        r#"{"info":{"id":"ses_host","slug":"host-wizard","projectID":"global","directory":"/tmp/host-opencode","title":"Host OpenCode","version":"1.0.0","time":{"created":1774543194067,"updated":1774543475213}},"messages":[{"info":{"id":"msg_user","role":"user","time":{"created":1774543194080}},"parts":[{"id":"part_user","type":"text","text":"hello host opencode"}]},{"info":{"id":"msg_assistant","role":"assistant","parentID":"msg_user","time":{"created":1774543194090}},"parts":[{"id":"part_text","type":"text","text":"opencode host reply"}]}]}"#,
        "\n",
    );
    fs::write(export_dir.join("ses_host.json"), export_body).expect("export body");
    let script = bin_dir.join("opencode");
    let script_body = format!(
        r#"#!/bin/sh
set -eu
ROOT="{root}"
case "${{1:-}}" in
  db)
    if [ "${{2:-}}" = "path" ]; then
      printf '%s\n' "$ROOT/opencode.db"
      exit 0
    fi
    cat "$ROOT/sessions.json"
    exit 0
    ;;
  export)
    cat "$ROOT/exports/${{2:-}}.json"
    exit 0
    ;;
esac
printf 'unsupported fake opencode command\n' >&2
exit 1
"#,
        root = root.display(),
    );
    fs::write(&script, script_body).expect("opencode script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).expect("chmod");
    }
    ("ses_host".to_string(), "hello host opencode".to_string())
}

/**
 * Assert diagnostics strings never contain forbidden source content or paths.
 */
fn assert_host_text_redacted(rendered: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            !rendered.contains(needle),
            "host diagnostics leaked `{needle}`: {rendered}"
        );
    }
}

/**
 * Configure one Source, Sync it, then exercise search/detail/curation/export/ops.
 */
fn run_provider_host_journey(kind: &str, home: &Path, root: &Path, session_id: &str, query: &str) {
    let home_s = home.to_str().expect("home utf8");
    let root_s = root.to_str().expect("root utf8");

    let pref = validate_source_preference_request(home_s, kind, true, Some(root_s)).expect("pref");
    let set = execute_set_source_preference(&pref).expect("set preference");
    assert_eq!(set.kind, kind);
    assert!(set.enabled);

    let sync_request =
        validate_sync_start_request(home_s, vec![kind.to_string()]).expect("sync request");
    let result = execute_sync_start(&sync_request, |_| {}).expect("sync");
    assert_eq!(result.run.status, "completed");
    assert_eq!(result.run.accepted_captures, 1);
    assert_eq!(result.run.sources[0].source_kind, kind);
    assert_eq!(result.run.sources[0].status, "completed");
    assert_eq!(result.session_identities[0].source_kind, kind);
    assert_eq!(result.session_identities[0].external_session_id, session_id);
    for detail in &result.run.warning_details {
        assert_host_text_redacted(detail, &[root_s, query]);
    }

    let home_request = validate_home_request(home_s).expect("home");
    let page = execute_list_sessions(
        &home_request,
        SessionListRequest {
            query: Some(query.into()),
            lane: WorkflowLane::All,
            limit: 5,
            cursor: None,
        },
    )
    .expect("sessions");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].source_kind, kind);
    assert_eq!(page.items[0].external_session_id, session_id);

    let detail = execute_session_detail(
        &home_request,
        SessionDetailRequest {
            source_kind: kind.into(),
            external_session_id: session_id.into(),
            message_limit: 10,
            artifact_limit: 10,
            message_cursor: None,
            artifact_cursor: None,
        },
    )
    .expect("detail")
    .expect("session present");
    assert_eq!(detail.summary.source_kind, kind);
    assert_eq!(detail.summary.external_session_id, session_id);
    assert!(
        detail
            .messages
            .iter()
            .any(|message| message.text.contains(query)),
        "detail missing query text {query}"
    );

    let (home_request, tag_request) = validate_session_curation_request(
        home_s,
        SessionCurationRequest {
            source_kind: kind.into(),
            external_session_id: session_id.into(),
            name: "host-provider".into(),
        },
    )
    .expect("tag request");
    let tagged = execute_add_session_tag(&home_request, tag_request).expect("tag");
    assert!(tagged.changed);
    assert_eq!(tagged.tags[0].name, "host-provider");

    let (_home_request, label_request) = validate_session_curation_request(
        home_s,
        SessionCurationRequest {
            source_kind: kind.into(),
            external_session_id: session_id.into(),
            name: "train".into(),
        },
    )
    .expect("label request");
    let labeled = execute_toggle_session_label(&home_request, label_request).expect("label");
    assert!(labeled.changed);
    assert_eq!(
        labeled.workflow_state,
        distill_library::WorkflowState::TrainReady
    );

    let preview_request = validate_export_request(home_s, "train").expect("preview request");
    let preview = execute_preview_export(&preview_request).expect("preview");
    assert_eq!(preview.dataset.as_str(), "train");
    assert_eq!(preview.format_id, "distill-session-jsonl-v1");
    assert_eq!(preview.eligible[0].external_session_id, session_id);

    let publish_request = validate_export_request(home_s, "train").expect("publish request");
    let published = execute_publish_export(&publish_request, |_| {}).expect("publish");
    assert_eq!(published.status, ExportStatus::Published);
    assert_eq!(published.record_count, 1);
    assert_eq!(published.format_id, "distill-session-jsonl-v1");

    let activity = execute_list_activity(
        &home_request,
        ActivityListRequest {
            limit: 20,
            cursor: None,
        },
    )
    .expect("activity");
    assert!(!activity.items.is_empty());
    assert!(activity.items.iter().any(|event| {
        event.event_type == "capture_recorded" || event.event_type == "sync_completed"
    }));
    let activity_json = serde_json::to_string(&activity).expect("activity json");
    assert_host_text_redacted(&activity_json, &[root_s]);

    let operations = execute_list_operations(
        &home_request,
        OperationsRequest {
            sync_limit: 10,
            export_limit: 10,
            sync_cursor: None,
            export_cursor: None,
        },
    )
    .expect("operations");
    assert!(!operations.sync_runs.is_empty());
    assert!(!operations.exports.is_empty());
    let operations_json = serde_json::to_string(&operations).expect("operations json");
    assert_host_text_redacted(&operations_json, &[root_s]);
}

/**
 * Prove Distill-owned Session Projection survives source-root deletion via host only.
 */
fn assert_host_projection_survives_source_removal(
    kind: &str,
    home: &Path,
    root: &Path,
    session_id: &str,
    query: &str,
) {
    fs::remove_dir_all(root).expect("remove provider root");
    assert!(!root.exists(), "provider root should be gone");

    let home_request = validate_home_request(home.to_str().expect("home utf8")).expect("home");
    let page = execute_list_sessions(
        &home_request,
        SessionListRequest {
            query: Some(query.into()),
            lane: WorkflowLane::All,
            limit: 5,
            cursor: None,
        },
    )
    .expect("sessions after removal");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].external_session_id, session_id);

    let detail = execute_session_detail(
        &home_request,
        SessionDetailRequest {
            source_kind: kind.into(),
            external_session_id: session_id.into(),
            message_limit: 10,
            artifact_limit: 10,
            message_cursor: None,
            artifact_cursor: None,
        },
    )
    .expect("detail after removal")
    .expect("session present");
    assert!(detail
        .messages
        .iter()
        .any(|message| message.text.contains(query)));
}

#[test]
fn host_codex_provider_journey_and_projection_survival() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let root = temp.path().join("codex-home");
    fs::create_dir_all(&root).expect("codex root");
    let (session_id, query) = write_host_codex_root(&root);
    run_provider_host_journey("codex", &home, &root, &session_id, &query);
    assert_host_projection_survives_source_removal("codex", &home, &root, &session_id, &query);
}

#[test]
fn host_claude_code_provider_journey() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let root = temp.path().join("claude-home");
    fs::create_dir_all(&root).expect("claude root");
    let (session_id, query) = write_host_claude_root(&root);
    run_provider_host_journey("claude_code", &home, &root, &session_id, &query);
}

#[test]
fn host_opencode_provider_journey_and_projection_survival() {
    let _guard = OPENCODE_HARNESS_LOCK.lock().expect("lock");
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let root = temp.path().join("opencode-home");
    fs::create_dir_all(&root).expect("opencode root");
    let (session_id, query) = install_host_fake_opencode(&root);
    run_provider_host_journey("opencode", &home, &root, &session_id, &query);
    assert_host_projection_survives_source_removal("opencode", &home, &root, &session_id, &query);
}

#[test]
fn host_droid_provider_journey() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let root = temp.path().join("factory-sessions");
    fs::create_dir_all(&root).expect("droid root");
    let (session_id, query) = write_host_droid_root(&root);
    run_provider_host_journey("droid", &home, &root, &session_id, &query);
}

#[test]
fn host_provider_failure_isolation_redacts_diagnostics() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    let secret = "secret-token-host-42";
    let missing = temp.path().join(format!("{secret}-missing-root"));
    fs::create_dir_all(&fixture).expect("fixture");
    write_basic_fixture(&fixture);

    let home_s = home.to_str().expect("home utf8");
    let fixture_s = fixture.to_str().expect("fixture utf8");
    let missing_s = missing.to_str().expect("missing utf8");

    let fixture_pref = validate_source_preference_request(home_s, "fixture", true, Some(fixture_s))
        .expect("fixture pref");
    execute_set_source_preference(&fixture_pref).expect("set fixture");

    let rejected = validate_source_preference_request(home_s, "codex", true, Some(missing_s))
        .expect("host path validation accepts the opaque path before Library canonicalization");
    let err = execute_set_source_preference(&rejected)
        .expect_err("Library must reject a missing configured provider root");
    let rendered = format!("{}:{}", err.code, err.message);
    assert_eq!(err.code, "invalid_configured_root");
    assert_host_text_redacted(&rendered, &[secret, missing_s]);

    let codex_pref =
        validate_source_preference_request(home_s, "codex", true, None).expect("codex pref");
    execute_set_source_preference(&codex_pref).expect("set codex");

    let sync_request = validate_sync_start_request(home_s, vec!["fixture".into(), "codex".into()])
        .expect("sync request");
    let sync = execute_sync_start(&sync_request, |_| {}).expect("sync");
    assert_eq!(sync.run.status, "warning");
    assert!(sync.run.accepted_captures >= 1);
    assert!(!sync.run.warning_details.is_empty());
    assert!(sync
        .run
        .sources
        .iter()
        .any(|source| source.source_kind == "fixture" && source.status == "completed"));
    assert!(sync
        .run
        .sources
        .iter()
        .any(|source| source.source_kind == "codex" && source.status == "failed"));

    let sync_json = serde_json::to_string(&sync).expect("sync json");
    assert_host_text_redacted(&sync_json, &[secret, missing_s, "Hello from host fixture"]);
    for detail in &sync.run.warning_details {
        assert_host_text_redacted(detail, &[secret, missing_s, fixture_s]);
    }

    let home_request = validate_home_request(home_s).expect("home");
    let activity = execute_list_activity(
        &home_request,
        ActivityListRequest {
            limit: 20,
            cursor: None,
        },
    )
    .expect("activity");
    let activity_json = serde_json::to_string(&activity).expect("activity json");
    assert_host_text_redacted(&activity_json, &[secret, missing_s]);
}
