//! Same-Capture renormalization from Distill-owned replay bytes.

use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::adapter::{
    parse_fixture_bytes, CaptureCandidate, CaptureSnapshot, ParserIdentity, SourceKind,
    FIXTURE_PARSER_ID,
};
use crate::error::{LibraryError, LibraryResult};
use crate::storage::{read_capture_bytes, ContentRef, DistillPaths};
use crate::types::RenormalizeReport;

use super::attempt::{
    fail_attempt, insert_attempt, publish_projection,
    refresh_session_counters_after_failed_attempt, safe_failure_message,
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
 * Does not create a new Capture. Caller supplies only the Capture id; parser identity
 * comes from the Library-registered Fixture parser, never an arbitrary caller string.
 *
 * Parameters:
 * - `conn`: Library SQLite connection.
 * - `paths`: Distill home paths for content replay.
 * - `capture_id`: Accepted Capture to retry.
 * - `parser`: Registered Fixture parser identity/version.
 */
pub fn renormalize_capture(
    conn: &mut Connection,
    paths: &DistillPaths,
    capture_id: i64,
    parser: &ParserIdentity,
) -> LibraryResult<RenormalizeReport> {
    if parser.id != FIXTURE_PARSER_ID {
        return Err(LibraryError::InvalidArgument(
            "only the registered fixture parser may renormalize captures".into(),
        ));
    }

    let capture = load_capture_for_retry(conn, capture_id)?;
    if capture.source_kind != SourceKind::Fixture.as_str() {
        return Err(LibraryError::InvalidArgument(format!(
            "renormalize is not implemented for source kind {}",
            capture.source_kind
        )));
    }

    let bytes = read_capture_bytes(paths.home.as_path(), &capture.content, capture_id)?;
    let candidate = CaptureCandidate {
        source_kind: SourceKind::Fixture,
        source_path: capture.source_path.clone(),
        absolute_path: None,
        external_session_id: capture.external_session_id.clone(),
        title: None,
        is_virtual: true,
        virtual_bytes: Some(bytes.clone()),
        media_type: capture.media_type.clone(),
    };
    let snapshot = CaptureSnapshot {
        byte_size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(&bytes)),
        bytes,
        media_type: capture.media_type.clone(),
        source_modified_at: None,
    };

    match parse_fixture_bytes(&candidate, &snapshot.bytes, &parser.version) {
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
                Err(err) => {
                    fail_attempt(
                        conn,
                        attempt_id,
                        "projection_failed",
                        &safe_failure_message(&err),
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
        Err(err) => {
            let attempt_id = insert_attempt(
                conn,
                capture_id,
                &parser.id,
                &parser.version,
                "failed",
                Some("parse_failed"),
                Some(&safe_failure_message(&err)),
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
