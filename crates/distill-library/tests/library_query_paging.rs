//! Library query paging contracts for issue #23.
//!
//! Proves Unicode/quoted-AND/punctuation/zero-token search, current-projection FTS,
//! deterministic keyset cursors, lane ∩ manual-origin filtering, and bounded detail pages
//! over real temporary Distill homes and Fixture ingest.

use std::fs;
use std::path::{Path, PathBuf};

use distill_library::{
    derive_workflow_state, Library, SessionDetailRequest, SessionListRequest, WorkflowLane,
    WorkflowState,
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
 * Open the Distill SQLite database under a home for SQL-seeded curation rows.
 */
fn open_home_db(home: &Path) -> Connection {
    Connection::open(home.join("distill.db")).expect("open distill.db")
}

/**
 * Assign a manual label to a session by external identity.
 */
fn assign_manual_label(home: &Path, external_session_id: &str, label_name: &str) {
    let conn = open_home_db(home);
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
        .expect("label catalog");
    conn.execute(
        "INSERT INTO label_assignments (object_type, object_id, label_id, origin, created_at)
         VALUES ('session', ?1, ?2, 'manual', ?3)",
        params![session_id, label_id, chrono::Utc::now().to_rfc3339()],
    )
    .expect("assign label");
}

/**
 * Assign a non-manual label that query read models must ignore.
 */
fn assign_non_manual_label(home: &Path, external_session_id: &str, label_name: &str) {
    let conn = open_home_db(home);
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
        .expect("label catalog");
    conn.execute(
        "INSERT INTO label_assignments (object_type, object_id, label_id, origin, created_at)
         VALUES ('session', ?1, ?2, 'system', ?3)",
        params![session_id, label_id, chrono::Utc::now().to_rfc3339()],
    )
    .expect("assign non-manual label");
}

/**
 * Assign a manual tag to a session.
 */
fn assign_manual_tag(home: &Path, external_session_id: &str, tag_name: &str) {
    let conn = open_home_db(home);
    let session_id: i64 = conn
        .query_row(
            "SELECT id FROM sessions WHERE external_session_id = ?1",
            [external_session_id],
            |row| row.get(0),
        )
        .expect("session");
    conn.execute(
        "INSERT INTO tags (name, kind, created_at) VALUES (?1, 'general', ?2)
         ON CONFLICT(name) DO NOTHING",
        params![tag_name, chrono::Utc::now().to_rfc3339()],
    )
    .expect("insert tag");
    let tag_id: i64 = conn
        .query_row("SELECT id FROM tags WHERE name = ?1", [tag_name], |row| {
            row.get(0)
        })
        .expect("tag id");
    conn.execute(
        "INSERT INTO tag_assignments (object_type, object_id, tag_id, origin, created_at)
         VALUES ('session', ?1, ?2, 'manual', ?3)",
        params![session_id, tag_id, chrono::Utc::now().to_rfc3339()],
    )
    .expect("assign tag");
}

/**
 * Force identical updated_at values so id DESC tie-breaking is observable.
 */
fn force_updated_at(home: &Path, external_session_id: &str, updated_at: &str) {
    let conn = open_home_db(home);
    conn.execute(
        "UPDATE sessions SET updated_at = ?1 WHERE external_session_id = ?2",
        params![updated_at, external_session_id],
    )
    .expect("force updated_at");
}

#[test]
fn derive_workflow_state_priority_matrix() {
    assert_eq!(
        derive_workflow_state(["exclude"]),
        WorkflowState::NeedsReview
    );
    assert_eq!(
        derive_workflow_state(["sensitive", "train"]),
        WorkflowState::NeedsReview
    );
    assert_eq!(
        derive_workflow_state(["train", "holdout"]),
        WorkflowState::NeedsReview
    );
    assert_eq!(derive_workflow_state(["train"]), WorkflowState::TrainReady);
    assert_eq!(
        derive_workflow_state(["holdout"]),
        WorkflowState::HoldoutReady
    );
    assert_eq!(derive_workflow_state(["favorite"]), WorkflowState::Favorite);
    assert_eq!(
        derive_workflow_state(["favorite", "train"]),
        WorkflowState::TrainReady
    );
    assert_eq!(
        derive_workflow_state::<[&str; 0], &str>([]),
        WorkflowState::Neutral
    );
}

#[test]
fn search_unicode_quoted_and_punctuation_and_zero_token() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "search-unicode",
        "captures/a.jsonl",
        concat!(
            r#"{"record_type":"session_meta","title":"analytics-regression","project_path":"/proj/βeta","summary":"s"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"analytics-regression: \"beta\" env café"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"ok"}"#,
            "\n",
        ),
    );

    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("ingest");

    fs::write(
        fixture.join("captures/b.jsonl"),
        concat!(
            r#"{"record_type":"session_meta","title":"partial"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"analytics-regression only"}"#,
            "\n",
        ),
    )
    .expect("write partial capture");
    fs::write(
        fixture.join("distill.fixture.json"),
        r#"{
  "version": 1,
  "captures": [
    {"id":"a","kind":"file","relative_path":"captures/a.jsonl","external_session_id":"search-unicode","title":"search-unicode"},
    {"id":"b","kind":"file","relative_path":"captures/b.jsonl","external_session_id":"search-partial","title":"search-partial"}
  ]
}"#,
    )
    .expect("write second manifest");
    library
        .ingest_fixture(&fixture)
        .expect("ingest second capture");

    let unicode = library
        .list_sessions(SessionListRequest {
            query: Some("café".into()),
            lane: WorkflowLane::All,
            limit: 20,
            cursor: None,
        })
        .expect("unicode search");
    assert_eq!(unicode.items.len(), 1);
    assert_eq!(unicode.items[0].external_session_id, "search-unicode");

    let quoted = library
        .list_sessions(SessionListRequest {
            query: Some(r#"analytics-regression: "beta" env"#.into()),
            lane: WorkflowLane::All,
            limit: 20,
            cursor: None,
        })
        .expect("quoted and");
    assert_eq!(quoted.items.len(), 1);
    assert_eq!(quoted.items[0].external_session_id, "search-unicode");

    let title_hits = library
        .list_sessions(SessionListRequest {
            query: Some("analytics-regression".into()),
            lane: WorkflowLane::All,
            limit: 20,
            cursor: None,
        })
        .expect("title search");
    assert_eq!(title_hits.items.len(), 2);
    for query in ["βeta", "user"] {
        let field_hit = library
            .list_sessions(SessionListRequest {
                query: Some(query.into()),
                lane: WorkflowLane::All,
                limit: 20,
                cursor: None,
            })
            .expect("field search");
        assert_eq!(field_hit.items.len(), 1, "query {query:?}");
    }

    let punct = library
        .list_sessions(SessionListRequest {
            query: Some("!!! /// ???".into()),
            lane: WorkflowLane::All,
            limit: 20,
            cursor: None,
        })
        .expect("zero token");
    assert!(punct.items.is_empty());
    assert!(punct.next_cursor.is_none());

    let hits = library
        .search(r#"foo/bar baz?"#, 20)
        .expect("legacy search");
    // Tokens become foo, bar, baz — no matching content, empty is fine.
    assert!(hits.is_empty() || hits.iter().all(|hit| !hit.text.is_empty()));
}

#[test]
fn search_excludes_superseded_projection_text() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");
    write_fixture(
        &fixture,
        "proj-replace",
        "captures/old.jsonl",
        concat!(
            r#"{"record_type":"session_meta","title":"replace"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"obsolete unique token alphazero"}"#,
            "\n",
        ),
    );
    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("first ingest");

    write_fixture(
        &fixture,
        "proj-replace",
        "captures/old.jsonl",
        concat!(
            r#"{"record_type":"session_meta","title":"replace"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"current unique token betamax"}"#,
            "\n",
        ),
    );
    library.ingest_fixture(&fixture).expect("second ingest");

    let stale = library
        .list_sessions(SessionListRequest {
            query: Some("alphazero".into()),
            lane: WorkflowLane::All,
            limit: 20,
            cursor: None,
        })
        .expect("stale");
    assert!(stale.items.is_empty(), "superseded text must leave FTS");

    let current = library
        .list_sessions(SessionListRequest {
            query: Some("betamax".into()),
            lane: WorkflowLane::All,
            limit: 20,
            cursor: None,
        })
        .expect("current");
    assert_eq!(current.items.len(), 1);
}

#[test]
fn list_cursor_traversal_is_deterministic_without_duplicates() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let stamp = "2026-01-01T00:00:00Z";

    for (idx, session_id) in ["tie-a", "tie-b", "tie-c", "tie-d"].iter().enumerate() {
        let fixture = temp.path().join(format!("fixture-{idx}"));
        fs::create_dir_all(&fixture).expect("fixture");
        write_fixture(
            &fixture,
            session_id,
            &format!("captures/{session_id}.jsonl"),
            &format!(
                "{}\n{{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"token {session_id}\"}}\n",
                r#"{"record_type":"session_meta","title":"tie"}"#,
            ),
        );
        let mut library = Library::open(&home).expect("open");
        library.ingest_fixture(&fixture).expect("ingest");
        force_updated_at(&home, session_id, stamp);
    }

    let library = Library::open(&home).expect("reopen");
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = library
            .list_sessions(SessionListRequest {
                query: None,
                lane: WorkflowLane::All,
                limit: 2,
                cursor,
            })
            .expect("page");
        for item in &page.items {
            seen.push(item.external_session_id.clone());
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    assert_eq!(seen.len(), 4, "no omissions");
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 4, "no duplicates");

    // Same updated_at → deterministic id DESC: higher session row ids first.
    let first = library
        .list_sessions(SessionListRequest {
            query: None,
            lane: WorkflowLane::All,
            limit: 4,
            cursor: None,
        })
        .expect("ordered");
    let ids: Vec<i64> = first.items.iter().map(|item| item.id).collect();
    let mut sorted = ids.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(ids, sorted, "tie order must be id DESC");
}

#[test]
fn lane_intersection_and_manual_origin_filtering() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");

    for (session_id, body_token) in [
        ("lane-train", "alpha"),
        ("lane-holdout", "alpha"),
        ("lane-review", "alpha"),
        ("lane-fav", "alpha"),
        ("lane-fav-train", "alpha"),
        ("lane-conflict", "alpha"),
        ("lane-system-only", "alpha"),
    ] {
        let fixture = temp.path().join(session_id);
        fs::create_dir_all(&fixture).expect("fixture");
        write_fixture(
            &fixture,
            session_id,
            &format!("captures/{session_id}.jsonl"),
            &format!(
                "{}\n{{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"{body_token} {session_id}\"}}\n",
                r#"{"record_type":"session_meta","title":"lane"}"#,
            ),
        );
        let mut library = Library::open(&home).expect("open");
        library.ingest_fixture(&fixture).expect("ingest");
    }

    assign_manual_label(&home, "lane-train", "train");
    assign_manual_label(&home, "lane-holdout", "holdout");
    assign_manual_label(&home, "lane-review", "exclude");
    assign_manual_label(&home, "lane-fav", "favorite");
    assign_manual_label(&home, "lane-fav-train", "favorite");
    assign_manual_label(&home, "lane-fav-train", "train");
    assign_manual_label(&home, "lane-conflict", "train");
    assign_manual_label(&home, "lane-conflict", "holdout");
    assign_non_manual_label(&home, "lane-system-only", "train");

    let library = Library::open(&home).expect("reopen");

    let train = library
        .list_sessions(SessionListRequest {
            query: Some("alpha".into()),
            lane: WorkflowLane::TrainReady,
            limit: 20,
            cursor: None,
        })
        .expect("train lane");
    let train_ids: Vec<_> = train
        .items
        .iter()
        .map(|item| item.external_session_id.as_str())
        .collect();
    assert!(train_ids.contains(&"lane-train"));
    assert!(train_ids.contains(&"lane-fav-train"));
    assert!(!train_ids.contains(&"lane-system-only"));
    assert!(train
        .items
        .iter()
        .all(|item| item.workflow_state == WorkflowState::TrainReady));
    assert!(!train
        .items
        .iter()
        .any(|item| item.external_session_id == "lane-conflict"));

    let holdout = library
        .list_sessions(SessionListRequest {
            query: None,
            lane: WorkflowLane::HoldoutReady,
            limit: 20,
            cursor: None,
        })
        .expect("holdout lane");
    assert!(holdout
        .items
        .iter()
        .any(|item| item.external_session_id == "lane-holdout"));
    assert!(!holdout
        .items
        .iter()
        .any(|item| item.external_session_id == "lane-conflict"));

    let favorites = library
        .list_sessions(SessionListRequest {
            query: None,
            lane: WorkflowLane::Favorites,
            limit: 20,
            cursor: None,
        })
        .expect("favorites");
    let fav_ids: Vec<_> = favorites
        .items
        .iter()
        .map(|item| item.external_session_id.as_str())
        .collect();
    assert!(fav_ids.contains(&"lane-fav"));
    assert!(fav_ids.contains(&"lane-fav-train"));
    let fav_train = favorites
        .items
        .iter()
        .find(|item| item.external_session_id == "lane-fav-train")
        .expect("fav+train");
    assert_eq!(fav_train.workflow_state, WorkflowState::TrainReady);

    let review = library
        .list_sessions(SessionListRequest {
            query: None,
            lane: WorkflowLane::NeedsReview,
            limit: 20,
            cursor: None,
        })
        .expect("review");
    assert!(review
        .items
        .iter()
        .any(|item| item.external_session_id == "lane-review"));
    assert!(review
        .items
        .iter()
        .any(|item| item.external_session_id == "lane-conflict"));
}

#[test]
fn session_detail_fields_tags_labels_and_cursors() {
    let temp = TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    fs::create_dir_all(&fixture).expect("fixture");

    let mut body = String::from(concat!(
        r#"{"record_type":"session_meta","title":"Detail Session","summary":"narrative","project_path":"/workspace/demo","source_url":"https://example.test/s","started_at":"2026-01-02T00:00:00Z","updated_at":"2026-01-03T00:00:00Z","metadata":{"k":"v"}}"#,
        "\n",
    ));
    for idx in 0..5 {
        body.push_str(&format!(
            "{{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"message-{idx}\"}}\n"
        ));
    }
    for idx in 0..4 {
        body.push_str(&format!(
            "{{\"record_type\":\"tool_use\",\"text\":\"artifact-{idx}\"}}\n"
        ));
    }
    write_fixture(&fixture, "detail-session", "captures/detail.jsonl", &body);

    let mut library = Library::open(&home).expect("open");
    library.ingest_fixture(&fixture).expect("ingest");
    assign_manual_label(&home, "detail-session", "train");
    assign_manual_label(&home, "detail-session", "favorite");
    assign_manual_tag(&home, "detail-session", "research");
    assign_non_manual_label(&home, "detail-session", "holdout");

    let library = Library::open(&home).expect("reopen");
    let first = library
        .session_detail(SessionDetailRequest {
            source_kind: "fixture".into(),
            external_session_id: "detail-session".into(),
            message_limit: 2,
            artifact_limit: 2,
            message_cursor: None,
            artifact_cursor: None,
        })
        .expect("detail")
        .expect("present");

    assert_eq!(first.project_path.as_deref(), Some("/workspace/demo"));
    assert_eq!(first.source_url.as_deref(), Some("https://example.test/s"));
    assert_eq!(first.projection_summary.as_deref(), Some("narrative"));
    assert_eq!(first.started_at.as_deref(), Some("2026-01-02T00:00:00Z"));
    assert_eq!(first.updated_at.as_deref(), Some("2026-01-03T00:00:00Z"));
    assert_eq!(first.raw_capture_count, 1);
    assert_eq!(first.summary.accepted_capture_count, 1);
    assert_eq!(first.summary.normalization_attempt_count, 1);
    assert_eq!(first.summary.successful_projection_generation, 1);
    assert!(first.metadata_json.contains("\"k\":\"v\""));
    assert_eq!(first.workflow_state, WorkflowState::TrainReady);
    assert_eq!(first.labels.len(), 2);
    assert!(first.labels.iter().all(|label| label.origin == "manual"));
    assert!(!first.labels.iter().any(|label| label.name == "holdout"));
    assert_eq!(first.tags.len(), 1);
    assert_eq!(first.tags[0].name, "research");
    assert_eq!(first.messages.len(), 2);
    assert_eq!(first.artifacts.len(), 2);
    assert!(first.next_message_cursor.is_some());
    assert!(first.next_artifact_cursor.is_some());

    let second = library
        .session_detail(SessionDetailRequest {
            source_kind: "fixture".into(),
            external_session_id: "detail-session".into(),
            message_limit: 2,
            artifact_limit: 2,
            message_cursor: first.next_message_cursor.clone(),
            artifact_cursor: first.next_artifact_cursor.clone(),
        })
        .expect("page 2")
        .expect("present");
    assert_eq!(second.messages.len(), 2);
    assert_ne!(
        first.messages[0].id, second.messages[0].id,
        "message pages must advance"
    );
    let message_ids: Vec<i64> = first
        .messages
        .iter()
        .chain(second.messages.iter())
        .map(|message| message.id)
        .collect();
    let mut unique = message_ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), message_ids.len());

    let compat = library
        .session_slice("fixture", "detail-session", 20, 20)
        .expect("slice")
        .expect("present");
    assert_eq!(compat.messages.len(), 5);
    assert_eq!(compat.project_path.as_deref(), Some("/workspace/demo"));
    assert_eq!(compat.workflow_state, WorkflowState::TrainReady);
}
