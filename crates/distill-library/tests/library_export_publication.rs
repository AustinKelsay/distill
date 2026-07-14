//! Library export publication contracts for issue #25.
//!
//! Covers eligibility parity, golden JSONL/turn pairs, publication lifecycle,
//! cancellation, open reconciliation, and `test-faults` recovery boundaries.

use std::fs;
use std::path::{Path, PathBuf};

use distill_library::{
    ExportDataset, ExportOmissionReason, ExportProgress, ExportProgressControl, ExportStatus,
    Library, SessionCurationRequest, SessionIdentity, EXPORT_FORMAT_ID,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

/**
 * Write a Fixture root with one Capture Candidate body.
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
      "title": "{session_id}"
    }}
  ]
}}"#
    );
    fs::write(root.join("distill.fixture.json"), manifest).expect("write manifest");
    capture_path
}

/**
 * Multi-message Fixture body covering meta messages and consecutive-user replacement.
 */
fn rich_body() -> &'static str {
    concat!(
        r#"{"record_type":"session_meta","title":"Export Me","summary":"projection summary","source_url":"https://example.test/export","metadata":{"capturePath":"/tmp/demo/session.jsonl"}}"#,
        "\n",
        r#"{"record_type":"message","role":"user","text":"draft one","created_at":"2026-07-11T10:00:00Z"}"#,
        "\n",
        r#"{"record_type":"message","role":"user","text":"draft two","created_at":"2026-07-11T10:00:01Z"}"#,
        "\n",
        r#"{"record_type":"message","role":"assistant","text":"thinking","message_kind":"meta","created_at":"2026-07-11T10:00:02Z","metadata":{"tool":"plan"}}"#,
        "\n",
        r#"{"record_type":"message","role":"assistant","text":"final answer","created_at":"2026-07-11T10:00:03Z"}"#,
        "\n",
        r#"{"record_type":"message","role":"user","text":"trailing only","created_at":"2026-07-11T10:00:04Z"}"#,
        "\n",
    )
}

/**
 * Open a temp home and ingest one Fixture session.
 */
fn seeded_library(session_id: &str, body: &str) -> (TempDir, Library, SessionIdentity) {
    let home = TempDir::new().expect("temp home");
    let fixture = TempDir::new().expect("temp fixture");
    write_fixture(
        fixture.path(),
        session_id,
        &format!("sessions/{session_id}.jsonl"),
        body,
    );
    let mut library = Library::open(home.path()).expect("open library");
    library
        .ingest_fixture(fixture.path())
        .expect("ingest fixture");
    let identity = SessionIdentity {
        source_kind: "fixture".into(),
        external_session_id: session_id.into(),
    };
    (home, library, identity)
}

fn request(identity: &SessionIdentity, name: &str) -> SessionCurationRequest {
    SessionCurationRequest {
        source_kind: identity.source_kind.clone(),
        external_session_id: identity.external_session_id.clone(),
        name: name.into(),
    }
}

fn continue_progress(_progress: ExportProgress) -> ExportProgressControl {
    ExportProgressControl::Continue
}

fn activity_count(home: &Path, event_type: &str) -> i64 {
    let conn = Connection::open(home.join("distill.db")).expect("db");
    conn.query_row(
        "SELECT COUNT(*) FROM activity_events WHERE event_type = ?1",
        [event_type],
        |row| row.get(0),
    )
    .expect("count")
}

fn export_status(home: &Path, export_id: i64) -> String {
    let conn = Connection::open(home.join("distill.db")).expect("db");
    conn.query_row(
        "SELECT status FROM exports WHERE id = ?1",
        [export_id],
        |row| row.get(0),
    )
    .expect("status")
}

fn export_row_count(home: &Path) -> i64 {
    let conn = Connection::open(home.join("distill.db")).expect("db");
    conn.query_row("SELECT COUNT(*) FROM exports", [], |row| row.get(0))
        .expect("count")
}

#[test]
fn preview_and_publish_share_eligibility_and_omit_blocked_sessions() {
    let home = TempDir::new().expect("home");
    let fixture = TempDir::new().expect("fixture");
    let sessions = [
        ("ok-train", "train"),
        ("fav-train", "train"),
        ("excluded", "train"),
        ("sensitive", "train"),
        ("conflict", "train"),
        ("favorite-only", "favorite"),
        ("unreviewed", ""),
        ("holdout-ready", "holdout"),
    ];
    let mut captures = Vec::new();
    for (session_id, _) in sessions {
        let relative = format!("sessions/{session_id}.jsonl");
        write_fixture(
            fixture.path(),
            session_id,
            &relative,
            concat!(
                r#"{"record_type":"session_meta","title":"t","summary":"s"}"#,
                "\n",
                r#"{"record_type":"message","role":"user","text":"u"}"#,
                "\n",
                r#"{"record_type":"message","role":"assistant","text":"a"}"#,
                "\n",
            ),
        );
        captures.push(relative);
    }
    let entries: Vec<String> = sessions
        .iter()
        .zip(captures.iter())
        .map(|((session_id, _), relative)| {
            format!(
                r#"{{"id":"{session_id}","kind":"file","relative_path":"{relative}","external_session_id":"{session_id}","title":"{session_id}"}}"#
            )
        })
        .collect();
    fs::write(
        fixture.path().join("distill.fixture.json"),
        format!(r#"{{"version":1,"captures":[{}]}}"#, entries.join(",")),
    )
    .expect("manifest");

    let mut library = Library::open(home.path()).expect("open");
    library.ingest_fixture(fixture.path()).expect("ingest");

    for (session_id, label) in sessions {
        let identity = SessionIdentity {
            source_kind: "fixture".into(),
            external_session_id: session_id.into(),
        };
        if !label.is_empty() {
            library
                .toggle_session_label(request(&identity, label))
                .expect("label");
        }
        if session_id == "fav-train" {
            library
                .toggle_session_label(request(&identity, "favorite"))
                .expect("favorite");
        }
        if session_id == "sensitive" {
            library
                .toggle_session_label(request(&identity, "sensitive"))
                .expect("sensitive");
        }
    }

    // Toggle exclusivity cannot leave train+exclude or train+holdout; seed those
    // policy cases through SQL so omission reasons remain observable.
    seed_extra_manual_label(home.path(), "excluded", "exclude");
    seed_extra_manual_label(home.path(), "conflict", "holdout");

    let before_activity = activity_count(home.path(), "export_written");
    let before_exports = export_row_count(home.path());
    let exports_before = fs::read_dir(home.path().join("exports"))
        .map(|entries| entries.count())
        .unwrap_or(0);

    let preview = library
        .preview_export(ExportDataset::Train)
        .expect("preview");
    assert_eq!(preview.format_id, EXPORT_FORMAT_ID);
    assert_eq!(
        preview
            .eligible
            .iter()
            .map(|id| id.external_session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["fav-train", "ok-train"]
    );
    assert!(preview.omitted.iter().any(|row| {
        row.identity.external_session_id == "excluded"
            && row.reason == ExportOmissionReason::Exclude
    }));
    assert!(preview.omitted.iter().any(|row| {
        row.identity.external_session_id == "sensitive"
            && row.reason == ExportOmissionReason::Sensitive
    }));
    assert!(preview.omitted.iter().any(|row| {
        row.identity.external_session_id == "conflict"
            && row.reason == ExportOmissionReason::ConflictingDatasetLabels
    }));
    assert!(!preview
        .eligible
        .iter()
        .any(|id| id.external_session_id == "favorite-only"));
    assert!(preview.omitted.iter().any(|row| {
        row.identity.external_session_id == "favorite-only"
            && row.reason == ExportOmissionReason::FavoriteOnly
    }));
    assert!(preview.omitted.iter().any(|row| {
        row.identity.external_session_id == "unreviewed"
            && row.reason == ExportOmissionReason::Unreviewed
    }));
    assert!(!preview
        .eligible
        .iter()
        .any(|id| id.external_session_id == "holdout-ready"));

    assert_eq!(
        activity_count(home.path(), "export_written"),
        before_activity
    );
    assert_eq!(export_row_count(home.path()), before_exports);
    let exports_after_preview = fs::read_dir(home.path().join("exports"))
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(exports_after_preview, exports_before);

    let published = library
        .publish_export(ExportDataset::Train, continue_progress)
        .expect("publish");
    assert_eq!(published.status, ExportStatus::Published);
    assert_eq!(published.record_count, 2);
    assert_eq!(
        published
            .eligible
            .iter()
            .map(|id| id.external_session_id.as_str())
            .collect::<Vec<_>>(),
        preview
            .eligible
            .iter()
            .map(|id| id.external_session_id.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        activity_count(home.path(), "export_written"),
        before_activity + 1
    );
}

/**
 * Insert an additional manual label without going through toggle exclusivity.
 */
fn seed_extra_manual_label(home: &Path, external_session_id: &str, label_name: &str) {
    let conn = Connection::open(home.join("distill.db")).expect("db");
    let session_id: i64 = conn
        .query_row(
            "SELECT id FROM sessions WHERE external_session_id = ?1",
            [external_session_id],
            |row| row.get(0),
        )
        .expect("session");
    let label_id: i64 = conn
        .query_row(
            "SELECT id FROM labels WHERE name = ?1",
            [label_name],
            |row| row.get(0),
        )
        .expect("label");
    conn.execute(
        "INSERT OR IGNORE INTO label_assignments (object_type, object_id, label_id, origin, created_at)
         VALUES ('session', ?1, ?2, 'manual', '2026-07-11T00:00:00Z')",
        [session_id, label_id],
    )
    .expect("assign");
}

#[test]
fn publish_writes_deterministic_jsonl_with_turn_pairs_and_manual_curation() {
    let (home, mut library, identity) = seeded_library("golden-export", rich_body());
    library
        .toggle_session_label(request(&identity, "train"))
        .expect("train");
    library
        .toggle_session_label(request(&identity, "favorite"))
        .expect("favorite");
    library
        .add_session_tag(request(&identity, "alpha"))
        .expect("tag");

    let result = library
        .publish_export(ExportDataset::Train, continue_progress)
        .expect("publish");
    assert_eq!(result.status, ExportStatus::Published);
    assert_eq!(result.format_id, EXPORT_FORMAT_ID);
    assert_eq!(result.record_count, 1);
    let output = result.output_path.expect("output path");
    assert!(output.starts_with(home.path().join("exports").to_str().unwrap()));
    let bytes = fs::read(&output).expect("read export");
    assert_eq!(result.byte_size, Some(bytes.len() as u64));
    assert_eq!(
        result.sha256.as_deref(),
        Some(hex::encode(Sha256::digest(&bytes)).as_str())
    );

    let line = std::str::from_utf8(&bytes)
        .expect("utf8")
        .trim_end()
        .to_string();
    let payload: serde_json::Value = serde_json::from_str(&line).expect("json");
    assert_eq!(payload["source"], "fixture");
    assert_eq!(payload["external_session_id"], "golden-export");
    assert_eq!(payload["summary"], "projection summary");
    assert_eq!(payload["labels"], serde_json::json!(["favorite", "train"]));
    assert_eq!(payload["tags"], serde_json::json!(["alpha"]));
    assert_eq!(payload["turn_pairs"].as_array().unwrap().len(), 1);
    assert_eq!(payload["turn_pairs"][0]["user"], "draft two");
    assert_eq!(payload["turn_pairs"][0]["assistant"], "final answer");
    let messages = payload["messages"].as_array().expect("messages");
    assert!(messages
        .iter()
        .any(|message| { message["message_kind"] == "meta" && message["text"] == "thinking" }));
    assert!(messages.last().unwrap()["text"] == "trailing only");

    let again = library
        .publish_export(ExportDataset::Train, continue_progress)
        .expect("second publish");
    let again_bytes = fs::read(again.output_path.as_ref().unwrap()).expect("read again");
    let first_payload: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
    let second_line = std::str::from_utf8(&again_bytes).unwrap().trim_end();
    let second_payload: serde_json::Value = serde_json::from_str(second_line).unwrap();
    assert_eq!(first_payload["messages"], second_payload["messages"]);
    assert_eq!(first_payload["turn_pairs"], second_payload["turn_pairs"]);
    assert_eq!(first_payload["labels"], second_payload["labels"]);
    assert_eq!(first_payload["tags"], second_payload["tags"]);
}

#[test]
fn publish_reaches_published_only_after_rename_and_export_written() {
    let (home, mut library, identity) = seeded_library(
        "lifecycle",
        concat!(
            r#"{"record_type":"session_meta","title":"Lifecycle","summary":"s"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"u"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"a"}"#,
            "\n",
        ),
    );
    library
        .toggle_session_label(request(&identity, "holdout"))
        .expect("holdout");

    let result = library
        .publish_export(ExportDataset::Holdout, continue_progress)
        .expect("publish");
    assert_eq!(result.status, ExportStatus::Published);
    assert_eq!(export_status(home.path(), result.export_id), "published");
    let output = result.output_path.as_ref().unwrap();
    assert!(Path::new(output).is_file());
    assert!(!Path::new(&format!("{output}.tmp")).exists());
    assert_eq!(activity_count(home.path(), "export_written"), 1);

    let conn = Connection::open(home.path().join("distill.db")).expect("db");
    let (status, output_path, sha256, byte_size, record_count): (
        String,
        String,
        String,
        i64,
        i64,
    ) = conn
        .query_row(
            "SELECT status, output_path, sha256, byte_size, record_count FROM exports WHERE id = ?1",
            [result.export_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .expect("row");
    assert_eq!(status, "published");
    assert_eq!(output_path, *output);
    assert_eq!(sha256, result.sha256.unwrap());
    assert_eq!(byte_size as u64, result.byte_size.unwrap());
    assert_eq!(record_count as u64, result.record_count);
}

#[test]
fn publish_cancellation_is_terminal_without_export_written() {
    let (home, mut library, identity) = seeded_library(
        "cancel-export",
        concat!(
            r#"{"record_type":"session_meta","title":"Cancel","summary":"s"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"u"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"a"}"#,
            "\n",
        ),
    );
    library
        .toggle_session_label(request(&identity, "train"))
        .expect("train");

    let result = library
        .publish_export(ExportDataset::Train, |progress| match progress {
            ExportProgress::Preparing { .. } => ExportProgressControl::Cancel,
            _ => ExportProgressControl::Continue,
        })
        .expect("cancelled result");
    assert_eq!(result.status, ExportStatus::Cancelled);
    assert!(result.output_path.is_none());
    assert_eq!(export_status(home.path(), result.export_id), "cancelled");
    assert_eq!(activity_count(home.path(), "export_written"), 0);
    let tmp_count = fs::read_dir(home.path().join("exports"))
        .expect("exports")
        .filter(|entry| {
            entry
                .as_ref()
                .ok()
                .and_then(|e| e.file_name().into_string().ok())
                .is_some_and(|name| name.ends_with(".jsonl.tmp"))
        })
        .count();
    assert_eq!(tmp_count, 0);
}

#[cfg(feature = "test-faults")]
mod fault_recovery {
    use super::*;
    use distill_library::faults::{self, FaultPoint};

    #[test]
    fn after_temp_write_fault_reopens_without_inventing_success() {
        let (home, mut library, identity) = seeded_library(
            "fault-temp",
            concat!(
                r#"{"record_type":"session_meta","title":"Fault","summary":"s"}"#,
                "\n",
                r#"{"record_type":"message","role":"user","text":"u"}"#,
                "\n",
                r#"{"record_type":"message","role":"assistant","text":"a"}"#,
                "\n",
            ),
        );
        library
            .toggle_session_label(request(&identity, "train"))
            .expect("train");
        faults::arm(FaultPoint::AfterExportTempWrite);
        let err = library
            .publish_export(ExportDataset::Train, continue_progress)
            .expect_err("fault");
        assert_eq!(err.code(), "injected_test_fault");
        assert_eq!(activity_count(home.path(), "export_written"), 0);

        let reopened = Library::open(home.path()).expect("reopen");
        assert!(reopened.open_reconciliation().classified_incomplete_exports >= 1);
        let conn = Connection::open(home.path().join("distill.db")).expect("db");
        let status: String = conn
            .query_row(
                "SELECT status FROM exports ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("status");
        assert_eq!(status, "failed_publish");
        assert_eq!(activity_count(home.path(), "export_written"), 0);
        let tmp_count = fs::read_dir(home.path().join("exports"))
            .expect("exports")
            .filter(|entry| {
                entry
                    .as_ref()
                    .ok()
                    .and_then(|e| e.file_name().into_string().ok())
                    .is_some_and(|name| name.ends_with(".jsonl.tmp"))
            })
            .count();
        assert_eq!(tmp_count, 0);
    }

    #[test]
    fn after_committed_before_rename_fault_reopens_safely() {
        let (home, mut library, identity) = seeded_library(
            "fault-commit",
            concat!(
                r#"{"record_type":"session_meta","title":"Fault","summary":"s"}"#,
                "\n",
                r#"{"record_type":"message","role":"user","text":"u"}"#,
                "\n",
                r#"{"record_type":"message","role":"assistant","text":"a"}"#,
                "\n",
            ),
        );
        library
            .toggle_session_label(request(&identity, "train"))
            .expect("train");
        faults::arm(FaultPoint::AfterExportCommittedBeforeRename);
        let err = library
            .publish_export(ExportDataset::Train, continue_progress)
            .expect_err("fault");
        assert_eq!(err.code(), "injected_test_fault");

        let reopened = Library::open(home.path()).expect("reopen");
        assert!(reopened.open_reconciliation().classified_incomplete_exports >= 1);
        assert_eq!(activity_count(home.path(), "export_written"), 0);
        let final_files = fs::read_dir(home.path().join("exports"))
            .expect("exports")
            .filter(|entry| {
                entry
                    .as_ref()
                    .ok()
                    .and_then(|e| e.file_name().into_string().ok())
                    .is_some_and(|name| name.ends_with(".jsonl") && !name.ends_with(".jsonl.tmp"))
            })
            .count();
        assert_eq!(final_files, 0);
    }

    #[test]
    fn after_rename_before_finalization_fault_reopens_as_published_when_checksum_matches() {
        let (home, mut library, identity) = seeded_library(
            "fault-rename",
            concat!(
                r#"{"record_type":"session_meta","title":"Fault","summary":"s"}"#,
                "\n",
                r#"{"record_type":"message","role":"user","text":"u"}"#,
                "\n",
                r#"{"record_type":"message","role":"assistant","text":"a"}"#,
                "\n",
            ),
        );
        library
            .toggle_session_label(request(&identity, "train"))
            .expect("train");
        faults::arm(FaultPoint::AfterExportRenameBeforeFinalization);
        let err = library
            .publish_export(ExportDataset::Train, continue_progress)
            .expect_err("fault");
        assert_eq!(err.code(), "injected_test_fault");

        let final_files_before: Vec<_> =
            fs::read_dir(home.path().join("exports"))
                .expect("exports")
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry.file_name().into_string().ok().is_some_and(|name| {
                        name.ends_with(".jsonl") && !name.ends_with(".jsonl.tmp")
                    })
                })
                .map(|entry| entry.path())
                .collect();
        assert_eq!(final_files_before.len(), 1);
        let preserved = final_files_before[0].clone();
        let preserved_bytes = fs::read(&preserved).expect("bytes before reopen");

        let reopened = Library::open(home.path()).expect("reopen");
        assert!(reopened.open_reconciliation().classified_incomplete_exports >= 1);
        assert_eq!(activity_count(home.path(), "export_written"), 1);
        assert!(preserved.is_file());
        assert_eq!(fs::read(&preserved).expect("bytes after"), preserved_bytes);

        let conn = Connection::open(home.path().join("distill.db")).expect("db");
        let (status, output_path): (String, Option<String>) = conn
            .query_row(
                "SELECT status, output_path FROM exports ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("row");
        assert_eq!(status, "published");
        assert_eq!(output_path.as_deref(), preserved.to_str());
    }

    #[test]
    fn finalize_failure_after_rename_keeps_committed_row_for_recovery() {
        let (home, mut library, identity) = seeded_library(
            "fault-finalize",
            concat!(
                r#"{"record_type":"session_meta","title":"Fault","summary":"s"}"#,
                "\n",
                r#"{"record_type":"message","role":"user","text":"u"}"#,
                "\n",
                r#"{"record_type":"message","role":"assistant","text":"a"}"#,
                "\n",
            ),
        );
        library
            .toggle_session_label(request(&identity, "train"))
            .expect("train");
        faults::arm(FaultPoint::DuringExportFinalizationBeforeCommit);
        let err = library
            .publish_export(ExportDataset::Train, continue_progress)
            .expect_err("fault");
        assert_eq!(err.code(), "injected_test_fault");
        assert_eq!(activity_count(home.path(), "export_written"), 0);

        let conn = Connection::open(home.path().join("distill.db")).expect("db");
        let status: String = conn
            .query_row(
                "SELECT status FROM exports ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("status");
        assert_eq!(status, "committed");

        let reopened = Library::open(home.path()).expect("reopen");
        assert!(reopened.open_reconciliation().classified_incomplete_exports >= 1);
        assert_eq!(activity_count(home.path(), "export_written"), 1);
        let recovered: String = Connection::open(home.path().join("distill.db"))
            .expect("db")
            .query_row(
                "SELECT status FROM exports ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("recovered status");
        assert_eq!(recovered, "published");
    }
}
