//! Referenced Capture content integrity checks.

use std::fs;
use std::path::Path;

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::cas::{open_safe_cas_file, SafeCasOpen};
use crate::error::LibraryResult;
use crate::types::HealthIssue;

/**
 * Verify every accepted Capture's inline or blob bytes match size and checksum.
 *
 * Blob reads never follow symlinks and never leave the Distill home, even when a
 * corrupted row stores an absolute or traversal `blob_path`.
 */
pub(super) fn check_referenced_content(
    conn: &Connection,
    home: &Path,
    issues: &mut Vec<HealthIssue>,
) -> LibraryResult<String> {
    let mut status = "ok".to_string();
    let mut stmt = conn.prepare(
        "SELECT id, content_kind, sha256, byte_size, inline_text, blob_path FROM captures",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;
    for row in rows {
        let (id, kind, sha256, byte_size, inline_text, blob_path) = row?;
        match kind.as_str() {
            "inline" => {
                let text = inline_text.unwrap_or_default();
                let actual = hex::encode(Sha256::digest(text.as_bytes()));
                if text.len() as i64 != byte_size {
                    push_content_issue(
                        issues,
                        &mut status,
                        "content_size_mismatch",
                        format!("inline capture {id} size mismatch"),
                    );
                } else if actual != sha256 {
                    push_content_issue(
                        issues,
                        &mut status,
                        "content_checksum_mismatch",
                        format!("inline capture {id} checksum mismatch"),
                    );
                }
            }
            "blob" => {
                let relative = blob_path.unwrap_or_default();
                match open_safe_cas_file(home, &relative) {
                    SafeCasOpen::Regular(absolute) => {
                        let bytes = fs::read(&absolute)?;
                        let actual = hex::encode(Sha256::digest(&bytes));
                        if bytes.len() as i64 != byte_size {
                            push_content_issue(
                                issues,
                                &mut status,
                                "content_size_mismatch",
                                format!("blob capture {id} size mismatch"),
                            );
                        } else if actual != sha256 {
                            push_content_issue(
                                issues,
                                &mut status,
                                "content_checksum_mismatch",
                                format!("blob capture {id} checksum mismatch"),
                            );
                        }
                    }
                    SafeCasOpen::Missing => {
                        push_content_issue(
                            issues,
                            &mut status,
                            "content_missing",
                            format!("blob capture {id} missing referenced content"),
                        );
                    }
                    SafeCasOpen::InvalidPath => {
                        push_content_issue(
                            issues,
                            &mut status,
                            "content_invalid_blob_path",
                            format!("blob capture {id} has invalid blob path"),
                        );
                    }
                    SafeCasOpen::SymlinkOrSpecial => {
                        push_content_issue(
                            issues,
                            &mut status,
                            "content_symlink_blob",
                            format!("blob capture {id} references a non-regular CAS entry"),
                        );
                    }
                }
            }
            _ => {
                push_content_issue(
                    issues,
                    &mut status,
                    "content_unknown_kind",
                    format!("capture {id} has unknown content kind"),
                );
            }
        }
    }
    Ok(status)
}

/**
 * Push a content-category health issue and mark content status failed.
 */
fn push_content_issue(
    issues: &mut Vec<HealthIssue>,
    status: &mut String,
    code: &str,
    summary: String,
) {
    *status = "failed".to_string();
    issues.push(HealthIssue {
        code: code.into(),
        severity: "blocking".into(),
        category: "content".into(),
        summary,
    });
}
