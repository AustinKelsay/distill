//! Library curation mutation contracts for issue #24.
//!
//! Proves name normalization, duplicate/missing no-ops, Activity counts,
//! dataset exclusivity, modifier preservation, workflow priority, and manual
//! origin over real temporary Distill homes and Fixture ingest.

use std::fs;
use std::path::{Path, PathBuf};

use distill_library::{Library, SessionCurationRequest, SessionIdentity, WorkflowState};
use rusqlite::Connection;
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
 * Open a temp home, ingest one Fixture session, and return `(home, library, identity)`.
 */
fn seeded_library(session_id: &str) -> (TempDir, Library, SessionIdentity) {
    let home = TempDir::new().expect("temp home");
    let fixture = TempDir::new().expect("temp fixture");
    write_fixture(
        fixture.path(),
        session_id,
        "sessions/one.jsonl",
        concat!(
            r#"{"record_type":"session_meta","title":"Curation","summary":"curation"}"#,
            "\n",
            r#"{"record_type":"message","role":"user","text":"hello curation"}"#,
            "\n",
            r#"{"record_type":"message","role":"assistant","text":"tagged"}"#,
            "\n",
        ),
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

fn curation_activity_count(home: &Path, event_type: &str) -> i64 {
    let conn = Connection::open(home.join("distill.db")).expect("open distill.db");
    conn.query_row(
        "SELECT COUNT(*) FROM activity_events WHERE event_type = ?1",
        [event_type],
        |row| row.get(0),
    )
    .expect("count activity")
}

fn all_curation_activity_count(home: &Path) -> i64 {
    let conn = Connection::open(home.join("distill.db")).expect("open distill.db");
    conn.query_row(
        "SELECT COUNT(*) FROM activity_events
         WHERE event_type IN ('tag_added', 'tag_removed', 'label_toggled')",
        [],
        |row| row.get(0),
    )
    .expect("count curation activity")
}

fn label_names(result: &distill_library::CurationMutationResult) -> Vec<String> {
    result
        .labels
        .iter()
        .map(|label| label.name.clone())
        .collect()
}

fn tag_names(result: &distill_library::CurationMutationResult) -> Vec<String> {
    result.tags.iter().map(|tag| tag.name.clone()).collect()
}

#[test]
fn tag_add_normalizes_unicode_lowercase_and_emits_activity() {
    let (home, mut library, identity) = seeded_library("curation-normalize");
    let before = all_curation_activity_count(home.path());

    let result = library
        .add_session_tag(request(&identity, "  Research "))
        .expect("add tag");
    assert!(result.changed);
    assert_eq!(result.identity, identity);
    assert_eq!(tag_names(&result), vec!["research"]);
    assert!(result.tags.iter().all(|tag| tag.origin == "manual"));
    assert!(result.tags.iter().all(|tag| tag.kind == "manual"));
    assert_eq!(result.workflow_state, WorkflowState::Neutral);
    assert_eq!(
        curation_activity_count(home.path(), "tag_added"),
        before + 1
    );

    let unicode = library
        .add_session_tag(request(&identity, "  ΑΒΓ "))
        .expect("add unicode tag");
    assert!(unicode.changed);
    assert_eq!(tag_names(&unicode), vec!["research", "αβγ"]);
}

#[test]
fn tag_duplicate_blank_missing_session_and_missing_remove_are_noops() {
    let (home, mut library, identity) = seeded_library("curation-tag-noop");
    library
        .add_session_tag(request(&identity, "topic"))
        .expect("seed tag");
    let baseline = all_curation_activity_count(home.path());

    let duplicate = library
        .add_session_tag(request(&identity, "  TOPIC "))
        .expect("duplicate add");
    assert!(!duplicate.changed);
    assert_eq!(tag_names(&duplicate), vec!["topic"]);
    assert_eq!(all_curation_activity_count(home.path()), baseline);

    let blank = library
        .add_session_tag(request(&identity, "   "))
        .expect("blank add");
    assert!(!blank.changed);
    assert_eq!(tag_names(&blank), vec!["topic"]);
    assert_eq!(all_curation_activity_count(home.path()), baseline);

    let missing_remove = library
        .remove_session_tag(request(&identity, "absent"))
        .expect("missing remove");
    assert!(!missing_remove.changed);
    assert_eq!(tag_names(&missing_remove), vec!["topic"]);
    assert_eq!(all_curation_activity_count(home.path()), baseline);

    let missing_session = library
        .add_session_tag(SessionCurationRequest {
            source_kind: "fixture".into(),
            external_session_id: "does-not-exist".into(),
            name: "topic".into(),
        })
        .expect("missing session");
    assert!(!missing_session.changed);
    assert!(missing_session.tags.is_empty());
    assert!(missing_session.labels.is_empty());
    assert_eq!(missing_session.workflow_state, WorkflowState::Neutral);
    assert_eq!(all_curation_activity_count(home.path()), baseline);
}

#[test]
fn tag_remove_emits_activity_and_clears_assignment() {
    let (home, mut library, identity) = seeded_library("curation-tag-remove");
    library
        .add_session_tag(request(&identity, "ephemeral"))
        .expect("add");
    let before_removed = curation_activity_count(home.path(), "tag_removed");

    let removed = library
        .remove_session_tag(request(&identity, "  Ephemeral "))
        .expect("remove");
    assert!(removed.changed);
    assert!(removed.tags.is_empty());
    assert_eq!(
        curation_activity_count(home.path(), "tag_removed"),
        before_removed + 1
    );
}

#[test]
fn label_unknown_blank_and_duplicate_toggle_off_are_noops_or_single_events() {
    let (home, mut library, identity) = seeded_library("curation-label-noop");
    let baseline = all_curation_activity_count(home.path());

    let unknown = library
        .toggle_session_label(request(&identity, "not-a-real-label"))
        .expect("unknown");
    assert!(!unknown.changed);
    assert!(unknown.labels.is_empty());
    assert_eq!(all_curation_activity_count(home.path()), baseline);

    let blank = library
        .toggle_session_label(request(&identity, "\t"))
        .expect("blank");
    assert!(!blank.changed);
    assert_eq!(all_curation_activity_count(home.path()), baseline);

    let enabled = library
        .toggle_session_label(request(&identity, "  Favorite "))
        .expect("enable favorite");
    assert!(enabled.changed);
    assert_eq!(label_names(&enabled), vec!["favorite"]);
    assert_eq!(enabled.workflow_state, WorkflowState::Favorite);
    assert_eq!(
        curation_activity_count(home.path(), "label_toggled"),
        baseline + 1
    );

    let disabled = library
        .toggle_session_label(request(&identity, "favorite"))
        .expect("disable favorite");
    assert!(disabled.changed);
    assert!(disabled.labels.is_empty());
    assert_eq!(disabled.workflow_state, WorkflowState::Neutral);
    assert_eq!(
        curation_activity_count(home.path(), "label_toggled"),
        baseline + 2
    );
}

#[test]
fn dataset_label_exclusivity_emits_removal_and_enable_events() {
    let (home, mut library, identity) = seeded_library("curation-dataset");
    library
        .toggle_session_label(request(&identity, "train"))
        .expect("enable train");
    let before = curation_activity_count(home.path(), "label_toggled");

    let switched = library
        .toggle_session_label(request(&identity, "holdout"))
        .expect("switch to holdout");
    assert!(switched.changed);
    assert_eq!(label_names(&switched), vec!["holdout"]);
    assert_eq!(switched.workflow_state, WorkflowState::HoldoutReady);
    // One disable (train) + one enable (holdout).
    assert_eq!(
        curation_activity_count(home.path(), "label_toggled"),
        before + 2
    );

    let to_exclude = library
        .toggle_session_label(request(&identity, "exclude"))
        .expect("switch to exclude");
    assert_eq!(label_names(&to_exclude), vec!["exclude"]);
    assert_eq!(to_exclude.workflow_state, WorkflowState::NeedsReview);
}

#[test]
fn dataset_toggle_preserves_sensitive_and_favorite_modifiers() {
    let (_home, mut library, identity) = seeded_library("curation-modifiers");
    library
        .toggle_session_label(request(&identity, "favorite"))
        .expect("favorite");
    library
        .toggle_session_label(request(&identity, "sensitive"))
        .expect("sensitive");
    library
        .toggle_session_label(request(&identity, "train"))
        .expect("train");

    let switched = library
        .toggle_session_label(request(&identity, "holdout"))
        .expect("holdout");
    assert_eq!(
        label_names(&switched),
        vec!["favorite", "holdout", "sensitive"]
    );
    assert!(switched.labels.iter().all(|label| label.origin == "manual"));
    // sensitive keeps workflow in needs_review even with holdout present.
    assert_eq!(switched.workflow_state, WorkflowState::NeedsReview);
}

#[test]
fn workflow_priority_and_manual_origin_on_mutation_results() {
    let (_home, mut library, identity) = seeded_library("curation-workflow");

    let train = library
        .toggle_session_label(request(&identity, "train"))
        .expect("train");
    assert_eq!(train.workflow_state, WorkflowState::TrainReady);
    assert!(train.labels.iter().all(|label| label.origin == "manual"));

    let with_favorite = library
        .toggle_session_label(request(&identity, "favorite"))
        .expect("favorite");
    assert_eq!(with_favorite.workflow_state, WorkflowState::TrainReady);

    let with_sensitive = library
        .toggle_session_label(request(&identity, "sensitive"))
        .expect("sensitive");
    assert_eq!(with_sensitive.workflow_state, WorkflowState::NeedsReview);

    library
        .toggle_session_label(request(&identity, "sensitive"))
        .expect("clear sensitive");
    library
        .toggle_session_label(request(&identity, "train"))
        .expect("clear train");
    let favorite_only = library
        .session_detail(distill_library::SessionDetailRequest {
            source_kind: identity.source_kind.clone(),
            external_session_id: identity.external_session_id.clone(),
            message_limit: 20,
            artifact_limit: 20,
            message_cursor: None,
            artifact_cursor: None,
        })
        .expect("detail")
        .expect("session");
    assert_eq!(favorite_only.workflow_state, WorkflowState::Favorite);
    assert!(favorite_only
        .labels
        .iter()
        .all(|label| label.origin == "manual"));
}

#[test]
fn tag_and_label_mutations_are_atomic_with_activity() {
    let (home, mut library, identity) = seeded_library("curation-atomic");
    let ingest_activity = {
        let conn = Connection::open(home.path().join("distill.db")).expect("db");
        conn.query_row("SELECT COUNT(*) FROM activity_events", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("count")
    };

    let added = library
        .add_session_tag(request(&identity, "atomic"))
        .expect("add");
    assert!(added.changed);
    assert_eq!(curation_activity_count(home.path(), "tag_added"), 1);

    let labeled = library
        .toggle_session_label(request(&identity, "train"))
        .expect("label");
    assert!(labeled.changed);
    assert_eq!(curation_activity_count(home.path(), "label_toggled"), 1);

    let conn = Connection::open(home.path().join("distill.db")).expect("db");
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM activity_events", [], |row| row.get(0))
        .expect("total");
    assert_eq!(total, ingest_activity + 2);
}

#[test]
fn non_manual_label_collision_is_a_noop_without_removing_manual_state() {
    let (home, mut library, identity) = seeded_library("curation-derived-collision");
    library
        .toggle_session_label(request(&identity, "holdout"))
        .expect("holdout");
    let baseline = all_curation_activity_count(home.path());

    let conn = Connection::open(home.path().join("distill.db")).expect("open distill.db");
    let session_id: i64 = conn
        .query_row(
            "SELECT id FROM sessions WHERE source_kind = 'fixture' AND external_session_id = ?1",
            [&identity.external_session_id],
            |row| row.get(0),
        )
        .expect("session id");
    let label_id: i64 = conn
        .query_row("SELECT id FROM labels WHERE name = 'train'", [], |row| {
            row.get(0)
        })
        .expect("train label");
    conn.execute(
        "INSERT INTO label_assignments (object_type, object_id, label_id, origin, created_at)
         VALUES ('session', ?1, ?2, 'derived', '2026-01-01T00:00:00Z')",
        rusqlite::params![session_id, label_id],
    )
    .expect("derived assignment");

    let result = library
        .toggle_session_label(request(&identity, "train"))
        .expect("collision no-op");
    assert!(!result.changed);
    assert_eq!(label_names(&result), vec!["holdout"]);
    assert_eq!(result.workflow_state, WorkflowState::HoldoutReady);
    assert_eq!(all_curation_activity_count(home.path()), baseline);
}
