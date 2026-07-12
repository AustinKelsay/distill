//! Safe in-root content resolution and destination CAS storage for legacy import.

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::fingerprint::resolve_in_root_regular_file;
use crate::error::LibraryResult;
use crate::storage::{store_capture_bytes, ContentRef, DistillPaths};
use crate::types::{LegacyImportSkip, DEFAULT_MAX_CAPTURE_BYTES};

/// Resolved legacy capture bytes ready for destination storage.
pub struct ResolvedLegacyContent {
    /// Exact source bytes.
    pub bytes: Vec<u8>,
    /// Media type hint.
    pub media_type: String,
}

/**
 * Resolve legacy capture content from payload JSON and optional blob column.
 *
 * Only regular files inside the source home are read. Missing or unsafe content
 * yields `None` plus a redacted skip entry.
 */
pub fn resolve_legacy_capture_content(
    source_home: &std::path::Path,
    raw_sha256: &str,
    raw_blob_path: Option<&str>,
    raw_payload_json: Option<&str>,
    skips: &mut Vec<LegacyImportSkip>,
) -> LibraryResult<Option<ResolvedLegacyContent>> {
    let payload: Value = raw_payload_json
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

    if let Some(content_ref) = payload.get("contentRef") {
        if let Some(resolved) =
            resolve_from_content_ref(source_home, content_ref, raw_sha256, skips)?
        {
            return Ok(Some(resolved));
        }
    }

    if let Some(blob_path) = raw_blob_path {
        if let Some(path) = resolve_in_root_regular_file(source_home, blob_path, true)? {
            let bytes = std::fs::read(&path)?;
            let actual = hex::encode(Sha256::digest(&bytes));
            if !raw_sha256.is_empty() && actual != raw_sha256 {
                skips.push(LegacyImportSkip {
                    category: "capture_content".into(),
                    reason: "checksum_mismatch".into(),
                    legacy_kind: Some("blob".into()),
                });
                return Ok(None);
            }
            return Ok(Some(ResolvedLegacyContent {
                bytes,
                media_type: "application/octet-stream".into(),
            }));
        }
        skips.push(LegacyImportSkip {
            category: "capture_content".into(),
            reason: "missing_or_unsafe_blob".into(),
            legacy_kind: Some("blob".into()),
        });
        return Ok(None);
    }

    skips.push(LegacyImportSkip {
        category: "capture_content".into(),
        reason: "missing_content_ref".into(),
        legacy_kind: None,
    });
    Ok(None)
}

/**
 * Persist resolved bytes into the destination inline/CAS store.
 *
 * The boolean reports whether this call created a new destination CAS file;
 * callers use it to remove only import-owned files if the SQL transaction fails.
 */
pub fn store_resolved_content(
    paths: &DistillPaths,
    resolved: &ResolvedLegacyContent,
) -> LibraryResult<(ContentRef, bool)> {
    let expected_blob =
        if resolved.bytes.len() as u64 > crate::types::INLINE_CONTENT_THRESHOLD_BYTES {
            let sha256 = hex::encode(Sha256::digest(&resolved.bytes));
            let (prefix, rest) = sha256.split_at(2);
            Some(paths.home.join(format!("blobs/{prefix}/{rest}")))
        } else {
            None
        };
    let existed = expected_blob.as_ref().is_some_and(|path| path.exists());
    let content = store_capture_bytes(
        &paths.home,
        &paths.staging,
        &paths.blobs,
        &resolved.bytes,
        &resolved.media_type,
        DEFAULT_MAX_CAPTURE_BYTES,
    )?;
    let created = matches!(&content, ContentRef::Blob { .. }) && !existed;
    Ok((content, created))
}

/**
 * Interpret a legacy `contentRef` object without leaking path strings into skips.
 */
fn resolve_from_content_ref(
    source_home: &std::path::Path,
    content_ref: &Value,
    raw_sha256: &str,
    skips: &mut Vec<LegacyImportSkip>,
) -> LibraryResult<Option<ResolvedLegacyContent>> {
    let kind = content_ref
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let media_type = content_ref
        .get("mediaType")
        .or_else(|| content_ref.get("media_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("application/octet-stream")
        .to_string();
    let declared_sha = content_ref
        .get("sha256")
        .and_then(|v| v.as_str())
        .unwrap_or(raw_sha256);

    match kind {
        "inline" => {
            let text = content_ref
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let bytes = text.as_bytes().to_vec();
            let actual = hex::encode(Sha256::digest(&bytes));
            if !declared_sha.is_empty() && actual != declared_sha {
                skips.push(LegacyImportSkip {
                    category: "capture_content".into(),
                    reason: "checksum_mismatch".into(),
                    legacy_kind: Some("inline".into()),
                });
                return Ok(None);
            }
            Ok(Some(ResolvedLegacyContent { bytes, media_type }))
        }
        "blob" => {
            let blob_path = content_ref
                .get("blobPath")
                .or_else(|| content_ref.get("blob_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let Some(path) = resolve_in_root_regular_file(source_home, blob_path, true)? else {
                skips.push(LegacyImportSkip {
                    category: "capture_content".into(),
                    reason: "missing_or_unsafe_blob".into(),
                    legacy_kind: Some("blob".into()),
                });
                return Ok(None);
            };
            let bytes = std::fs::read(path)?;
            let actual = hex::encode(Sha256::digest(&bytes));
            if !declared_sha.is_empty() && actual != declared_sha {
                skips.push(LegacyImportSkip {
                    category: "capture_content".into(),
                    reason: "checksum_mismatch".into(),
                    legacy_kind: Some("blob".into()),
                });
                return Ok(None);
            }
            Ok(Some(ResolvedLegacyContent { bytes, media_type }))
        }
        _ => {
            skips.push(LegacyImportSkip {
                category: "capture_content".into(),
                reason: "unsupported_content_kind".into(),
                legacy_kind: Some(kind.to_string()),
            });
            Ok(None)
        }
    }
}
