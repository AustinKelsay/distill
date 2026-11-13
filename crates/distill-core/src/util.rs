use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Write a file by writing a sibling temp file and renaming it, so readers never see half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("path has no parent")?;
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    {
        let mut f = fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    set_private(&tmp);
    fs::rename(&tmp, path).with_context(|| format!("rename into {}", path.display()))?;
    Ok(())
}

pub fn write_json_pretty<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

pub fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(&line)?;
    // The manifest line must survive a crash before anything it supersedes is removed.
    f.sync_data()?;
    set_private(path);
    Ok(())
}

/// Flush a directory's entries (a rename or a new subdirectory in it) to disk.
pub fn sync_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .with_context(|| format!("sync {}", dir.display()))?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

pub fn read_jsonl<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // A torn last line (crash mid-append) is skipped rather than failing every reader.
        if let Ok(v) = serde_json::from_str(line) {
            out.push(v);
        }
    }
    Ok(out)
}

#[cfg(unix)]
pub fn set_private(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if path.is_dir() { 0o700 } else { 0o600 };
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
pub fn set_private(_path: &Path) {}

/// Make a string safe to use as one path component.
pub fn safe_component(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.');
    if trimmed.is_empty() {
        "_".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

/// Shorten text to at most `max` characters on a char boundary, marking the cut.
pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str(" …[clipped]");
    out
}
