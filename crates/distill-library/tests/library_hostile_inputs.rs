//! Shared hostile-input / privacy-hardening contracts for issue #32.
//!
//! Covers traversal, symlinks, oversized captures, deep/oversized JSON,
//! malformed Unicode, HTML/script payloads, and secret-bearing Activity
//! redaction across Fixture plus Codex/Claude/Droid. OpenCode timeout/output
//! bounds stay in `library_opencode_source` / `library_ops_sync`; this suite
//! asserts the shared redaction/JSON-depth policy those adapters now use.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use distill_library::{
    safe_caller_message, ActivityListRequest, Library, LibraryError, SessionListRequest,
    SyncRequest, SyncRunResult,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tempfile::TempDir;

/// Nesting above the product JSON depth cap.
const HOSTILE_JSON_DEPTH: usize = 80;

fn open_home_db(home: &Path) -> Connection {
    Connection::open(home.join("distill.db")).expect("open db")
}

fn write_fixture_file(root: &Path, session_id: &str, relative: &str, body: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent");
    }
    fs::write(&path, body).expect("body");
    write_fixture_manifest(root, session_id, relative);
}

fn write_fixture_virtual(root: &Path, session_id: &str, virtual_text: &str) {
    fs::create_dir_all(root).expect("root");
    let manifest = json!({
        "version": 1,
        "captures": [{
            "id": session_id,
            "kind": "virtual",
            "virtual_text": virtual_text,
            "external_session_id": session_id,
            "title": session_id
        }]
    });
    fs::write(
        root.join("distill.fixture.json"),
        serde_json::to_string_pretty(&manifest).expect("manifest"),
    )
    .expect("write manifest");
}

fn write_fixture_manifest(root: &Path, session_id: &str, relative: &str) {
    fs::create_dir_all(root).expect("root");
    let manifest = json!({
        "version": 1,
        "captures": [{
            "id": session_id,
            "kind": "file",
            "relative_path": relative,
            "external_session_id": session_id,
            "title": session_id
        }]
    });
    fs::write(
        root.join("distill.fixture.json"),
        serde_json::to_string_pretty(&manifest).expect("manifest"),
    )
    .expect("write manifest");
}

fn deep_json_object(depth: usize) -> Value {
    let mut value = json!({"leaf": true, "text": "deep"});
    for _ in 0..depth {
        value = json!({ "n": value });
    }
    value
}

fn write_codex_session(root: &Path, body: &str) -> PathBuf {
    let relative =
        "sessions/2026/07/12/rollout-2026-07-12T10-00-00-hostile222-1111-2222-3333-abcdefabcdef.jsonl";
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    fs::write(&path, body).expect("codex body");
    path
}

fn write_claude_session(root: &Path, body: &str) -> PathBuf {
    let relative = "projects/hostile/123e4567-e89b-12d3-a456-426614174099.jsonl";
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    fs::write(&path, body).expect("claude body");
    path
}

fn write_droid_session(root: &Path, body: &str) -> PathBuf {
    let path = root
        .join("ws-hostile")
        .join("123e4567-e89b-12d3-a456-426614174088.jsonl");
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    fs::write(&path, body).expect("droid body");
    path
}

fn sync_kind(library: &mut Library, kind: &str, root: &Path) -> SyncRunResult {
    library
        .set_source_preference(kind, true, Some(root))
        .expect("pref");
    library
        .start_sync(
            SyncRequest {
                source_kinds: vec![kind.to_string()],
            },
            |_| {},
        )
        .expect("sync")
}

#[test]
fn fixture_traversal_and_symlink_escape_reject_without_capture() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    let outside = temp.path().join("outside-secret.jsonl");
    fs::create_dir_all(fixture.join("captures")).expect("captures");
    fs::write(
        &outside,
        "{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"leak\"}\n",
    )
    .expect("outside");
    symlink(&outside, fixture.join("captures/escaped.jsonl")).expect("symlink");
    write_fixture_manifest(&fixture, "escaped", "captures/escaped.jsonl");

    let mut library = Library::open(&home).expect("open");
    let err = library
        .ingest_fixture(&fixture)
        .expect_err("symlink escape must fail");
    assert!(
        matches!(err, LibraryError::PathOutsideConfiguredRoot { .. }),
        "{err:?}"
    );
    assert!(!safe_caller_message(&err).contains("outside-secret"));
    assert_eq!(
        library
            .list_sessions(SessionListRequest::default())
            .expect("list")
            .items
            .len(),
        0
    );

    let traversal = temp.path().join("traverse");
    fs::create_dir_all(&traversal).expect("traverse");
    write_fixture_manifest(&traversal, "escape", "../outside-secret.jsonl");
    let err = library
        .ingest_fixture(&traversal)
        .expect_err("traversal must fail");
    assert!(matches!(
        err,
        LibraryError::PathOutsideConfiguredRoot { .. }
    ));
}

#[test]
fn fixture_oversized_capture_rejected_before_acceptance() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    let relative = "captures/huge.jsonl";
    let path = fixture.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    fs::write(&path, vec![b'x'; 64]).expect("huge");
    write_fixture_manifest(&fixture, "huge", relative);

    let mut library = Library::open_with_limits(&home, 32).expect("open");
    let err = library
        .ingest_fixture(&fixture)
        .expect_err("oversized must fail");
    assert!(
        matches!(err, LibraryError::CaptureTooLarge { .. }),
        "{err:?}"
    );
    assert!(safe_caller_message(&err).contains("size limit"));
    assert!(!safe_caller_message(&err).contains(relative));
}

#[test]
fn deep_and_oversized_json_lines_fail_typed_parse_without_projection() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    let deep = deep_json_object(HOSTILE_JSON_DEPTH);
    let line = format!(
        "{{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"x\",\"meta\":{deep}}}\n"
    );
    write_fixture_file(&fixture, "deep-json", "captures/deep.jsonl", &line);

    let mut library = Library::open(&home).expect("open");
    let report = library
        .ingest_fixture(&fixture)
        .expect("ingest records capture");
    assert_eq!(report.accepted_captures, 1);
    assert_eq!(report.successful_attempts, 0);
    assert_eq!(report.failed_attempts, 1);
    assert!(library
        .session_slice("fixture", "deep-json", 20, 20)
        .expect("slice")
        .is_none());

    let oversized_line = format!(
        "{{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"{}\"}}\n",
        "y".repeat(1024 * 1024 + 8)
    );
    let oversized_root = temp.path().join("oversized-json");
    write_fixture_file(
        &oversized_root,
        "oversized-json",
        "captures/oversized.jsonl",
        &oversized_line,
    );
    let report = library
        .ingest_fixture(&oversized_root)
        .expect("ingest oversized line");
    assert_eq!(report.successful_attempts, 0);
    assert_eq!(report.failed_attempts, 1);
}

#[test]
fn malformed_unicode_bytes_fail_before_false_acceptance() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    let relative = "captures/bad-utf8.jsonl";
    let path = fixture.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    fs::write(&path, [0xff, 0xfe, 0xfd, b'\n']).expect("bad bytes");
    write_fixture_manifest(&fixture, "bad-utf8", relative);

    let mut library = Library::open(&home).expect("open");
    let err = library
        .ingest_fixture(&fixture)
        .expect_err("invalid utf-8 must not become an inline Capture");
    assert!(matches!(err, LibraryError::InvalidArgument(_)), "{err:?}");
    assert!(!safe_caller_message(&err).contains(relative));
    assert_eq!(
        library
            .list_sessions(SessionListRequest::default())
            .expect("list")
            .items
            .len(),
        0
    );
}

#[test]
fn html_script_payload_is_literal_transcript_not_executed_policy() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    let payload = "<script>alert('xss')</script><img src=x onerror=alert(1)>";
    write_fixture_virtual(
        &fixture,
        "html-script",
        &format!(
            "{{\"record_type\":\"message\",\"role\":\"user\",\"text\":{}}}\n{{\"record_type\":\"message\",\"role\":\"assistant\",\"text\":\"ok\"}}\n",
            Value::String(payload.into())
        ),
    );

    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("ingest");
    let detail = library
        .session_slice("fixture", "html-script", 20, 20)
        .expect("slice")
        .expect("session");
    assert_eq!(detail.messages[0].text, payload);
    assert!(detail.messages[0].text.contains("<script>"));
}

#[test]
fn secret_bearing_activity_and_safe_caller_messages_are_redacted() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let mut library = Library::open(&home).expect("open");
    drop(library);

    let conn = open_home_db(&home);
    conn.execute(
        "INSERT INTO activity_events (
            event_type, occurred_at, source_kind, session_id, capture_id, attempt_id, payload_json
         ) VALUES ('sync_failed', ?1, NULL, NULL, NULL, NULL, ?2)",
        params![
            chrono::Utc::now().to_rfc3339(),
            r#"{
              "reason": "provider failed",
              "api_key": "sk-live-SECRET",
              "authorization": "Bearer abc.def",
              "token": "tok_123",
              "note": "Bearer abc.def leaked",
              "output_path": "/tmp/secret/token/path.jsonl"
            }"#
        ],
    )
    .expect("insert");
    drop(conn);

    library = Library::open(&home).expect("reopen");
    let page = library
        .list_activity(ActivityListRequest {
            limit: 10,
            cursor: None,
        })
        .expect("page");
    let payload = &page
        .items
        .iter()
        .find(|item| item.event_type == "sync_failed")
        .expect("event")
        .payload_json;
    assert!(payload.get("api_key").is_none());
    assert!(payload.get("authorization").is_none());
    assert!(payload.get("token").is_none());
    assert!(payload.get("output_path").is_none());
    assert_eq!(
        payload.get("note").and_then(Value::as_str),
        Some("[redacted]")
    );
    assert_eq!(
        safe_caller_message(&LibraryError::PathOutsideConfiguredRoot {
            path: PathBuf::from("/tmp/secret-token-path.jsonl"),
            root: PathBuf::from("/tmp/fixture"),
        }),
        "path escaped the configured Source root"
    );
}

#[test]
fn codex_claude_droid_skip_symlink_candidates_and_reject_deep_json() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let mut library = Library::open(&home).expect("open");
    let deep = deep_json_object(HOSTILE_JSON_DEPTH);

    let codex = temp.path().join("codex");
    fs::create_dir_all(codex.join("sessions")).expect("sessions");
    let outside = temp.path().join("codex-outside.jsonl");
    fs::write(
        &outside,
        r#"{"timestamp":"2026-07-12T10:00:00.000Z","type":"session_meta","payload":{"id":"hostile111-1111-2222-3333-abcdefabcdef"}}
"#,
    )
    .expect("outside");
    symlink(
        &outside,
        codex
            .join("sessions")
            .join("rollout-2026-07-12T10-00-00-hostile111-1111-2222-3333-abcdefabcdef.jsonl"),
    )
    .expect("symlink");
    assert_eq!(
        sync_kind(&mut library, "codex", &codex)
            .run
            .accepted_captures,
        0
    );

    let codex_deep = temp.path().join("codex-deep");
    write_codex_session(
        &codex_deep,
        &format!(
            "{{\"timestamp\":\"2026-07-12T10:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"hostile222-1111-2222-3333-abcdefabcdef\",\"meta\":{deep}}}}}\n{{\"timestamp\":\"2026-07-12T10:00:01.000Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"hi\"}}]}}}}\n"
        ),
    );
    assert_eq!(
        sync_kind(&mut library, "codex", &codex_deep)
            .run
            .successful_attempts,
        0
    );

    let claude = temp.path().join("claude");
    fs::create_dir_all(claude.join("projects/hostile")).expect("projects");
    let claude_outside = temp.path().join("claude-outside.jsonl");
    fs::write(&claude_outside, "{}\n").expect("outside");
    symlink(
        &claude_outside,
        claude.join("projects/hostile/123e4567-e89b-12d3-a456-426614174099.jsonl"),
    )
    .expect("symlink");
    assert_eq!(
        sync_kind(&mut library, "claude_code", &claude)
            .run
            .accepted_captures,
        0
    );

    let claude_deep = temp.path().join("claude-deep");
    write_claude_session(
        &claude_deep,
        &format!(
            "{{\"type\":\"user\",\"uuid\":\"u1\",\"sessionId\":\"123e4567-e89b-12d3-a456-426614174099\",\"timestamp\":\"2026-07-12T11:00:00.000Z\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"hi\",\"meta\":{deep}}}]}}}}\n"
        ),
    );
    assert_eq!(
        sync_kind(&mut library, "claude_code", &claude_deep)
            .run
            .successful_attempts,
        0
    );

    let droid = temp.path().join("droid");
    fs::create_dir_all(droid.join("ws-hostile")).expect("ws");
    let droid_outside = temp.path().join("droid-outside.jsonl");
    fs::write(&droid_outside, "{}\n").expect("outside");
    symlink(
        &droid_outside,
        droid.join("ws-hostile/123e4567-e89b-12d3-a456-426614174088.jsonl"),
    )
    .expect("symlink");
    assert_eq!(
        sync_kind(&mut library, "droid", &droid)
            .run
            .accepted_captures,
        0
    );

    let droid_deep = temp.path().join("droid-deep");
    write_droid_session(
        &droid_deep,
        &format!(
            "{{\"type\":\"session_start\",\"id\":\"123e4567-e89b-12d3-a456-426614174088\",\"title\":\"deep\",\"meta\":{deep}}}\n{{\"type\":\"message\",\"id\":\"u1\",\"message\":{{\"role\":\"user\",\"content\":[\"hi\"]}}}}\n"
        ),
    );
    assert_eq!(
        sync_kind(&mut library, "droid", &droid_deep)
            .run
            .successful_attempts,
        0
    );
}

#[test]
fn provider_process_bound_errors_are_safe_for_callers() {
    let err = LibraryError::ProviderProcessBoundExceeded {
        detail: "provider output exceeded configured byte cap at /secret/token/path".into(),
    };
    let safe = safe_caller_message(&err);
    assert!(safe.contains("provider process bound exceeded"));
    assert!(!safe.contains("/secret/token/path"));
}
