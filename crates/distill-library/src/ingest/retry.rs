//! Same-Capture renormalization from Distill-owned replay bytes.

use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::adapter::{
    parse_replay_bytes, CaptureCandidate, CaptureSnapshot, ParserRegistry, SourceKind,
};
use crate::error::{LibraryError, LibraryResult};
use crate::storage::{read_capture_bytes, ContentRef, DistillPaths};
use crate::types::RenormalizeReport;

use super::attempt::{
    fail_attempt, insert_attempt, publish_projection,
    refresh_session_counters_after_failed_attempt, PARSE_FAILURE_MESSAGE,
    PROJECTION_FAILURE_MESSAGE,
};

/// Capture metadata needed to rebuild a Candidate for Distill-owned replay.
struct CaptureRetryRow {
    source_kind: String,
    source_path: String,
    external_session_id: Option<String>,
    media_type: String,
    content: ContentRef,
}

/**
 * Re-normalize an accepted Capture from Distill-owned bytes with a registered parser.
 *
 * Does not create a new Capture. Parser identity comes from the Library-owned
 * registry for the Capture's persisted Source kind. Replay never rereads a Source
 * root and never reruns an OpenCode subprocess.
 *
 * Parameters:
 * - `conn`: Library SQLite connection.
 * - `paths`: Distill home paths for content replay.
 * - `capture_id`: Accepted Capture to retry.
 * - `registry`: Library-owned parser registry for closed v1 Source kinds.
 */
pub fn renormalize_capture(
    conn: &mut Connection,
    paths: &DistillPaths,
    capture_id: i64,
    registry: &ParserRegistry,
) -> LibraryResult<RenormalizeReport> {
    let capture = load_capture_for_retry(conn, capture_id)?;
    let Some(source_kind) = SourceKind::parse(&capture.source_kind) else {
        return Err(LibraryError::UnknownSourceKind {
            kind: capture.source_kind,
        });
    };
    let parser = registry.get(source_kind).clone();

    let bytes = read_capture_bytes(paths.home.as_path(), &capture.content, capture_id)?;
    let candidate = CaptureCandidate {
        source_kind,
        source_path: capture.source_path.clone(),
        absolute_path: None,
        external_session_id: capture.external_session_id.clone(),
        title: None,
        is_virtual: true,
        // Replay must not treat Capture bytes as OpenCode discovery metadata.
        virtual_bytes: None,
        media_type: capture.media_type.clone(),
    };
    let snapshot = CaptureSnapshot {
        byte_size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(&bytes)),
        bytes,
        media_type: capture.media_type.clone(),
        source_modified_at: None,
    };

    match parse_replay_bytes(source_kind, &candidate, &snapshot.bytes, &parser.version) {
        Ok(parsed) => {
            let attempt_id = insert_attempt(
                conn,
                capture_id,
                &parser.id,
                &parser.version,
                "pending",
                None,
                None,
            )?;
            match publish_projection(conn, capture_id, attempt_id, &candidate, &parsed) {
                Ok(()) => Ok(RenormalizeReport {
                    capture_id,
                    attempt_id,
                    outcome: "succeeded".into(),
                    parser_id: parser.id.clone(),
                    parser_version: parser.version.clone(),
                }),
                Err(_err) => {
                    fail_attempt(
                        conn,
                        attempt_id,
                        "projection_failed",
                        PROJECTION_FAILURE_MESSAGE,
                    )?;
                    refresh_session_counters_after_failed_attempt(
                        conn,
                        candidate.source_kind.as_str(),
                        Some(parsed.external_session_id.as_str()),
                    )?;
                    Ok(RenormalizeReport {
                        capture_id,
                        attempt_id,
                        outcome: "failed".into(),
                        parser_id: parser.id.clone(),
                        parser_version: parser.version.clone(),
                    })
                }
            }
        }
        Err(_err) => {
            let attempt_id = insert_attempt(
                conn,
                capture_id,
                &parser.id,
                &parser.version,
                "failed",
                Some("parse_failed"),
                Some(PARSE_FAILURE_MESSAGE),
            )?;
            refresh_session_counters_after_failed_attempt(
                conn,
                candidate.source_kind.as_str(),
                candidate.external_session_id.as_deref(),
            )?;
            Ok(RenormalizeReport {
                capture_id,
                attempt_id,
                outcome: "failed".into(),
                parser_id: parser.id.clone(),
                parser_version: parser.version.clone(),
            })
        }
    }
}

/**
 * Load Capture identity and Distill-owned content for renormalization.
 */
fn load_capture_for_retry(conn: &Connection, capture_id: i64) -> LibraryResult<CaptureRetryRow> {
    let row = conn
        .query_row(
            "SELECT source_kind, source_path, external_session_id, media_type,
                    content_kind, sha256, byte_size, inline_text, blob_path
             FROM captures WHERE id = ?1",
            [capture_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| LibraryError::NotFound(format!("capture {capture_id}")))?;

    let (
        source_kind,
        source_path,
        external_session_id,
        media_type,
        content_kind,
        sha256,
        byte_size,
        inline_text,
        blob_path,
    ) = row;

    let content = match content_kind.as_str() {
        "inline" => ContentRef::Inline {
            text: inline_text.ok_or_else(|| LibraryError::ContentIntegrity {
                capture_id,
                detail: "missing inline_text".into(),
            })?,
            sha256,
            byte_size: byte_size as u64,
            media_type: media_type.clone(),
        },
        "blob" => ContentRef::Blob {
            relative_path: blob_path.ok_or_else(|| LibraryError::ContentIntegrity {
                capture_id,
                detail: "missing blob_path".into(),
            })?,
            sha256,
            byte_size: byte_size as u64,
            media_type: media_type.clone(),
        },
        other => {
            return Err(LibraryError::ContentIntegrity {
                capture_id,
                detail: format!("unknown content_kind {other}"),
            })
        }
    };

    Ok(CaptureRetryRow {
        source_kind,
        source_path,
        external_session_id,
        media_type,
        content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{FixtureAdapter, ParserRegistry};
    use crate::ingest::ingest_adapter;
    use crate::storage::{ensure_home_layout, migrate_to_latest, open_connection};
    use std::fs;
    use tempfile::TempDir;

    /**
     * Unknown persisted Source kinds reject renormalize without Attempt mutation.
     */
    #[test]
    fn unknown_persisted_source_kind_rejects_without_mutation() {
        let temp = TempDir::new().expect("temp");
        let home = temp.path().join("home");
        let fixture = temp.path().join("fixture");
        fs::create_dir_all(&fixture).expect("fixture");
        fs::write(
            fixture.join("distill.fixture.json"),
            r#"{
  "version": 1,
  "captures": [
    {
      "id": "hello",
      "kind": "virtual",
      "virtual_text": "{\"record_type\":\"message\",\"role\":\"user\",\"text\":\"hi\"}\n",
      "external_session_id": "fixture-unknown-kind",
      "title": "Unknown Kind"
    }
  ]
}"#,
        )
        .expect("manifest");

        let paths = ensure_home_layout(&home).expect("paths");
        let mut conn = open_connection(&paths).expect("open");
        migrate_to_latest(&mut conn).expect("migrate");
        let adapter = FixtureAdapter::new(&fixture);
        let report = ingest_adapter(&mut conn, &paths, &adapter, 16 * 1024 * 1024).expect("ingest");
        let capture_id = report.capture_ids[0];
        let attempt_count_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM normalization_attempts WHERE capture_id = ?1",
                [capture_id],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(attempt_count_before, 1);

        conn.execute(
            "UPDATE captures SET source_kind = 'not_a_source' WHERE id = ?1",
            [capture_id],
        )
        .expect("plant unknown kind");

        let registry = ParserRegistry::default_v1();
        let err = renormalize_capture(&mut conn, &paths, capture_id, &registry)
            .expect_err("unknown kind");
        assert!(matches!(
            err,
            LibraryError::UnknownSourceKind { ref kind } if kind == "not_a_source"
        ));
        assert_eq!(err.code(), "unknown_source_kind");

        let attempt_count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM normalization_attempts WHERE capture_id = ?1",
                [capture_id],
                |row| row.get(0),
            )
            .expect("count after");
        assert_eq!(attempt_count_after, attempt_count_before);
    }
}
