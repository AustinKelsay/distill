//! Library Activity and Operations diagnostics contracts for issue #30.
//!
//! Proves append-only Activity paging, redacted payloads, Operations Sync/export
//! summaries that do not mutate Activity, and `recent_activity` compatibility.

use std::fs;
use std::path::{Path, PathBuf};

use distill_library::{
    ActivityListRequest, ExportDataset, Library, LibraryError, OperationsRequest, RepairOptions,
    SyncRequest,
};
use rusqlite::{params, Connection};
use tempfile::TempDir;

/**
 * Write a Fixture root with one Capture Candidate.
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
 * Write a Fixture root with two Capture Candidates for multi-event Activity.
 */
fn write_two_candidate_fixture(root: &Path) {
    let captures = root.join("captures");
    fs::create_dir_all(&captures).expect("captures");
    for (name, text) in [("one", "one"), ("two", "two")] {
        fs::write(
            captures.join(format!("{name}.jsonl")),
            format!("{{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"{text}\"}}\n"),
        )
        .expect("capture");
    }
    fs::write(
        root.join("distill.fixture.json"),
        r#"{
  "version": 1,
  "captures": [
    {"id": "one", "kind": "file", "relative_path": "captures/one.jsonl", "external_session_id": "act-one"},
    {"id": "two", "kind": "file", "relative_path": "captures/two.jsonl", "external_session_id": "act-two"}
  ]
}"#,
    )
    .expect("manifest");
}

fn open_home_db(home: &Path) -> Connection {
    Connection::open(home.join("distill.db")).expect("open distill.db")
}

#[test]
fn activity_page_is_newest_first_deterministic_and_paged() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_two_candidate_fixture(&fixture);

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("pref");
    library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");

    let first = library
        .list_activity(ActivityListRequest {
            limit: 2,
            cursor: None,
        })
        .expect("first page");
    assert_eq!(first.items.len(), 2);
    assert!(first.next_cursor.is_some());
    assert!(first.items[0].id > first.items[1].id);
    assert!(!first.items[0].event_type.is_empty());
    assert!(!first.items[0].occurred_at.is_empty());
    assert!(first.items[0].payload_json.is_object());

    let second = library
        .list_activity(ActivityListRequest {
            limit: 2,
            cursor: first.next_cursor.clone(),
        })
        .expect("second page");
    assert!(!second.items.is_empty());
    let first_ids: Vec<i64> = first.items.iter().map(|e| e.id).collect();
    let second_ids: Vec<i64> = second.items.iter().map(|e| e.id).collect();
    for id in &second_ids {
        assert!(!first_ids.contains(id), "pages must not overlap");
    }
    assert!(second_ids.iter().all(|id| *id < first_ids[1]));

    let again = library
        .list_activity(ActivityListRequest {
            limit: 2,
            cursor: first.next_cursor,
        })
        .expect("replay second page");
    assert_eq!(
        again.items.iter().map(|e| e.id).collect::<Vec<_>>(),
        second_ids
    );
}

#[test]
fn recent_activity_remains_compatibility_wrapper() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "compat-session",
        "captures/a.jsonl",
        "{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"hi\"}\n",
    );

    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("ingest");
    let recent = library.recent_activity(50).expect("recent");
    assert!(recent.iter().any(|e| e.event_type == "capture_recorded"));
    assert!(recent.iter().any(|e| e.event_type == "projection_replaced"));

    let page = library
        .list_activity(ActivityListRequest {
            limit: 50,
            cursor: None,
        })
        .expect("page");
    assert!(page
        .items
        .iter()
        .any(|e| e.event_type == "capture_recorded"));
}

#[test]
fn activity_survives_operational_cleanup_and_is_append_only() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "persist-session",
        "captures/a.jsonl",
        "{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"persist\"}\n",
    );

    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("ingest");
    let before = library
        .list_activity(ActivityListRequest {
            limit: 200,
            cursor: None,
        })
        .expect("before");
    assert!(!before.items.is_empty());
    let before_snapshot: Vec<(i64, String)> = before
        .items
        .iter()
        .map(|e| (e.id, e.event_type.clone()))
        .collect();

    // Operational cleanup must not rewrite or delete Activity Events.
    library
        .repair(RepairOptions::all_documented())
        .expect("repair");
    let ops = library
        .list_operations(OperationsRequest::default())
        .expect("ops");
    assert_eq!(ops.operations_status, "ok");

    let after = library
        .list_activity(ActivityListRequest {
            limit: 200,
            cursor: None,
        })
        .expect("after");
    let after_snapshot: Vec<(i64, String)> = after
        .items
        .iter()
        .map(|e| (e.id, e.event_type.clone()))
        .collect();
    for row in &before_snapshot {
        assert!(
            after_snapshot.contains(row),
            "operational cleanup must preserve Activity row {:?}",
            row
        );
    }

    // Append a new event via curation; prior ids remain unchanged.
    library
        .add_session_tag(distill_library::SessionCurationRequest {
            source_kind: "fixture".into(),
            external_session_id: "persist-session".into(),
            name: "audit-tag".into(),
        })
        .expect("tag");
    let appended = library
        .list_activity(ActivityListRequest {
            limit: 200,
            cursor: None,
        })
        .expect("appended");
    assert!(appended.items.iter().any(|e| e.event_type == "tag_added"));
    for row in &before_snapshot {
        assert!(
            appended
                .items
                .iter()
                .any(|e| e.id == row.0 && e.event_type == row.1),
            "append must not rewrite prior Activity {:?}",
            row
        );
    }
}

#[test]
fn activity_payload_redacts_paths_sql_and_provider_blobs() {
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
              "object_type": "sync_job",
              "object_id": 9,
              "output_path": "/tmp/secret/export.jsonl",
              "outputPath": "/tmp/secret/camel-case-export.jsonl",
              "source_path": "/Users/me/.codex/session.jsonl",
              "sql": "SELECT * FROM captures",
              "payload": {
                "provider_payload": {"raw": "secret"},
                "reason": "cancelled",
                "outputPath": "/tmp/secret/nested.jsonl"
              },
              "nested": {"path": "/var/private", "count": 1}
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
    let event = page
        .items
        .iter()
        .find(|e| e.event_type == "sync_failed")
        .expect("sync_failed");
    let payload = &event.payload_json;
    assert!(payload.get("output_path").is_none());
    assert!(payload.get("outputPath").is_none());
    assert!(payload.get("source_path").is_none());
    assert!(payload.get("sql").is_none());
    assert!(payload.pointer("/payload/provider_payload").is_none());
    assert!(payload.pointer("/payload/outputPath").is_none());
    assert_eq!(
        payload.pointer("/payload/reason").and_then(|v| v.as_str()),
        Some("cancelled")
    );
    assert_eq!(
        payload.pointer("/nested/path").and_then(|v| v.as_str()),
        None
    );
    assert_eq!(
        payload.pointer("/nested/count").and_then(|v| v.as_u64()),
        Some(1)
    );
}

#[test]
fn operations_reports_sync_and_export_without_mutating_activity() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "ops-session",
        "captures/a.jsonl",
        "{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"ops\"}\n",
    );

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("pref");
    let sync = library
        .start_sync(SyncRequest::default(), |_| {})
        .expect("sync");
    assert_eq!(sync.run.status, "completed");

    library
        .toggle_session_label(distill_library::SessionCurationRequest {
            source_kind: "fixture".into(),
            external_session_id: "ops-session".into(),
            name: "train".into(),
        })
        .expect("label");
    let export = library
        .publish_export(ExportDataset::Train, |_| {
            distill_library::ExportProgressControl::Continue
        })
        .expect("export");
    assert_eq!(export.status.as_str(), "published");

    let activity_before = library
        .list_activity(ActivityListRequest {
            limit: 200,
            cursor: None,
        })
        .expect("activity before");
    let activity_count = activity_before.items.len();

    let ops = library
        .list_operations(OperationsRequest {
            sync_limit: 10,
            export_limit: 10,
            sync_cursor: None,
            export_cursor: None,
        })
        .expect("operations");
    assert_eq!(ops.operations_status, "ok");
    assert!(!ops.sync_runs.is_empty());
    assert_eq!(ops.sync_runs[0].id, sync.run.id);
    assert_eq!(ops.sync_runs[0].status, "completed");
    assert!(!ops.exports.is_empty());
    assert_eq!(ops.exports[0].id, export.export_id);
    assert_eq!(ops.exports[0].status, "published");
    assert_eq!(ops.exports[0].dataset, "train");
    // No filesystem paths in export lifecycle summaries.
    let export_json = serde_json::to_value(&ops.exports[0]).expect("export json");
    assert!(export_json.get("output_path").is_none());
    assert!(export_json.get("temp_path").is_none());

    let activity_after = library
        .list_activity(ActivityListRequest {
            limit: 200,
            cursor: None,
        })
        .expect("activity after");
    assert_eq!(
        activity_after.items.len(),
        activity_count,
        "operations must not mutate Activity"
    );
}

#[test]
fn operations_surfaces_warning_failed_cancelled_and_export_lifecycle() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_two_candidate_fixture(&fixture);

    let mut owner = Library::open(&home).expect("owner");
    owner
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("pref");
    let observer_home = home.clone();
    let cancelled = owner
        .start_sync(SyncRequest::default(), |event| {
            if let distill_library::SyncProgress::CandidateStarted { sync_run_id, .. } = event {
                let mut other = Library::open(&observer_home).expect("canceller");
                other
                    .request_sync_cancel(sync_run_id)
                    .expect("cancel request");
            }
        })
        .expect("cancelled sync");
    assert_eq!(cancelled.run.status, "cancelled");
    drop(owner);

    // Seed failed/warning Sync Runs and cancelled/failed export rows.
    let conn = open_home_db(&home);
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO sync_runs (
            status, requested_at, started_at, finished_at, cancel_requested,
            owner_id, heartbeat_at, lease_expires_at, metrics_json,
            error_class, error_message, warning_details_json
         ) VALUES (
            'failed', ?1, ?1, ?1, 0, 'seed', ?1, ?1, '{}',
            'sync_failed', 'seeded failure at /Users/private/distill.db', '[]'
         )",
        params![now],
    )
    .expect("failed sync");
    conn.execute(
        "INSERT INTO sync_runs (
            status, requested_at, started_at, finished_at, cancel_requested,
            owner_id, heartbeat_at, lease_expires_at, metrics_json,
            error_class, error_message, warning_details_json
         ) VALUES (
            'warning', ?1, ?1, ?1, 0, 'seed', ?1, ?1,
            '{\"accepted_captures\":1,\"failed_attempts\":1}',
            NULL, NULL, '[\"sibling source unavailable at /tmp/private-source\"]'
         )",
        params![now],
    )
    .expect("warning sync");
    conn.execute(
        "INSERT INTO exports (
            format_id, dataset, status, created_at, updated_at, record_count,
            eligibility_snapshot_json, error_class, error_message
         ) VALUES (
            'distill-session-jsonl-v1', 'holdout', 'cancelled', ?1, ?1, 0,
            '{}', 'cancelled', 'export cancelled at a safe checkpoint'
         )",
        params![now],
    )
    .expect("cancelled export");
    conn.execute(
        "INSERT INTO exports (
            format_id, dataset, status, created_at, updated_at, record_count,
            eligibility_snapshot_json, error_class, error_message
         ) VALUES (
            'distill-session-jsonl-v1', 'train', 'failed_publish', ?1, ?1, 0,
            '{}', 'export_failed', 'seeded export failure at /private/export.jsonl'
         )",
        params![now],
    )
    .expect("failed export");
    drop(conn);

    let library = Library::open(&home).expect("reopen");
    let ops = library
        .list_operations(OperationsRequest::default())
        .expect("ops");
    let statuses: Vec<&str> = ops.sync_runs.iter().map(|r| r.status.as_str()).collect();
    assert!(statuses.contains(&"cancelled"));
    assert!(statuses.contains(&"failed"));
    assert!(statuses.contains(&"warning"));
    let warning = ops
        .sync_runs
        .iter()
        .find(|r| r.status == "warning")
        .expect("warning run");
    assert!(warning
        .warning_details
        .iter()
        .any(|d| d.contains("unavailable")));
    let failed_sync = ops
        .sync_runs
        .iter()
        .find(|r| r.status == "failed")
        .expect("failed run");
    assert_eq!(
        failed_sync.error_message.as_deref(),
        Some("seeded failure at [redacted]")
    );
    let warning_detail = warning.warning_details.first().expect("warning detail");
    assert_eq!(warning_detail, "sibling source unavailable at [redacted]");

    let export_statuses: Vec<&str> = ops.exports.iter().map(|e| e.status.as_str()).collect();
    assert!(export_statuses.contains(&"cancelled"));
    assert!(export_statuses.contains(&"failed_publish"));
    let failed_export = ops
        .exports
        .iter()
        .find(|export| export.status == "failed_publish")
        .expect("failed export");
    assert_eq!(
        failed_export.error_message.as_deref(),
        Some("seeded export failure at [redacted]")
    );
}

#[test]
fn operations_pagination_is_deterministic() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let mut library = Library::open(&home).expect("open");
    drop(library);

    let conn = open_home_db(&home);
    let now = chrono::Utc::now().to_rfc3339();
    for i in 0..3 {
        conn.execute(
            "INSERT INTO sync_runs (
                status, requested_at, started_at, finished_at, cancel_requested,
                owner_id, heartbeat_at, lease_expires_at, metrics_json,
                error_class, error_message, warning_details_json
             ) VALUES (
                'completed', ?1, ?1, ?1, 0, ?2, ?1, ?1, '{}', NULL, NULL, '[]'
             )",
            params![now, format!("owner-{i}")],
        )
        .expect("sync row");
        conn.execute(
            "INSERT INTO exports (
                format_id, dataset, status, created_at, updated_at, record_count,
                eligibility_snapshot_json, output_path, sha256, byte_size
             ) VALUES (
                'distill-session-jsonl-v1', 'train', 'published', ?1, ?1, 0, '{}',
                'exports/train.jsonl', 'abc', 1
             )",
            params![now],
        )
        .expect("export row");
    }
    drop(conn);

    library = Library::open(&home).expect("reopen");
    let first = library
        .list_operations(OperationsRequest {
            sync_limit: 1,
            export_limit: 1,
            sync_cursor: None,
            export_cursor: None,
        })
        .expect("first");
    assert_eq!(first.sync_runs.len(), 1);
    assert_eq!(first.exports.len(), 1);
    assert!(first.next_sync_cursor.is_some());
    assert!(first.next_export_cursor.is_some());

    let second = library
        .list_operations(OperationsRequest {
            sync_limit: 1,
            export_limit: 1,
            sync_cursor: first.next_sync_cursor.clone(),
            export_cursor: first.next_export_cursor.clone(),
        })
        .expect("second");
    assert_eq!(second.sync_runs.len(), 1);
    assert_eq!(second.exports.len(), 1);
    assert_ne!(first.sync_runs[0].id, second.sync_runs[0].id);
    assert_ne!(first.exports[0].id, second.exports[0].id);
    assert!(second.sync_runs[0].id < first.sync_runs[0].id);
    assert!(second.exports[0].id < first.exports[0].id);
}

#[test]
fn activity_and_operations_reject_invalid_cursors() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let library = Library::open(&home).expect("open");

    let activity_error = library
        .list_activity(ActivityListRequest {
            limit: 1,
            cursor: Some("not-a-cursor".into()),
        })
        .expect_err("invalid Activity cursor");
    assert!(matches!(
        activity_error,
        LibraryError::InvalidArgument(message) if message.contains("activity cursor")
    ));

    let operations_error = library
        .list_operations(OperationsRequest {
            sync_limit: 1,
            export_limit: 1,
            sync_cursor: Some("v1\u{1f}export\u{1f}1".into()),
            export_cursor: None,
        })
        .expect_err("wrong Operations cursor kind");
    assert!(matches!(
        operations_error,
        LibraryError::InvalidArgument(message) if message.contains("sync operations cursor")
    ));
}
