//! The raw archive: exact source bytes, content-addressed, plus an append-only manifest of every
//! capture. Nothing else in Distill writes here, and agents never do.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::home::Home;
use crate::source::SourceKind;
use crate::util::{append_jsonl, now_rfc3339, read_jsonl, set_private};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    /// First capture from this origin.
    New,
    /// The new bytes start with the previous capture's bytes. The previous blob is dropped,
    /// since the new one contains it.
    Append,
    /// Earlier bytes changed (compaction, edits). Both versions are kept.
    Rewrite,
}

/// One line of `raw/manifest.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureRecord {
    pub source: SourceKind,
    pub origin: String,
    pub sha256: String,
    pub size: u64,
    pub kind: CaptureKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// The candidate's change signal when captured, so a sweep can skip unchanged sources.
    pub fingerprint: String,
    pub captured_at: String,
}

pub struct Archive {
    raw: PathBuf,
    manifest: PathBuf,
    records: Vec<CaptureRecord>,
    latest: BTreeMap<String, CaptureRecord>,
}

impl Archive {
    pub fn open(home: &Home) -> Result<Self> {
        let records: Vec<CaptureRecord> = read_jsonl(&home.manifest_path())?;
        let mut latest = BTreeMap::new();
        for r in &records {
            latest.insert(r.origin.clone(), r.clone());
        }
        Ok(Self {
            raw: home.raw_dir(),
            manifest: home.manifest_path(),
            records,
            latest,
        })
    }

    pub fn blob_path(&self, sha: &str) -> PathBuf {
        self.raw.join(&sha[..2]).join(sha)
    }

    /// The latest capture per origin.
    pub fn latest(&self) -> &BTreeMap<String, CaptureRecord> {
        &self.latest
    }

    pub fn records(&self) -> &[CaptureRecord] {
        &self.records
    }

    pub fn read_blob(&self, sha: &str) -> Result<Vec<u8>> {
        let bytes = fs::read(self.blob_path(sha)).with_context(|| format!("blob {sha} missing"))?;
        let actual = hex::encode(Sha256::digest(&bytes));
        if actual != sha {
            bail!("blob {sha} is corrupt (hashes to {actual})");
        }
        Ok(bytes)
    }

    /// Bytes on disk for every distinct blob still present.
    pub fn stored_bytes(&self) -> u64 {
        let mut seen = HashSet::new();
        self.records
            .iter()
            .filter(|r| seen.insert(r.sha256.clone()))
            .filter_map(|r| fs::metadata(self.blob_path(&r.sha256)).ok())
            .map(|m| m.len())
            .sum()
    }

    /// Store bytes for an origin. Returns the record, or None when the content is unchanged.
    pub fn put(
        &mut self,
        source: SourceKind,
        origin: &str,
        fingerprint: &str,
        mut reader: impl Read,
    ) -> Result<Option<CaptureRecord>> {
        let previous = self.latest.get(origin).cloned();
        fs::create_dir_all(&self.raw)?;
        let tmp = self.raw.join(format!(".incoming-{}", std::process::id()));
        let mut out = fs::File::create(&tmp)?;
        let mut full = Sha256::new();
        // Hash the first `previous.size` bytes separately to tell an append from a rewrite.
        let mut prefix = previous.as_ref().map(|p| (Sha256::new(), p.size));
        let mut prefix_sha = None;
        let mut total: u64 = 0;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            let chunk = &buf[..n];
            if let Some((h, want)) = prefix.as_mut()
                && total < *want
            {
                let take = ((*want - total) as usize).min(n);
                h.update(&chunk[..take]);
                if total + take as u64 == *want {
                    prefix_sha = prefix.take().map(|(h, _)| hex::encode(h.finalize()));
                }
            }
            full.update(chunk);
            out.write_all(chunk)?;
            total += n as u64;
        }
        out.sync_all()?;
        drop(out);
        let sha = hex::encode(full.finalize());

        if let Some(p) = &previous
            && p.sha256 == sha
        {
            fs::remove_file(&tmp)?;
            if p.fingerprint != fingerprint {
                // Same bytes, new mtime: remember the new fingerprint so the next sweep skips it.
                let mut rec = p.clone();
                rec.fingerprint = fingerprint.to_string();
                rec.captured_at = now_rfc3339();
                append_jsonl(&self.manifest, &rec)?;
                self.latest.insert(origin.to_string(), rec.clone());
                self.records.push(rec);
            }
            return Ok(None);
        }

        let kind = match &previous {
            None => CaptureKind::New,
            Some(p) if total > p.size && prefix_sha.as_deref() == Some(p.sha256.as_str()) => {
                CaptureKind::Append
            }
            Some(_) => CaptureKind::Rewrite,
        };

        let dest = self.blob_path(&sha);
        let shard = dest.parent().expect("blob has parent").to_path_buf();
        if !shard.exists() {
            fs::create_dir_all(&shard)?;
            set_private(&shard);
        }
        if dest.exists() {
            fs::remove_file(&tmp)?;
        } else {
            set_private(&tmp);
            fs::rename(&tmp, &dest)?;
        }
        // The blob must be durable before the manifest names it and before anything it
        // supersedes is removed. Synced every time, so a retry after a failed sync (which
        // leaves the shard or blob in place) still gets its barrier.
        crate::util::sync_dir(&self.raw)?;
        crate::util::sync_dir(&shard)?;

        let rec = CaptureRecord {
            source,
            origin: origin.to_string(),
            sha256: sha,
            size: total,
            kind,
            previous: previous.as_ref().map(|p| p.sha256.clone()),
            fingerprint: fingerprint.to_string(),
            captured_at: now_rfc3339(),
        };
        // The manifest line is written after the blob is in place, so every record points at
        // bytes that exist.
        append_jsonl(&self.manifest, &rec)?;
        self.latest.insert(origin.to_string(), rec.clone());
        self.records.push(rec.clone());

        if kind == CaptureKind::Append
            && let Some(p) = previous
        {
            self.drop_if_unreferenced(&p.sha256)?;
        }
        Ok(Some(rec))
    }

    /// Remove a blob that only a superseded append referred to.
    fn drop_if_unreferenced(&self, sha: &str) -> Result<()> {
        // Still the latest for some origin, or kept as the older side of a rewrite.
        let in_use = self.latest.values().any(|r| r.sha256 == sha)
            || self
                .records
                .iter()
                .any(|r| r.kind == CaptureKind::Rewrite && r.previous.as_deref() == Some(sha));
        if !in_use {
            let path = self.blob_path(sha);
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    /// Check every blob the archive should still hold.
    pub fn verify(&self) -> Result<VerifyReport> {
        let mut report = VerifyReport::default();
        let mut expected: BTreeMap<String, u64> = BTreeMap::new();
        for r in self.latest.values() {
            expected.insert(r.sha256.clone(), r.size);
        }
        for r in &self.records {
            if r.kind == CaptureKind::Rewrite
                && let Some(prev) = &r.previous
                && let Some(p) = self.records.iter().find(|x| &x.sha256 == prev)
            {
                expected.insert(prev.clone(), p.size);
            }
        }
        for (sha, size) in &expected {
            report.checked += 1;
            let path = self.blob_path(sha);
            match fs::File::open(&path) {
                Err(_) => report.missing.push(sha.clone()),
                Ok(mut f) => {
                    let mut h = Sha256::new();
                    let n = std::io::copy(&mut f, &mut h)?;
                    if hex::encode(h.finalize()) != *sha || n != *size {
                        report.corrupt.push(sha.clone());
                    } else {
                        report.bytes += n;
                    }
                }
            }
        }
        Ok(report)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VerifyReport {
    pub checked: u64,
    pub bytes: u64,
    pub missing: Vec<String>,
    pub corrupt: Vec<String>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.missing.is_empty() && self.corrupt.is_empty()
    }
}

pub fn copy_sqlite(path: &Path) -> Result<Vec<u8>> {
    let src = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let dir = std::env::temp_dir().join(format!("distill-sqlite-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let dest_path = dir.join("copy.db");
    let _ = fs::remove_file(&dest_path);
    {
        let mut dest = rusqlite::Connection::open(&dest_path)?;
        let backup = rusqlite::backup::Backup::new(&src, &mut dest)?;
        backup.run_to_completion(256, std::time::Duration::from_millis(0), None)?;
    }
    let bytes = fs::read(&dest_path)?;
    let _ = fs::remove_dir_all(&dir);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> (tempfile::TempDir, Home) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        home.ensure().unwrap();
        (dir, home)
    }

    #[test]
    fn append_replaces_and_rewrite_keeps_both() {
        let (_d, home) = home();
        let mut a = Archive::open(&home).unwrap();
        let k = SourceKind::Pi;
        let r1 = a.put(k, "s", "1", &b"line1\n"[..]).unwrap().unwrap();
        assert_eq!(r1.kind, CaptureKind::New);
        assert!(
            a.put(k, "s", "1", &b"line1\n"[..]).unwrap().is_none(),
            "unchanged bytes"
        );

        let r2 = a.put(k, "s", "2", &b"line1\nline2\n"[..]).unwrap().unwrap();
        assert_eq!(r2.kind, CaptureKind::Append);
        assert!(
            !a.blob_path(&r1.sha256).exists(),
            "appended-over blob is dropped"
        );

        let r3 = a.put(k, "s", "3", &b"compacted\n"[..]).unwrap().unwrap();
        assert_eq!(r3.kind, CaptureKind::Rewrite);
        assert!(
            a.blob_path(&r2.sha256).exists(),
            "rewrite keeps the old version"
        );
        assert!(a.blob_path(&r3.sha256).exists());

        let reopened = Archive::open(&home).unwrap();
        assert_eq!(reopened.latest()["s"].sha256, r3.sha256);
        let v = reopened.verify().unwrap();
        assert!(v.ok(), "{v:?}");
        assert_eq!(v.checked, 2);
    }

    #[test]
    fn verify_reports_corruption() {
        let (_d, home) = home();
        let mut a = Archive::open(&home).unwrap();
        let r = a
            .put(SourceKind::Pi, "s", "1", &b"hello"[..])
            .unwrap()
            .unwrap();
        fs::write(a.blob_path(&r.sha256), b"tampered").unwrap();
        let v = Archive::open(&home).unwrap().verify().unwrap();
        assert_eq!(v.corrupt, vec![r.sha256]);
    }
}
