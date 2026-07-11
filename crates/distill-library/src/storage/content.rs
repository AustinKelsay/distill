//! Content-addressed Capture storage with staging, checksum, and atomic rename.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::home::set_file_mode_600;
use crate::error::{LibraryError, LibraryResult};
use crate::types::INLINE_CONTENT_THRESHOLD_BYTES;

/// Distill-owned Capture content reference after verified persistence.
#[derive(Clone, Debug)]
pub enum ContentRef {
    /// Small payload stored inline in SQLite.
    Inline {
        /// UTF-8 text body.
        text: String,
        /// SHA-256 hex digest.
        sha256: String,
        /// Byte size.
        byte_size: u64,
        /// Media type.
        media_type: String,
    },
    /// Larger payload stored under the blob CAS.
    Blob {
        /// Relative blob path from the Distill home (`blobs/ab/cd...`).
        relative_path: String,
        /// SHA-256 hex digest.
        sha256: String,
        /// Byte size.
        byte_size: u64,
        /// Media type.
        media_type: String,
    },
}

/**
 * Persist Capture bytes after checksum verification.
 *
 * Small payloads are inlined. Larger payloads are staged, checksummed, renamed
 * atomically into the CAS, and marked `0o600` on Unix.
 */
pub fn store_capture_bytes(
    home: &Path,
    staging_dir: &Path,
    _blobs_dir: &Path,
    bytes: &[u8],
    media_type: &str,
    max_capture_bytes: u64,
) -> LibraryResult<ContentRef> {
    let byte_size = bytes.len() as u64;
    if byte_size > max_capture_bytes {
        return Err(LibraryError::CaptureTooLarge {
            byte_size,
            limit: max_capture_bytes,
        });
    }
    let sha256 = hex::encode(Sha256::digest(bytes));

    if byte_size <= INLINE_CONTENT_THRESHOLD_BYTES {
        let text = String::from_utf8(bytes.to_vec()).map_err(|err| {
            LibraryError::InvalidArgument(format!("inline capture must be utf-8: {err}"))
        })?;
        return Ok(ContentRef::Inline {
            text,
            sha256,
            byte_size,
            media_type: media_type.to_string(),
        });
    }

    let relative = blob_relative_path(&sha256);
    let absolute = home.join(&relative);
    if absolute.exists() {
        verify_existing_blob(&absolute, &sha256, byte_size)?;
        return Ok(ContentRef::Blob {
            relative_path: relative,
            sha256,
            byte_size,
            media_type: media_type.to_string(),
        });
    }

    if let Some(parent) = absolute.parent() {
        fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(parent) = absolute.parent() {
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }

    let stage_name = format!("{sha256}.partial");
    let stage_path = staging_dir.join(stage_name);
    {
        let mut file = File::create(&stage_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    set_file_mode_600(&stage_path)?;

    let staged_hash = hex::encode(Sha256::digest(fs::read(&stage_path)?));
    if staged_hash != sha256 {
        let _ = fs::remove_file(&stage_path);
        return Err(LibraryError::ContentIntegrity {
            capture_id: -1,
            detail: "staged blob checksum mismatch before rename".into(),
        });
    }

    fs::rename(&stage_path, &absolute)?;
    set_file_mode_600(&absolute)?;

    Ok(ContentRef::Blob {
        relative_path: relative,
        sha256,
        byte_size,
        media_type: media_type.to_string(),
    })
}

/**
 * Read Capture bytes from an inline or blob ContentRef.
 */
pub fn read_capture_bytes(home: &Path, content: &ContentRef) -> LibraryResult<Vec<u8>> {
    match content {
        ContentRef::Inline { text, sha256, .. } => {
            let bytes = text.as_bytes().to_vec();
            let actual = hex::encode(Sha256::digest(&bytes));
            if actual != *sha256 {
                return Err(LibraryError::ContentIntegrity {
                    capture_id: -1,
                    detail: "inline content checksum mismatch".into(),
                });
            }
            Ok(bytes)
        }
        ContentRef::Blob {
            relative_path,
            sha256,
            byte_size,
            ..
        } => {
            let absolute = home.join(relative_path);
            let bytes = fs::read(&absolute)?;
            if bytes.len() as u64 != *byte_size {
                return Err(LibraryError::ContentIntegrity {
                    capture_id: -1,
                    detail: format!(
                        "blob size mismatch at {}: {} != {}",
                        relative_path,
                        bytes.len(),
                        byte_size
                    ),
                });
            }
            let actual = hex::encode(Sha256::digest(&bytes));
            if actual != *sha256 {
                return Err(LibraryError::ContentIntegrity {
                    capture_id: -1,
                    detail: format!("blob checksum mismatch at {relative_path}"),
                });
            }
            Ok(bytes)
        }
    }
}

/**
 * Build the relative CAS path `blobs/ab/<rest-of-sha256>`.
 */
fn blob_relative_path(sha256: &str) -> String {
    let (prefix, rest) = sha256.split_at(2);
    format!("blobs/{prefix}/{rest}")
}

/**
 * Verify an already-present blob matches the expected digest and size.
 */
fn verify_existing_blob(path: &Path, sha256: &str, byte_size: u64) -> LibraryResult<()> {
    let bytes = fs::read(path)?;
    if bytes.len() as u64 != byte_size {
        return Err(LibraryError::ContentIntegrity {
            capture_id: -1,
            detail: "existing blob size mismatch".into(),
        });
    }
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != sha256 {
        return Err(LibraryError::ContentIntegrity {
            capture_id: -1,
            detail: "existing blob checksum mismatch".into(),
        });
    }
    Ok(())
}
