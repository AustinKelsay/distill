//! Multi-Source parser registry and same-Capture renormalization contracts for issue #43.
//!
//! Public-seam TDD over real temporary Distill homes. Replay never rereads Source roots
//! and never reruns an OpenCode subprocess.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use distill_library::{
    Library, LibraryError, SourceDetectRequest, SourceKind, SyncProgress, SyncRequest,
};
use tempfile::TempDir;

/// Serialize PATH / env-bound OpenCode harness mutations across tests in this file.
static OPENCODE_HARNESS_LOCK: Mutex<()> = Mutex::new(());

/**
 * Write a Fixture root with one file-backed Capture Candidate.
 */
fn write_fixture(root: &Path, session_id: &str, relative: &str, body: &str) -> PathBuf {
    let capture_path = root.join(relative);
    if let Some(parent) = capture_path.parent() {
        fs::create_dir_all(parent).expect("capture parent");
    }
    fs::write(&capture_path, body).expect("write capture");
    let id = Path::new(relative)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("capture");
    let manifest = format!(
        r#"{{
  "version": 1,
  "captures": [
    {{
      "id": "{id}",
      "kind": "file",
      "relative_path": "{relative}",
      "external_session_id": "{session_id}",
      "title": "Parser Registry Fixture"
    }}
  ]
}}"#
    );
    fs::write(root.join("distill.fixture.json"), manifest).expect("write manifest");
    capture_path
}

/**
 * Write a Codex home with one live session.
 */
fn write_codex_session(root: &Path) -> PathBuf {
    let relative =
        "sessions/2026/03/25/rollout-2026-03-25T10-00-00-abc12345-1111-2222-3333-abcdefabcdef.jsonl";
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    fs::write(
        &path,
        concat!(
            r#"{"timestamp":"2026-03-25T10:00:00.000Z","type":"session_meta","payload":{"id":"abc12345-1111-2222-3333-abcdefabcdef","timestamp":"2026-03-25T10:00:00.000Z","cwd":"/tmp/demo","cli_version":"1.2.3","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-03-25T10:00:02.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello codex"}]}}"#,
            "\n",
            r#"{"timestamp":"2026-03-25T10:00:03.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"I will update the code."}]}}"#,
            "\n",
        ),
    )
    .expect("write live");
    fs::write(
        root.join("session_index.jsonl"),
        r#"{"id":"abc12345-1111-2222-3333-abcdefabcdef","thread_name":"Demo Thread","updated_at":"2026-03-25T11:00:00.000Z"}
"#,
    )
    .expect("index");
    path
}

/**
 * Write a Claude Code home with one mixed session.
 */
fn write_claude_session(root: &Path) -> PathBuf {
    let relative = "projects/demo-project/123e4567-e89b-12d3-a456-426614174000.jsonl";
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    fs::write(
        &path,
        concat!(
            r#"{"type":"user","uuid":"u1","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-03-25T11:00:00.000Z","cwd":"/tmp/demo-project","message":{"role":"user","content":[{"type":"text","text":"Please review the screenshot and fix the layout."}]}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a1","parentUuid":"u1","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-03-25T11:00:02.000Z","cwd":"/tmp/demo-project","message":{"role":"assistant","content":[{"type":"text","text":"I will tighten the layout."}]}}"#,
            "\n",
        ),
    )
    .expect("write session");
    fs::write(
        root.join("history.jsonl"),
        r#"{"display":"Claude mixed content fixture","sessionId":"123e4567-e89b-12d3-a456-426614174000"}
"#,
    )
    .expect("history");
    path
}

/**
 * Write a Droid sessions root with one mixed session.
 */
fn write_droid_session(root: &Path) -> PathBuf {
    let session_id = "123e4567-e89b-12d3-a456-426614174000";
    let path = root.join("ws-demo").join(format!("{session_id}.jsonl"));
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    fs::write(
        &path,
        concat!(
            r#"{"type":"session_start","id":"123e4567-e89b-12d3-a456-426614174000","title":"Droid mixed content fixture","owner":"plebdev","cwd":"/tmp/droid-demo"}"#,
            "\n",
            r#"{"type":"message","id":"u1","timestamp":"2026-04-12T18:17:28.000Z","message":{"role":"user","content":[{"type":"text","text":"Please review the screenshot and fix the layout."}]}}"#,
            "\n",
            r#"{"type":"message","id":"a1","timestamp":"2026-04-12T18:17:29.000Z","message":{"role":"assistant","content":[{"type":"text","text":"I will tighten the layout."}]}}"#,
            "\n",
        ),
    )
    .expect("write session");
    path
}

/**
 * Mixed OpenCode export covering dialogue text.
 */
fn mixed_export_json() -> String {
    r#"{"info":{"id":"ses_1","slug":"tidy-wizard","projectID":"global","directory":"/tmp/opencode-demo","title":"New session - 2026-03-26T19:15:49.354Z","version":"1.3.3","time":{"created":1774543194067,"updated":1774543475213}},"messages":[{"info":{"id":"msg_user","role":"user","time":{"created":1774543194080}},"parts":[{"id":"part_user_text","type":"text","text":"Do I have a project for VisiBible in GTDspace?"}]},{"info":{"id":"msg_assistant","role":"assistant","parentID":"msg_user","time":{"created":1774543194090}},"parts":[{"id":"part_text","type":"text","text":"Yes. Your GTDSpace includes a project named Visibible."}]}]}"#
        .to_string()
}

/**
 * Install a hermetic fake `opencode` under `{root}/bin`.
 */
fn install_fake_opencode(root: &Path) -> PathBuf {
    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin");
    let export_dir = root.join("exports");
    fs::create_dir_all(&export_dir).expect("exports");
    fs::write(
        root.join("sessions.json"),
        r#"[{"id":"ses_1","title":"New session - 2026-03-26T19:15:49.354Z","directory":"/tmp/opencode-demo","version":"1.3.3","time_created":1774543194067,"time_updated":1774543475213,"time_archived":null,"share_url":"https://opencode.ai/share/ses_1"}]"#,
    )
    .expect("sessions");
    fs::write(
        export_dir.join("ses_1.json"),
        format!("Exporting session: ses_1\n{}", mixed_export_json()),
    )
    .expect("export");
    let script = bin_dir.join("opencode");
    let script_body = format!(
        r#"#!/bin/sh
set -eu
ROOT="{root}"
case "${{1:-}}" in
  db)
    cat "$ROOT/sessions.json"
    exit 0
    ;;
  export)
    cat "$ROOT/exports/${{2:-}}.json"
    exit 0
    ;;
esac
printf 'unsupported\n' >&2
exit 1
"#,
        root = root.display(),
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
 * Load the first Capture id recorded in Activity for assertions.
 */
fn first_capture_id(library: &Library) -> i64 {
    library
        .recent_activity(50)
        .expect("activity")
        .into_iter()
        .filter(|event| event.event_type == "capture_recorded")
        .filter_map(|event| event.capture_id)
        .next()
        .expect("capture id")
}

/**
 * Registry defaults and typed version updates record adapter parser ids.
 */
#[test]
fn registered_parser_versions_are_typed_and_source_specific() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-registry",
        "captures/hello.jsonl",
        concat!(
            r#"{"record_type":"session_meta","title":"Registry"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"hello fixture"}"#,
            "\n",
        ),
    );

    let mut library = Library::open(&home).expect("open");
    let first = library.ingest_fixture(&fixture).expect("ingest");
    let capture_id = first.capture_ids[0];
    let attempts = library.capture_attempts(capture_id).expect("attempts");
    assert_eq!(attempts[0].parser_id, "fixture");
    assert_eq!(attempts[0].parser_version, "1.0.0");

    library
        .set_registered_parser_version(SourceKind::Fixture, "1.1.0")
        .expect("advance fixture");
    library
        .set_registered_parser_version(SourceKind::Codex, "1.2.0")
        .expect("advance codex");
    library
        .set_registered_parser_version(SourceKind::ClaudeCode, "1.3.0")
        .expect("advance claude");
    library
        .set_registered_parser_version(SourceKind::OpenCode, "1.4.0")
        .expect("advance opencode");
    library
        .set_registered_parser_version(SourceKind::Droid, "1.5.0")
        .expect("advance droid");

    let err = library
        .set_registered_parser_version(SourceKind::Codex, "1.2.0")
        .expect_err("same version");
    assert!(matches!(err, LibraryError::InvalidArgument(_)));
    let err = library
        .set_registered_parser_version(SourceKind::Codex, "1.1.0")
        .expect_err("older version");
    assert!(matches!(err, LibraryError::InvalidArgument(_)));
    let err = library
        .set_registered_parser_version(SourceKind::Codex, "not-a-version")
        .expect_err("malformed");
    assert!(matches!(err, LibraryError::InvalidArgument(_)));

    // Compatibility Fixture setter still works and keeps parser id internal.
    library
        .set_registered_fixture_parser_version("1.2.0")
        .expect("compat fixture");
    let summary = library.detect_fixture(&fixture).expect("detect");
    assert_eq!(summary.parser_id, "fixture");
    assert_eq!(summary.parser_version, "1.2.0");
}

/**
 * Fixture renormalize after Source-root removal uses Distill-owned bytes only.
 */
#[test]
fn fixture_renormalize_after_source_removal_preserves_prior_attempts() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-replay",
        "captures/hello.jsonl",
        concat!(
            r#"{"record_type":"session_meta","title":"Replay Fixture"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"needs newer parser"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"baseline assistant"}"#,
            "\n",
        ),
    );

    let mut library = Library::open(&home).expect("open");
    let first = library.ingest_fixture(&fixture).expect("ingest");
    let capture_id = first.capture_ids[0];
    let before = library.capture_attempts(capture_id).expect("v1");
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].outcome, "succeeded");
    let original = before[0].clone();
    let detail_before = library
        .session_slice("fixture", "fixture-replay", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(detail_before.messages.len(), 2);

    fs::remove_dir_all(&fixture).expect("remove fixture root");

    library
        .set_registered_parser_version(SourceKind::Fixture, "2.0.0")
        .expect("register v2");
    let retry = library
        .renormalize_capture(capture_id)
        .expect("renormalize without source root");
    assert_eq!(retry.capture_id, capture_id);
    assert_eq!(retry.outcome, "succeeded");
    assert_eq!(retry.parser_id, "fixture");
    assert_eq!(retry.parser_version, "2.0.0");

    let after = library.capture_attempts(capture_id).expect("v2");
    assert_eq!(after.len(), 2);
    assert_eq!(after[0], original);
    assert_eq!(after[1].parser_version, "2.0.0");
    assert_eq!(after[1].outcome, "succeeded");

    let detail = library
        .session_slice("fixture", "fixture-replay", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(detail.summary.accepted_capture_count, 1);
    assert_eq!(detail.summary.normalization_attempt_count, 2);
    assert_eq!(detail.summary.successful_projection_generation, 2);
    assert_eq!(detail.messages[0].text, "needs newer parser");
}

/**
 * Fixture parse failure on renormalize preserves prior successful Projection state.
 */
#[test]
fn fixture_renormalize_parse_failure_preserves_last_good_projection() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-fail-retry",
        "captures/hello.jsonl",
        concat!(
            r#"{"record_type":"session_meta","title":"Good First"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"good first"}"#,
            "\n",
        ),
    );

    let mut library = Library::open(&home).expect("open");
    let first = library.ingest_fixture(&fixture).expect("good ingest");
    let good_capture = first.capture_ids[0];
    let before = library
        .session_slice("fixture", "fixture-fail-retry", 20, 20)
        .expect("session")
        .expect("present");
    let generation = before.summary.successful_projection_generation;
    let messages = before.messages.clone();

    write_fixture(
        &fixture,
        "fixture-gated",
        "captures/gated.jsonl",
        concat!(
            r#"{"record_type":"session_meta","title":"Gated"}"#,
            "\n",
            r#"{"record_type":"require_parser_min","version":"3.0.0"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"should stay unpublished"}"#,
            "\n",
        ),
    );
    let gated = library.ingest_fixture(&fixture).expect("gated ingest");
    assert_eq!(gated.accepted_captures, 1);
    assert_eq!(gated.failed_attempts, 1);
    let gated_id = gated.capture_ids[0];
    assert_ne!(gated_id, good_capture);
    assert!(library
        .session_slice("fixture", "fixture-gated", 20, 20)
        .expect("query")
        .is_none());

    let retry = library
        .renormalize_capture(gated_id)
        .expect("failed renormalize");
    assert_eq!(retry.outcome, "failed");
    assert_eq!(retry.parser_id, "fixture");
    assert_eq!(retry.parser_version, "1.0.0");

    let gated_attempts = library.capture_attempts(gated_id).expect("gated attempts");
    assert_eq!(gated_attempts.len(), 2);
    assert!(gated_attempts
        .iter()
        .all(|attempt| attempt.outcome == "failed"));
    assert!(library
        .session_slice("fixture", "fixture-gated", 20, 20)
        .expect("query")
        .is_none());

    let after = library
        .session_slice("fixture", "fixture-fail-retry", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(after.summary.successful_projection_generation, generation);
    assert_eq!(after.messages, messages);
}

/**
 * Codex renormalize after source-root removal appends a newer Attempt.
 */
#[test]
fn codex_renormalize_after_source_removal() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let codex = temp.path().join("codex-home");
    fs::create_dir_all(&codex).expect("codex");
    write_codex_session(&codex);

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("codex", true, Some(codex.as_path()))
        .expect("prefer");
    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.accepted_captures, 1);
    let capture_id = first_capture_id(&library);
    let before = library.capture_attempts(capture_id).expect("attempts");
    assert_eq!(before[0].parser_id, "codex");
    assert_eq!(before[0].parser_version, "1.0.0");
    let original = before[0].clone();

    fs::remove_dir_all(&codex).expect("remove codex root");
    library
        .set_registered_parser_version(SourceKind::Codex, "2.0.0")
        .expect("v2");
    let retry = library
        .renormalize_capture(capture_id)
        .expect("renormalize");
    assert_eq!(retry.outcome, "succeeded");
    assert_eq!(retry.parser_id, "codex");
    assert_eq!(retry.parser_version, "2.0.0");

    let after = library.capture_attempts(capture_id).expect("after");
    assert_eq!(after.len(), 2);
    assert_eq!(after[0], original);
    let detail = library
        .session_slice("codex", "abc12345-1111-2222-3333-abcdefabcdef", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(detail.summary.successful_projection_generation, 2);
    assert_eq!(detail.messages[0].text, "hello codex");
}

/**
 * Claude Code renormalize after source-root removal.
 */
#[test]
fn claude_renormalize_after_source_removal() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let claude = temp.path().join("claude-home");
    fs::create_dir_all(claude.join("projects")).expect("projects");
    write_claude_session(&claude);

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("claude_code", true, Some(claude.as_path()))
        .expect("prefer");
    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.accepted_captures, 1);
    let capture_id = first_capture_id(&library);
    let before = library.capture_attempts(capture_id).expect("attempts");
    assert_eq!(before[0].parser_id, "claude_code");
    let original = before[0].clone();

    fs::remove_dir_all(&claude).expect("remove claude root");
    library
        .set_registered_parser_version(SourceKind::ClaudeCode, "2.0.0")
        .expect("v2");
    let retry = library
        .renormalize_capture(capture_id)
        .expect("renormalize");
    assert_eq!(retry.outcome, "succeeded");
    assert_eq!(retry.parser_id, "claude_code");
    assert_eq!(retry.parser_version, "2.0.0");

    let after = library.capture_attempts(capture_id).expect("after");
    assert_eq!(after.len(), 2);
    assert_eq!(after[0], original);
    let detail = library
        .session_slice(
            "claude_code",
            "123e4567-e89b-12d3-a456-426614174000",
            20,
            20,
        )
        .expect("session")
        .expect("present");
    assert_eq!(detail.summary.successful_projection_generation, 2);
    assert!(detail.messages[0].text.contains("Please review"));
}

/**
 * OpenCode renormalize never reruns the provider executable.
 */
#[test]
fn opencode_renormalize_after_source_removal_without_subprocess() {
    let _guard = OPENCODE_HARNESS_LOCK.lock().expect("lock");
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let opencode = temp.path().join("opencode-home");
    fs::create_dir_all(&opencode).expect("root");
    install_fake_opencode(&opencode);

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("opencode", true, Some(opencode.as_path()))
        .expect("prefer");
    let mut progress = Vec::new();
    let result = library
        .start_sync(SyncRequest::default(), |event| progress.push(event))
        .expect("sync");
    assert_eq!(result.run.accepted_captures, 1);
    assert!(progress.iter().any(|event| matches!(
        event,
        SyncProgress::CandidateFinished { source_kind, outcome, .. }
            if source_kind == "opencode" && outcome == "accepted"
    )));
    let capture_id = first_capture_id(&library);
    let before = library.capture_attempts(capture_id).expect("attempts");
    assert_eq!(before[0].parser_id, "opencode");
    let original = before[0].clone();

    // Removing the root (and fake binary) proves renormalize does not invoke OpenCode.
    fs::remove_dir_all(&opencode).expect("remove opencode root");
    library
        .set_registered_parser_version(SourceKind::OpenCode, "2.0.0")
        .expect("v2");
    let retry = library
        .renormalize_capture(capture_id)
        .expect("renormalize");
    assert_eq!(retry.outcome, "succeeded");
    assert_eq!(retry.parser_id, "opencode");
    assert_eq!(retry.parser_version, "2.0.0");

    let after = library.capture_attempts(capture_id).expect("after");
    assert_eq!(after.len(), 2);
    assert_eq!(after[0], original);
    let detail = library
        .session_slice("opencode", "ses_1", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(detail.summary.successful_projection_generation, 2);
    assert!(detail.messages[0].text.contains("VisiBible"));
}

/**
 * Droid renormalize after sessions-root removal.
 */
#[test]
fn droid_renormalize_after_source_removal() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let sessions = temp.path().join("factory-sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    write_droid_session(&sessions);

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("droid", true, Some(sessions.as_path()))
        .expect("prefer");
    let result = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(result.run.accepted_captures, 1);
    let capture_id = first_capture_id(&library);
    let before = library.capture_attempts(capture_id).expect("attempts");
    assert_eq!(before[0].parser_id, "droid");
    let original = before[0].clone();

    fs::remove_dir_all(&sessions).expect("remove droid root");
    library
        .set_registered_parser_version(SourceKind::Droid, "2.0.0")
        .expect("v2");
    let retry = library
        .renormalize_capture(capture_id)
        .expect("renormalize");
    assert_eq!(retry.outcome, "succeeded");
    assert_eq!(retry.parser_id, "droid");
    assert_eq!(retry.parser_version, "2.0.0");

    let after = library.capture_attempts(capture_id).expect("after");
    assert_eq!(after.len(), 2);
    assert_eq!(after[0], original);
    let detail = library
        .session_slice("droid", "123e4567-e89b-12d3-a456-426614174000", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(detail.summary.successful_projection_generation, 2);
    assert!(detail.messages[0].text.contains("Please review"));
}

/**
 * Unknown persisted Source kinds reject renormalize without Attempt mutation.
 *
 * Plants an unreachable Capture kind through the Distill home database as setup only;
 * assertions go through the public Library seam.
 */
#[test]
fn unknown_persisted_source_kind_rejects_without_mutation() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "fixture-unknown",
        "captures/hello.jsonl",
        concat!(
            r#"{"record_type":"message","role":"user","text":"hi"}"#,
            "\n",
        ),
    );

    let mut library = Library::open(&home).expect("open");
    let report = library.ingest_fixture(&fixture).expect("ingest");
    let capture_id = report.capture_ids[0];
    let before = library.capture_attempts(capture_id).expect("attempts");
    assert_eq!(before.len(), 1);

    let db_path = home.join("distill.db");
    {
        let conn = rusqlite::Connection::open(&db_path).expect("db");
        conn.execute(
            "UPDATE captures SET source_kind = 'not_a_source' WHERE id = ?1",
            [capture_id],
        )
        .expect("plant unknown kind");
    }

    let mut library = Library::open(&home).expect("reopen");
    let err = library
        .renormalize_capture(capture_id)
        .expect_err("unknown kind");
    assert!(matches!(
        err,
        LibraryError::UnknownSourceKind { ref kind } if kind == "not_a_source"
    ));
    assert_eq!(err.code(), "unknown_source_kind");
    assert!(!format!("{err}").contains('/'));

    let after = library.capture_attempts(capture_id).expect("after");
    assert_eq!(after, before);

    // Detect still reports unknown kinds as unhealthy without mutating Captures.
    let detect = library
        .detect_sources(&[SourceDetectRequest {
            kind: "not_a_source".into(),
            configured_root: None,
        }])
        .expect("detect");
    assert_eq!(
        detect[0].error_class.as_deref(),
        Some("unknown_source_kind")
    );
}

/**
 * Projection-failure Capture leaves last-good Projection; renormalize fails the same way.
 */
#[test]
fn renormalize_projection_failure_preserves_last_good_projection() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    let capture_path = write_fixture(
        &fixture,
        "fixture-proj-fail",
        "captures/hello.jsonl",
        concat!(
            r#"{"record_type":"session_meta","title":"Good"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"keep me"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"also keep"}"#,
            "\n",
        ),
    );

    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("baseline");
    let before = library
        .session_slice("fixture", "fixture-proj-fail", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(before.messages.len(), 2);

    fs::write(
        &capture_path,
        concat!(
            r#"{"record_type":"session_meta","title":"Bomb"}"#,
            "\n",
            r#"{"record_type":"force_projection_fail","text":"boom"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"should not publish"}"#,
            "\n",
        ),
    )
    .expect("bomb");
    let bomb = library.ingest_fixture(&fixture).expect("bomb ingest");
    assert_eq!(bomb.failed_attempts, 1);
    let bomb_capture = bomb.capture_ids[0];

    let retry = library
        .renormalize_capture(bomb_capture)
        .expect("renormalize bomb");
    assert_eq!(retry.outcome, "failed");
    let attempts = library.capture_attempts(bomb_capture).expect("attempts");
    assert!(attempts.len() >= 2);
    assert_eq!(
        attempts.last().and_then(|a| a.error_class.as_deref()),
        Some("projection_failed")
    );

    let after = library
        .session_slice("fixture", "fixture-proj-fail", 20, 20)
        .expect("session")
        .expect("present");
    assert_eq!(after.messages[0].text, "keep me");
    assert!(!after
        .messages
        .iter()
        .any(|message| message.text.contains("should not publish")));
}

/**
 * Same-Capture successful empty replay fully clears prior messages and artifacts.
 */
#[test]
fn successful_empty_projection_fully_clears_messages_and_artifacts() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    let rich_path = fixture.join("captures/rich.jsonl");
    let empty_path = fixture.join("captures/empty.jsonl");
    fs::create_dir_all(rich_path.parent().expect("rich parent")).expect("capture parent");
    fs::write(
        &rich_path,
        concat!(
            r#"{"record_type":"session_meta","title":"Rich"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"first user"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"first assistant"}"#,
            "\n",
            r#"{"record_type":"tool_use","text":"echo tool"}"#,
            "\n",
        ),
    )
    .expect("rich capture");
    fs::write(
        &empty_path,
        concat!(
            r#"{"record_type":"session_meta","title":"Empty Projection","summary":"cleared"}"#,
            "\n",
            r#"{"record_type":"require_parser_min","version":"2.0.0"}"#,
            "\n",
        ),
    )
    .expect("empty capture");
    fs::write(
        fixture.join("distill.fixture.json"),
        r#"{
  "version": 1,
  "captures": [
    {"id":"rich","kind":"file","relative_path":"captures/rich.jsonl","external_session_id":"fixture-empty-clear","title":"Rich"},
    {"id":"empty","kind":"file","relative_path":"captures/empty.jsonl","external_session_id":"fixture-empty-clear","title":"Empty"}
  ]
}"#,
    )
    .expect("manifest");

    let mut library = Library::open(&home).expect("open");
    let report = library.ingest_fixture(&fixture).expect("rich");
    let rich = library
        .session_slice("fixture", "fixture-empty-clear", 20, 20)
        .expect("session")
        .expect("present");
    assert!(rich.messages.len() >= 2);
    assert!(!rich.artifacts.is_empty());

    let empty_capture = report
        .capture_ids
        .iter()
        .copied()
        .find(|capture_id| {
            library
                .capture_attempts(*capture_id)
                .expect("attempts")
                .first()
                .is_some_and(|attempt| attempt.outcome == "failed")
        })
        .expect("gated Capture");
    library
        .set_registered_parser_version(SourceKind::Fixture, "2.0.0")
        .expect("register parser");
    let retry = library
        .renormalize_capture(empty_capture)
        .expect("renormalize empty");
    assert_eq!(retry.outcome, "succeeded");
    let empty = library
        .session_slice("fixture", "fixture-empty-clear", 20, 20)
        .expect("session")
        .expect("present");
    assert!(empty.messages.is_empty());
    assert!(empty.artifacts.is_empty());
    assert_eq!(empty.summary.title.as_deref(), Some("Empty Projection"));
    assert!(library.search("first user", 20).expect("search").is_empty());
}
