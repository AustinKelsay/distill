//! Capture acceptance helpers: Source upsert, dedupe, path enforcement, and Activity.

use std::fs;
use std::path::{Component, Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::adapter::{CaptureCandidate, CaptureSnapshot, SourceKind};
use crate::error::{LibraryError, LibraryResult};
use crate::storage::ContentRef;

/**
 * Verify adapter-reported snapshot metadata before dedupe or persistence.
 */
pub(super) fn verify_snapshot_metadata(snapshot: &CaptureSnapshot) -> LibraryResult<()> {
    let actual_size = snapshot.bytes.len() as u64;
    let actual_sha256 = hex::encode(Sha256::digest(&snapshot.bytes));
    if snapshot.byte_size != actual_size || snapshot.sha256 != actual_sha256 {
        return Err(LibraryError::SourceAdapter(
            crate::adapter::SourceStageError::Snapshot(
                "adapter snapshot checksum or byte size did not match its bytes".to_string(),
            ),
        ));
    }
    Ok(())
}

/**
 * Reject file-backed candidates whose absolute path escapes the configured root.
 */
pub(super) fn enforce_configured_root(
    root: &Path,
    candidate: &CaptureCandidate,
) -> LibraryResult<()> {
    let Some(absolute) = candidate.absolute_path.as_ref() else {
        return Ok(());
    };
    let canonical_root = fs::canonicalize(root)?;
    let canonical_path = if absolute.exists() {
        fs::canonicalize(absolute)?
    } else {
        // Still reject obvious escapes before snapshot for missing hostile paths.
        normalize_join(&canonical_root, absolute)?
    };
    if !canonical_path.starts_with(&canonical_root) {
        return Err(LibraryError::PathOutsideConfiguredRoot {
            path: canonical_path,
            root: canonical_root,
        });
    }
    Ok(())
}

/**
 * Best-effort join/normalization used when the candidate path does not yet exist.
 */
fn normalize_join(root: &Path, path: &Path) -> LibraryResult<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(LibraryError::PathOutsideConfiguredRoot {
                        path: joined,
                        root: root.to_path_buf(),
                    });
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Ok(normalized)
}

/**
 * Upsert a Source row and return its id.
 */
pub(super) fn upsert_source(
    conn: &Connection,
    kind: &SourceKind,
    display_name: &str,
    data_root: &Path,
) -> LibraryResult<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO sources (
            kind, display_name, data_root, metadata_json, created_at, updated_at,
            enabled, configured_root
         ) VALUES (?1, ?2, ?3, '{}', ?4, ?4, 0, NULL)
         ON CONFLICT(kind) DO UPDATE SET
           display_name = excluded.display_name,
           data_root = excluded.data_root,
           updated_at = excluded.updated_at",
        params![
            kind.as_str(),
            display_name,
            data_root.display().to_string(),
            now
        ],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM sources WHERE kind = ?1",
        [kind.as_str()],
        |row| row.get(0),
    )?;
    Ok(id)
}

/**
 * Look up an exact-duplicate Capture by dedupe key.
 */
pub(super) fn find_duplicate(
    conn: &Connection,
    source_kind: &str,
    source_path: &str,
    sha256: &str,
) -> LibraryResult<Option<i64>> {
    let id = conn
        .query_row(
            "SELECT id FROM captures WHERE source_kind = ?1 AND source_path = ?2 AND sha256 = ?3",
            params![source_kind, source_path, sha256],
            |row| row.get(0),
        )
        .optional()?;
    Ok(id)
}

/**
 * Insert an accepted Capture after Distill owns recoverable content.
 */
pub(super) fn insert_capture(
    conn: &Connection,
    source_id: i64,
    candidate: &CaptureCandidate,
    content: &ContentRef,
    source_modified_at: Option<&str>,
) -> LibraryResult<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    let (content_kind, inline_text, blob_path, sha256, byte_size, media_type) = match content {
        ContentRef::Inline {
            text,
            sha256,
            byte_size,
            media_type,
        } => (
            "inline",
            Some(text.as_str()),
            None,
            sha256.as_str(),
            *byte_size,
            media_type.as_str(),
        ),
        ContentRef::Blob {
            relative_path,
            sha256,
            byte_size,
            media_type,
        } => (
            "blob",
            None,
            Some(relative_path.as_str()),
            sha256.as_str(),
            *byte_size,
            media_type.as_str(),
        ),
    };

    conn.execute(
        "INSERT INTO captures (
            source_id, source_kind, source_path, external_session_id,
            content_kind, media_type, sha256, byte_size, inline_text, blob_path,
            source_modified_at, accepted_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            source_id,
            candidate.source_kind.as_str(),
            candidate.source_path,
            candidate.external_session_id,
            content_kind,
            media_type,
            sha256,
            byte_size as i64,
            inline_text,
            blob_path,
            source_modified_at,
            now,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/**
 * Append an Activity Event outside a larger projection transaction when needed.
 */
pub(super) fn emit_activity(
    conn: &Connection,
    event_type: &str,
    source_kind: Option<&str>,
    session_id: Option<i64>,
    capture_id: Option<i64>,
    attempt_id: Option<i64>,
    payload: serde_json::Value,
) -> LibraryResult<()> {
    conn.execute(
        "INSERT INTO activity_events (
            event_type, occurred_at, source_kind, session_id, capture_id, attempt_id, payload_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event_type,
            chrono::Utc::now().to_rfc3339(),
            source_kind,
            session_id,
            capture_id,
            attempt_id,
            payload.to_string(),
        ],
    )?;
    Ok(())
}
