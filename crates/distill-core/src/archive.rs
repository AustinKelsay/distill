//! The raw archive: exact harness bytes, named by their capture hash, plus an append-only
//! manifest of every capture. Nothing else in DISTILL writes here, and agents never do.
//!
//! The on-disk format is the previous build's, unchanged (ADR 0001): `raw/<aa>/<sha256>` and
//! `raw/manifest.jsonl`, with every manifest field name kept.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::harness::Harness;
use crate::home::Home;
use crate::util::{
    append_jsonl, create_dir_private, now_rfc3339, private_file, read_jsonl, set_private, sync_dir,
    write_atomic_with,
};

/// A blob file read as a stream that hashes what passes through it.
pub(crate) struct BlobReader {
    file: fs::File,
    hasher: Sha256,
    sha: String,
}

impl Read for BlobReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.file.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

impl BlobReader {
    /// Read what is left (a decoder may stop before the end) and check that the whole blob
    /// hashes to its name, as [`Archive::read_blob`] does.
    pub(crate) fn finish(mut self) -> Result<()> {
        std::io::copy(&mut self, &mut std::io::sink())?;
        let actual = hex::encode(self.hasher.finalize());
        if actual != self.sha {
            bail!("blob {} is corrupt (hashes to {actual})", self.sha);
        }
        Ok(())
    }
}

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

/// One line of `raw/manifest.jsonl`. The field names on disk are the previous build's
/// (`source`, `sha256`, `fingerprint`) and never change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureRecord {
    #[serde(rename = "source")]
    pub harness: Harness,
    pub origin: String,
    /// The capture hash: the SHA-256 of the captured bytes, which names the blob.
    #[serde(rename = "sha256")]
    pub capture_hash: String,
    pub size: u64,
    pub kind: CaptureKind,
    /// The capture hash this one replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// The candidate's change signal when captured, so a sweep can skip unchanged files. Not a
    /// hash.
    #[serde(rename = "fingerprint")]
    pub change_signal: String,
    pub captured_at: String,
}

pub struct Archive {
    raw: PathBuf,
    manifest: PathBuf,
    records: Vec<CaptureRecord>,
    latest: BTreeMap<String, CaptureRecord>,
}

impl Archive {
    /// This home's own raw archive.
    pub fn open(home: &Home) -> Result<Self> {
        Self::open_of(home, None)
    }

    /// A raw archive: this home's own with no machine, else that spoke's (ADR 0010), in the same
    /// format.
    pub fn open_of(home: &Home, machine: Option<&str>) -> Result<Self> {
        if let Some(m) = machine {
            crate::home::machine_name(m)?;
        }
        let manifest = home.manifest_path_of(machine);
        let records: Vec<CaptureRecord> = read_jsonl(&manifest)?;
        let mut latest = BTreeMap::new();
        for r in &records {
            latest.insert(r.origin.clone(), r.clone());
        }
        Ok(Self {
            raw: home.raw_dir_of(machine),
            manifest,
            records,
            latest,
        })
    }

    pub(crate) fn blob_path(&self, sha: &str) -> PathBuf {
        self.raw.join(&sha[..2]).join(sha)
    }

    /// The latest capture per origin.
    pub fn latest(&self) -> &BTreeMap<String, CaptureRecord> {
        &self.latest
    }

    pub fn records(&self) -> &[CaptureRecord] {
        &self.records
    }

    pub(crate) fn read_blob(&self, sha: &str) -> Result<Vec<u8>> {
        let bytes = fs::read(self.blob_path(sha)).with_context(|| format!("blob {sha} missing"))?;
        let actual = hex::encode(Sha256::digest(&bytes));
        if actual != sha {
            bail!("blob {sha} is corrupt (hashes to {actual})");
        }
        Ok(bytes)
    }

    /// A blob as a stream, hashed as it is read, so nothing holds it whole. Call
    /// [`BlobReader::finish`] after reading: it fails if the blob does not hash to its name.
    pub(crate) fn open_blob(&self, sha: &str) -> Result<BlobReader> {
        let file =
            fs::File::open(self.blob_path(sha)).with_context(|| format!("blob {sha} missing"))?;
        Ok(BlobReader {
            file,
            hasher: Sha256::new(),
            sha: sha.to_string(),
        })
    }

    /// Bytes on disk for every distinct blob still present.
    pub fn stored_bytes(&self) -> u64 {
        let mut seen = HashSet::new();
        self.records
            .iter()
            .filter(|r| seen.insert(r.capture_hash.clone()))
            .filter_map(|r| fs::metadata(self.blob_path(&r.capture_hash)).ok())
            .map(|m| m.len())
            .sum()
    }

    /// Store bytes for an origin. Returns the record, or None when the content is unchanged.
    pub fn put(
        &mut self,
        harness: Harness,
        origin: &str,
        change_signal: &str,
        mut reader: impl Read,
    ) -> Result<Option<CaptureRecord>> {
        let previous = self.latest.get(origin).cloned();
        create_dir_private(&self.raw)?;
        let tmp = self.raw.join(format!("{INCOMING}{}", std::process::id()));
        let mut out = private_file()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
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
            && p.capture_hash == sha
        {
            fs::remove_file(&tmp)?;
            if p.change_signal != change_signal {
                // Same bytes, new mtime: remember the new signal so the next sweep skips it.
                let mut rec = p.clone();
                rec.change_signal = change_signal.to_string();
                rec.captured_at = now_rfc3339();
                append_jsonl(&self.manifest, &rec)?;
                self.latest.insert(origin.to_string(), rec.clone());
                self.records.push(rec);
            }
            return Ok(None);
        }

        let kind = match &previous {
            None => CaptureKind::New,
            Some(p) if total > p.size && prefix_sha.as_deref() == Some(p.capture_hash.as_str()) => {
                CaptureKind::Append
            }
            Some(_) => CaptureKind::Rewrite,
        };

        let dest = self.blob_path(&sha);
        let shard = dest.parent().expect("blob has parent").to_path_buf();
        if !shard.exists() {
            create_dir_private(&shard)?;
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
            harness,
            origin: origin.to_string(),
            capture_hash: sha,
            size: total,
            kind,
            previous: previous.as_ref().map(|p| p.capture_hash.clone()),
            change_signal: change_signal.to_string(),
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
            self.drop_if_unreferenced(&p.capture_hash)?;
        }
        Ok(Some(rec))
    }

    /// Remove a blob that only a superseded append referred to.
    fn drop_if_unreferenced(&self, sha: &str) -> Result<()> {
        if !in_use(&self.latest, &self.records, sha) {
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
        for (sha, size) in &expected_blobs(&self.records) {
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

/// Whether an archive with these records still needs blob `sha`: the latest capture of some
/// origin, or the older side of a rewrite.
fn in_use(latest: &BTreeMap<String, CaptureRecord>, records: &[CaptureRecord], sha: &str) -> bool {
    latest.values().any(|r| r.capture_hash == sha)
        || records
            .iter()
            .any(|r| r.kind == CaptureKind::Rewrite && r.previous.as_deref() == Some(sha))
}

/// The blobs an archive with these records should hold, with their sizes: the latest capture
/// of each origin, and the older side of every rewrite. A blob a longer append superseded is not
/// among them.
pub(crate) fn expected_blobs(records: &[CaptureRecord]) -> BTreeMap<String, u64> {
    let mut latest = BTreeMap::new();
    for r in records {
        latest.insert(&r.origin, r);
    }
    let mut expected: BTreeMap<String, u64> = latest
        .values()
        .map(|r| (r.capture_hash.clone(), r.size))
        .collect();
    for r in records {
        if r.kind == CaptureKind::Rewrite
            && let Some(prev) = &r.previous
            && let Some(p) = records.iter().find(|x| &x.capture_hash == prev)
        {
            expected.insert(prev.clone(), p.size);
        }
    }
    expected
}

/// The records in manifest lines, as [`read_jsonl`] reads them.
pub(crate) fn parse_manifest(bytes: &[u8]) -> Vec<CaptureRecord> {
    bytes
        .split(|b| *b == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect()
}

/// What a writer of a raw archive stages there before a rename, each named for its process: a
/// capture the sweep is taking, a blob a pull or a restore is taking, and the copy's manifest a
/// pull rewrites ([`write_atomic_with`]'s name for it).
const INCOMING: &str = ".incoming-";
const PULLING: &str = ".pulling-";
const RESTORING: &str = ".restoring-";
const STAGING: [&str; 4] = [INCOMING, PULLING, RESTORING, ".manifest.jsonl.tmp-"];

/// Remove the staging files a writer that died left in `home`'s raw archives, its own and each
/// spoke's, keeping any whose process still runs. Take the archive lock first. Returns how many
/// went. Best effort: what can't be read or removed stays, and nothing waits on it.
pub fn clear_staging(home: &Home) -> u64 {
    let mut raws = vec![home.raw_dir_of(None)];
    // Each spoke's folder on its own, so one entry that isn't a spoke's skips only itself.
    for e in fs::read_dir(home.machines_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Some(m) = e.file_name().to_str()
            && crate::home::machine_name(m).is_ok()
        {
            raws.push(home.raw_dir_of(Some(m)));
        }
    }
    let mut cleared = 0;
    for raw in raws {
        let Ok(entries) = fs::read_dir(&raw) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name();
            let pid = STAGING
                .iter()
                .find_map(|p| name.to_str()?.strip_prefix(p)?.parse::<u32>().ok());
            if let Some(pid) = pid
                && pid != std::process::id()
                && !running(pid)
                && e.file_type().is_ok_and(|t| t.is_file())
                && fs::remove_file(e.path()).is_ok()
            {
                cleared += 1;
            }
        }
    }
    cleared
}

/// Sync the folder of `blob`, already in place, and `raw`, unless this pull has: an earlier pull
/// may have renamed it in and failed before its sync, and a line is about to name it.
fn synced_once(synced: &mut HashSet<PathBuf>, raw: &Path, blob: &Path) -> Result<()> {
    let shard = blob.parent().context("blob has parent")?;
    if synced.insert(shard.to_path_buf()) {
        sync_dir(shard)?;
        sync_dir(raw)?;
    }
    Ok(())
}

/// Whether process `pid` is running, as far as this account can tell.
fn running(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 sends nothing; it only asks whether the process is there.
    let there = unsafe { libc::kill(pid, 0) } == 0;
    there || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Restore an archive into `home`, which has none (ADR 0009, issue #149): every blob `manifest`
/// says it should hold, from `fetch` (which writes a blob's bytes and says whether it had it),
/// then the manifest itself, so every line names bytes that are in place. Each blob is checked
/// against its capture hash and size before it goes in. Returns the blobs `fetch` lacked.
pub fn restore(
    home: &Home,
    manifest: &[u8],
    fetch: impl FnMut(&str, &mut dyn Write) -> Result<bool>,
) -> Result<Vec<String>> {
    restore_of(home, None, manifest, fetch)
}

/// [`restore`] into one raw archive: this home's own with no machine, else that spoke's.
pub fn restore_of(
    home: &Home,
    machine: Option<&str>,
    manifest: &[u8],
    mut fetch: impl FnMut(&str, &mut dyn Write) -> Result<bool>,
) -> Result<Vec<String>> {
    if let Some(m) = machine {
        crate::home::machine_name(m)?;
    }
    let raw = home.raw_dir_of(machine);
    if home.manifest_path_of(machine).exists() {
        bail!("{} already has an archive", raw.display());
    }
    create_dir_private(&raw)?;
    set_private(&raw);
    let tmp = raw.join(format!("{RESTORING}{}", std::process::id()));
    let mut missing = Vec::new();
    let mut shards = BTreeSet::new();
    for (sha, size) in expected_blobs(&parse_manifest(manifest)) {
        let placed = (|| -> Result<bool> {
            let file = private_file()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)?;
            let mut out = Hashing {
                file,
                hasher: Sha256::new(),
                bytes: 0,
            };
            if !fetch(&sha, &mut out)? {
                return Ok(false);
            }
            out.file.sync_all()?;
            let actual = hex::encode(out.hasher.finalize());
            if actual != sha || out.bytes != size {
                bail!("the backup's copy of blob {sha} does not match its capture hash");
            }
            Ok(true)
        })();
        if !placed.inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })? {
            missing.push(sha);
            continue;
        }
        let shard = raw.join(&sha[..2]);
        create_dir_private(&shard)?;
        fs::rename(&tmp, shard.join(&sha))?;
        shards.insert(shard);
    }
    let _ = fs::remove_file(&tmp);
    for shard in &shards {
        crate::util::sync_dir(shard)?;
    }
    crate::util::write_atomic(&home.manifest_path_of(machine), manifest)?;
    crate::util::sync_dir(&raw)?;
    Ok(missing)
}

/// A file being written, hashed and counted as it goes.
struct Hashing {
    file: fs::File,
    hasher: Sha256,
    bytes: u64,
}

impl Write for Hashing {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.file.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VerifyReport {
    pub checked: u64,
    pub bytes: u64,
    pub missing: Vec<String>,
    pub corrupt: Vec<String>,
}

/// [`Archive::verify`] over every raw archive the home holds (ADR 0010); a spoke's missing or
/// corrupt captures are named with their machine.
pub fn verify_all(home: &Home) -> Result<VerifyReport> {
    let mut all = VerifyReport::default();
    for machine in home.archives()? {
        let r = Archive::open_of(home, machine.as_deref())?.verify()?;
        let named = |c: String| match &machine {
            Some(m) => format!("{m}: {c}"),
            None => c,
        };
        all.checked += r.checked;
        all.bytes += r.bytes;
        all.missing.extend(r.missing.into_iter().map(named));
        all.corrupt.extend(r.corrupt.into_iter().map(named));
    }
    Ok(all)
}

/// The first line of an export (ADR 0010): what follows is one spoke's new captures.
const EXPORT_START: &str = "distill-export 1";

/// The line after [`EXPORT_START`]: which machine this is, and where its new captures start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ExportHeader {
    machine: String,
    /// The byte of its manifest the export starts at: the length of the hub's copy.
    since: u64,
    /// The SHA-256 of its manifest's first `since` bytes, which the hub's copy must match.
    prefix: String,
    /// The length of its manifest's whole lines when the export started.
    end: u64,
}

/// What an export sent, or a pull took.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Transfer {
    /// Where in the spoke's manifest it started and ended, in bytes.
    pub since: u64,
    pub to: u64,
    /// Manifest lines, and the blobs that came with them.
    pub lines: u64,
    pub blobs: u64,
    /// Blobs the hub already held, which it checked and skipped.
    #[serde(default)]
    pub present: u64,
    /// Blob bytes.
    pub bytes: u64,
    /// The pull stopped at the tick's end, between two captures; the next one goes on from there.
    #[serde(default)]
    pub stopped: bool,
}

/// Export this home's own captures from byte `since` of its manifest to `out`, as `machine`
/// (ADR 0010): each manifest line after it, in order and as it is, with the blob it names just
/// before it while this archive still holds that blob. A blob a later append superseded is gone,
/// so only its line goes. `t` counts what was sent as it goes, so a caller has it also when the
/// export stops short.
pub fn export(
    home: &Home,
    machine: &str,
    since: u64,
    out: &mut dyn Write,
    t: &mut Transfer,
) -> Result<()> {
    *t = Transfer {
        since,
        to: since,
        ..Default::default()
    };
    let manifest = match fs::read(home.manifest_path()) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).context("read the manifest"),
    };
    let end = manifest
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |i| i + 1);
    let at = usize::try_from(since).unwrap_or(usize::MAX);
    if at > end || (at > 0 && manifest[at - 1] != b'\n') {
        bail!(
            "no capture of {machine} ends at byte {since}: the hub's copy is not of this machine's \
             archive"
        );
    }
    let header = ExportHeader {
        machine: machine.to_string(),
        since,
        prefix: hex::encode(Sha256::digest(&manifest[..at])),
        end: end as u64,
    };
    writeln!(out, "{EXPORT_START}")?;
    writeln!(out, "{}", serde_json::to_string(&header)?)?;
    let archive = Archive::open(home)?;
    let held = expected_blobs(archive.records());
    // What the hub's copy holds as each line goes in, dropping as `take` drops: a blob goes
    // whenever the copy lacks it, also when an append there dropped it and a later line needs
    // it again.
    let mut records = parse_manifest(&manifest[..at]);
    let mut latest: BTreeMap<String, CaptureRecord> = BTreeMap::new();
    for r in &records {
        latest.insert(r.origin.clone(), r.clone());
    }
    let mut there: HashSet<String> = expected_blobs(&records).into_keys().collect();
    for line in manifest[at..end].split_inclusive(|b| *b == b'\n') {
        let line = &line[..line.len() - 1];
        let rec = serde_json::from_slice::<CaptureRecord>(line).ok();
        if let Some(rec) = &rec
            && held.contains_key(&rec.capture_hash)
            && there.insert(rec.capture_hash.clone())
        {
            let path = archive.blob_path(&rec.capture_hash);
            match fs::File::open(&path) {
                Ok(mut f) => {
                    let len = f.metadata()?.len();
                    writeln!(out, "blob {} {len}", rec.capture_hash)?;
                    // Counted as it goes, so an export cut off inside a blob logs what left.
                    let mut counted = Tally {
                        out: &mut *out,
                        bytes: &mut t.bytes,
                    };
                    let n = std::io::copy(&mut (&mut f).take(len), &mut counted)?;
                    if n != len {
                        bail!("blob {} changed while it was read", rec.capture_hash);
                    }
                    t.blobs += 1;
                }
                // Missing here too: the hub's copy will miss it as this one does.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("read blob {}", rec.capture_hash)),
            }
        }
        writeln!(out, "line {}", line.len())?;
        out.write_all(line)?;
        if let Some(rec) = rec {
            let previous = latest.insert(rec.origin.clone(), rec.clone());
            records.push(rec.clone());
            if rec.kind == CaptureKind::Append
                && let Some(p) = previous
                && p.capture_hash != rec.capture_hash
                && !in_use(&latest, &records, &p.capture_hash)
            {
                there.remove(&p.capture_hash);
            }
        }
        t.lines += 1;
        t.to += line.len() as u64 + 1;
    }
    writeln!(out, "end")?;
    out.flush()?;
    Ok(())
}

/// The length of `bytes` up to the end of its last line.
fn whole_lines(bytes: &[u8]) -> usize {
    bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1)
}

/// A writer that adds what each write took to `bytes`.
struct Tally<'a> {
    out: &'a mut dyn Write,
    bytes: &'a mut u64,
}

impl Write for Tally<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.out.write(buf)?;
        *self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

/// How [`take`] says the stream stopped before its end.
pub const ENDED_EARLY: &str = "the export ended early";

/// The longest frame [`take`] reads, a header's included, and the longest manifest line.
const MAX_FRAME: u64 = 4096;
const MAX_LINE: usize = 1 << 20;

fn is_blob_name(sha: &str) -> bool {
    sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Read an export's next frame line into `frame`, without its newline: at most [`MAX_FRAME`].
fn read_frame(frame: &mut String, input: &mut dyn std::io::BufRead) -> Result<()> {
    use std::io::BufRead;
    frame.clear();
    if Read::take(&mut *input, MAX_FRAME).read_line(frame)? == 0 {
        bail!(ENDED_EARLY);
    }
    if frame.pop() != Some('\n') {
        bail!(ENDED_EARLY);
    }
    Ok(())
}

/// `machine`'s copy of its manifest here, as a pull starts from it: its bytes, the length of its
/// whole lines (the byte its export is asked to start at), and their SHA-256, which the spoke's
/// manifest must start with.
pub struct Copy {
    bytes: Vec<u8>,
    pub since: u64,
    prefix: String,
}

/// Read `machine`'s copy here; none yet is an empty one.
pub fn copy_of(home: &Home, machine: &str) -> Result<Copy> {
    // Before its name makes a path.
    crate::home::machine_name(machine)?;
    let bytes = match fs::read(home.manifest_path_of(Some(machine))) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).with_context(|| format!("read {machine}'s manifest")),
    };
    let whole = whole_lines(&bytes);
    let prefix = hex::encode(Sha256::digest(&bytes[..whole]));
    let since = whole as u64;
    Ok(Copy {
        bytes,
        since,
        prefix,
    })
}

/// Read the start of `machine`'s export from `input`: the export's first line, then its header,
/// which must name `machine` and continue `copy`. Writes nothing.
fn export_header(
    machine: &str,
    copy: &Copy,
    input: &mut dyn std::io::BufRead,
) -> Result<ExportHeader> {
    crate::home::machine_name(machine)?;
    let mut frame = String::new();
    read_frame(&mut frame, input)?;
    if frame != EXPORT_START {
        bail!(
            "not an export from DISTILL: {:?}",
            frame.chars().take(80).collect::<String>()
        );
    }
    read_frame(&mut frame, input)?;
    let header: ExportHeader = serde_json::from_str(&frame).context("read the export's header")?;
    if header.machine != machine {
        bail!(
            "the machine there is \"{}\", not \"{machine}\"",
            header.machine
        );
    }
    if header.since != copy.since || header.prefix != copy.prefix {
        bail!(
            "\"{machine}\"'s archive there does not continue its copy here: is another machine \
             using the name \"{machine}\"?"
        );
    }
    Ok(header)
}

/// Check the start of `machine`'s export from `input` as a pull would (see [`export_header`]),
/// against `copy` as it was when the export was asked for, without taking anything: whether the
/// spoke has captures `copy` lacks.
pub fn check_export(machine: &str, copy: &Copy, input: &mut dyn std::io::BufRead) -> Result<bool> {
    let header = export_header(machine, copy, input)?;
    Ok(header.end > header.since)
}

/// Take `machine`'s export from `input` into its raw archive here (ADR 0010), which it must
/// continue: the export starts where this copy ends, and from the same bytes. Each blob is
/// checked against its name, and one already here is skipped, before the line that names it is
/// appended, so a pull that stops keeps every capture it took and the next one goes on from there.
/// `stop` is asked after each line.
pub fn take(
    home: &Home,
    machine: &str,
    input: &mut dyn std::io::BufRead,
    stop: &dyn Fn() -> bool,
) -> Result<Transfer> {
    let mut t = Transfer::default();
    take_into(home, machine, input, stop, &mut t).map(|()| t)
}

/// [`take`], counting into `t` what went in, so a pull that fails partway reports what it kept.
pub fn take_into(
    home: &Home,
    machine: &str,
    input: &mut dyn std::io::BufRead,
    stop: &dyn Fn() -> bool,
    t: &mut Transfer,
) -> Result<()> {
    let copy = copy_of(home, machine)?;
    let header = export_header(machine, &copy, input)?;
    let (ours, whole) = (copy.bytes, copy.since as usize);
    let path = home.manifest_path_of(Some(machine));
    let mut frame = String::new();
    // A line cut short by a write that failed here (a full disk) is dropped: the export sends it
    // again whole.
    if whole < ours.len() {
        let f = fs::OpenOptions::new().write(true).open(&path)?;
        f.set_len(whole as u64)?;
        f.sync_all()?;
    }
    let mut archive = Archive::open_of(home, Some(machine))?;
    create_dir_private(&archive.raw)?;
    let tmp = archive.raw.join(format!("{PULLING}{}", std::process::id()));
    // Folders of blobs already here that this pull has synced: an earlier pull may have renamed
    // one in and failed before its sync, so it is synced once before a line names it.
    let mut synced = HashSet::new();
    *t = Transfer {
        since: header.since,
        to: header.since,
        ..Default::default()
    };
    // Lines are held back while a capture they make the latest has no blob here yet (the spoke
    // sent no blob for an append a later one superseded), and go in together once a later line
    // brings one: so what an append drops is dropped only once what replaces it is in place, and
    // a pull stops only there.
    let mut pending: Vec<Vec<u8>> = Vec::new();
    let mut waiting: BTreeSet<String> = BTreeSet::new();
    let commit = |archive: &mut Archive, pending: &mut Vec<Vec<u8>>, t: &mut Transfer| {
        archive.append_lines(pending)?;
        for line in pending.drain(..) {
            t.lines += 1;
            t.to += line.len() as u64 + 1;
        }
        anyhow::Ok(())
    };
    let result = (|| -> Result<()> {
        loop {
            read_frame(&mut frame, input)?;
            let words: Vec<&str> = frame.split(' ').collect();
            match words.as_slice() {
                ["blob", sha, len] => {
                    let len: u64 = len.parse().context("a blob's length")?;
                    if !is_blob_name(sha) {
                        bail!("not a blob name: {sha:?}");
                    }
                    let mut out = Hashing {
                        file: private_file()
                            .write(true)
                            .create(true)
                            .truncate(true)
                            .open(&tmp)?,
                        hasher: Sha256::new(),
                        bytes: 0,
                    };
                    let n = std::io::copy(&mut input.take(len), &mut out)?;
                    if n != len {
                        bail!(ENDED_EARLY);
                    }
                    if hex::encode(out.hasher.finalize()) != *sha {
                        bail!("blob {sha} came changed: it does not hash to its name");
                    }
                    let dest = archive.blob_path(sha);
                    if dest.exists() {
                        t.present += 1;
                        synced_once(&mut synced, &archive.raw, &dest)?;
                        continue;
                    }
                    out.file.sync_all()?;
                    let shard = dest.parent().expect("blob has parent");
                    create_dir_private(shard)?;
                    fs::rename(&tmp, &dest)?;
                    crate::util::sync_dir(shard)?;
                    crate::util::sync_dir(&archive.raw)?;
                    synced.insert(shard.to_path_buf());
                    t.blobs += 1;
                    t.bytes += len;
                }
                ["line", len] => {
                    let len: usize = len.parse().context("a line's length")?;
                    if len > MAX_LINE {
                        bail!("a manifest line of {len} bytes is longer than any capture's");
                    }
                    let mut line = vec![0; len];
                    input.read_exact(&mut line).context(ENDED_EARLY)?;
                    if line.contains(&b'\n') {
                        bail!("a manifest line holds a line break");
                    }
                    if let Ok(rec) = serde_json::from_slice::<CaptureRecord>(&line) {
                        if !is_blob_name(&rec.capture_hash) {
                            bail!("not a blob name: {:?}", rec.capture_hash);
                        }
                        let blob = archive.blob_path(&rec.capture_hash);
                        if blob.exists() {
                            // Its blob may be one an earlier pull renamed in and failed to sync.
                            synced_once(&mut synced, &archive.raw, &blob)?;
                            waiting.remove(&rec.origin);
                        } else {
                            waiting.insert(rec.origin);
                        }
                    }
                    pending.push(line);
                    if waiting.is_empty() {
                        commit(&mut archive, &mut pending, t)?;
                        if stop() {
                            t.stopped = true;
                            return Ok(());
                        }
                    }
                }
                // What the spoke lacks too stays lacking: the copy is as the spoke's is.
                ["end"] => return commit(&mut archive, &mut pending, t),
                _ => bail!(
                    "not part of an export: {:?}",
                    frame.chars().take(80).collect::<String>()
                ),
            }
        }
    })();
    let _ = fs::remove_file(&tmp);
    result
}

impl Archive {
    /// Append `lines` as they are, all of them or none, then drop each blob an append among them
    /// superseded that nothing here needs once they are all in.
    fn append_lines(&mut self, lines: &[Vec<u8>]) -> Result<()> {
        if lines.is_empty() {
            return Ok(());
        }
        let mut batch = Vec::new();
        for line in lines {
            batch.extend_from_slice(line);
            batch.push(b'\n');
        }
        if lines.len() > 1 {
            // Lines that wait on a later one's blob go in by one rename, so even a crash leaves
            // all of them or none.
            let ours = match fs::read(&self.manifest) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(e).context("read the copy's manifest"),
            };
            write_atomic_with(&self.manifest, |w| {
                w.write_all(&ours)?;
                w.write_all(&batch)?;
                Ok(())
            })?;
            sync_dir(&self.raw)?;
        } else {
            let created = !self.manifest.exists();
            let mut f = private_file()
                .create(true)
                .append(true)
                .open(&self.manifest)?;
            let before = f.metadata()?.len();
            if let Err(e) = f.write_all(&batch).and_then(|()| f.sync_data()) {
                let _ = f.set_len(before).and_then(|()| f.sync_data());
                return Err(e).context("append to the copy's manifest");
            }
            set_private(&self.manifest);
            if created {
                sync_dir(&self.raw)?;
            }
        }
        let mut superseded = Vec::new();
        for line in lines {
            let Ok(rec) = serde_json::from_slice::<CaptureRecord>(line) else {
                continue;
            };
            let previous = self.latest.insert(rec.origin.clone(), rec.clone());
            if rec.kind == CaptureKind::Append
                && let Some(p) = previous
                && p.capture_hash != rec.capture_hash
            {
                superseded.push(p.capture_hash);
            }
            self.records.push(rec);
        }
        for sha in superseded {
            self.drop_if_unreferenced(&sha)?;
        }
        Ok(())
    }
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
    // The copy holds session text, so its folder is owner-only even in a shared temp dir.
    let dir = std::env::temp_dir().join(format!("distill-sqlite-{}", std::process::id()));
    create_dir_private(&dir)?;
    set_private(&dir);
    let dest_path = dir.join("copy.db");
    let _ = fs::remove_file(&dest_path);
    let result = (|| -> Result<Vec<u8>> {
        let mut dest = rusqlite::Connection::open(&dest_path)?;
        let backup = rusqlite::backup::Backup::new(&src, &mut dest)?;
        // Copy every page in one step, under one read lock, so a writer can't force the copy to
        // restart. A database another process holds locked is retried briefly, then reported;
        // the next sweep tries again. (Stepping in small chunks with no pause retried a busy
        // database forever.)
        use rusqlite::backup::StepResult;
        for _ in 0..50 {
            match backup.step(-1)? {
                StepResult::Done => {
                    drop(backup);
                    drop(dest);
                    return Ok(fs::read(&dest_path)?);
                }
                StepResult::More | StepResult::Busy | StepResult::Locked => {
                    std::thread::sleep(std::time::Duration::from_millis(100))
                }
                _ => std::thread::sleep(std::time::Duration::from_millis(100)),
            }
        }
        bail!(
            "{} stayed locked by another process; will retry next sweep",
            path.display()
        )
    })();
    let _ = fs::remove_dir_all(&dir);
    result
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

    /// What `export` writes from `spoke` for the hub's copy of `nas` as it is now.
    fn export_for(spoke: &Home, hub: &Home) -> Vec<u8> {
        let since = copy_of(hub, "nas").unwrap().since;
        let mut out = Vec::new();
        export(spoke, "nas", since, &mut out, &mut Transfer::default()).unwrap();
        out
    }

    fn pull(spoke: &Home, hub: &Home) -> Transfer {
        let bytes = export_for(spoke, hub);
        take(hub, "nas", &mut &bytes[..], &|| false).unwrap()
    }

    /// Every file under `dir`, by its path there, with its bytes.
    fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut all = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let rel = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                    all.insert(rel, fs::read(&p).unwrap());
                }
            }
        }
        all
    }

    #[test]
    fn a_pull_copies_a_spokes_new_captures_and_the_next_takes_only_whats_new() {
        let (_s, spoke) = home();
        let (_h, hub) = home();
        let mut a = Archive::open(&spoke).unwrap();
        let k = Harness::Pi;
        a.put(k, "s", "1", &b"one\n"[..]).unwrap();
        a.put(k, "t", "1", &b"other\n"[..]).unwrap();
        let first = pull(&spoke, &hub);
        assert_eq!((first.lines, first.blobs), (2, 2), "{first:?}");
        assert_eq!(files(&spoke.raw_dir()), files(&hub.raw_dir_of(Some("nas"))));

        // Two appends and a rewrite between pulls: the first append's blob is gone on the spoke,
        // so only its line comes, and the hub drops what the appends supersede.
        a.put(k, "s", "2", &b"one\ntwo\n"[..]).unwrap();
        a.put(k, "s", "3", &b"one\ntwo\nthree\n"[..]).unwrap();
        a.put(k, "t", "2", &b"compacted\n"[..]).unwrap();
        let second = pull(&spoke, &hub);
        assert_eq!((second.lines, second.blobs), (3, 2), "{second:?}");
        assert_eq!(second.since, first.to);
        assert_eq!(files(&spoke.raw_dir()), files(&hub.raw_dir_of(Some("nas"))));
        let nas = Archive::open_of(&hub, Some("nas"))
            .unwrap()
            .verify()
            .unwrap();
        assert!(nas.ok() && nas.checked == 3, "{nas:?}");

        // Nothing new: nothing comes.
        let third = pull(&spoke, &hub);
        assert_eq!((third.lines, third.blobs), (0, 0), "{third:?}");
    }

    #[test]
    fn staging_a_dead_writer_left_is_cleared_and_a_running_ones_kept() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        let dead = {
            let mut c = std::process::Command::new("true").spawn().unwrap();
            let pid = c.id();
            c.wait().unwrap();
            pid
        };
        let own = std::process::id();
        let (raw, nas) = (home.raw_dir_of(None), home.raw_dir_of(Some("nas")));
        fs::create_dir_all(&raw).unwrap();
        fs::create_dir_all(&nas).unwrap();
        let files = [
            raw.join(format!(".incoming-{dead}")),
            raw.join(format!(".incoming-{own}")),
            raw.join(".incoming-x"),
            nas.join(format!(".pulling-{dead}")),
            nas.join(format!(".manifest.jsonl.tmp-{dead}")),
            nas.join("manifest.jsonl"),
        ];
        for f in &files {
            fs::write(f, b"half").unwrap();
        }
        assert_eq!(clear_staging(&home), 3);
        let left: Vec<bool> = files.iter().map(|f| f.exists()).collect();
        assert_eq!(left, [false, true, true, false, false, true]);
        // The sweep clears it once it holds the archive, and a stray file where the spokes' folders
        // are doesn't stop it.
        fs::write(home.machines_dir().join("notes"), b"x").unwrap();
        fs::write(&files[0], b"half").unwrap();
        fs::write(&files[4], b"half").unwrap();
        // No harness on, so it sweeps nothing of this machine's.
        let none = crate::Config {
            harnesses: vec![],
            ..crate::Config::default()
        };
        crate::sweep::sweep(&home, &none, None).unwrap();
        assert!(!files[0].exists());
        assert!(!files[4].exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_line_naming_a_blob_already_here_waits_for_its_folder_to_sync() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let spoke = Home::new(dir.path().join("spoke"));
        let hub = Home::new(dir.path().join("hub"));
        let mut raw = Archive::open(&spoke).unwrap();
        let first = raw
            .put(Harness::Codex, "s", "1", &b"first"[..])
            .unwrap()
            .unwrap();
        // An earlier pull renamed the blob in, then failed to sync its folder, which still can't be.
        let shard = hub.raw_dir_of(Some("nas")).join(&first.capture_hash[..2]);
        fs::create_dir_all(&shard).unwrap();
        fs::write(shard.join(&first.capture_hash), b"first").unwrap();
        fs::set_permissions(&shard, fs::Permissions::from_mode(0o300)).unwrap();
        // The spoke appends meanwhile, so the retry gets the first line without its blob.
        raw.put(Harness::Codex, "s", "2", &b"first, then more"[..])
            .unwrap()
            .unwrap();
        let mut export = Vec::new();
        crate::archive::export(&spoke, "nas", 0, &mut export, &mut Default::default()).unwrap();
        let took = take(&hub, "nas", &mut std::io::Cursor::new(export), &|| true);
        fs::set_permissions(&shard, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(took.is_err(), "{took:?}");
        assert!(!hub.manifest_path_of(Some("nas")).exists());
    }

    #[test]
    fn a_name_that_isnt_a_machines_reads_no_copy() {
        let (_h, hub) = home();
        for bad in ["../x", "/etc", ""] {
            assert!(copy_of(&hub, bad).is_err(), "{bad}");
            assert!(take(&hub, bad, &mut &b""[..], &|| false).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_check_reads_the_header_against_the_copy_it_asked_from_while_a_pull_grows_it() {
        let (_s, spoke) = home();
        let (_h, hub) = home();
        let mut a = Archive::open(&spoke).unwrap();
        a.put(Harness::Pi, "s", "1", &b"one\n"[..]).unwrap();
        // The check asks for an export from the copy as it is (empty); a pull lands first.
        let asked = copy_of(&hub, "nas").unwrap();
        let export = export_for(&spoke, &hub);
        pull(&spoke, &hub);
        assert!(check_export("nas", &asked, &mut &export[..]).unwrap());
        // Against the copy as it is now, the same header doesn't continue it.
        let now = copy_of(&hub, "nas").unwrap();
        let e = check_export("nas", &now, &mut &export[..]).unwrap_err();
        assert!(format!("{e:#}").contains("does not continue"), "{e:#}");
        // And with nothing new on the spoke, the check says so.
        let none = export_for(&spoke, &hub);
        assert!(!check_export("nas", &now, &mut &none[..]).unwrap());
    }

    #[test]
    fn a_blob_an_append_dropped_and_another_origin_brought_back_comes_again() {
        let (_s, spoke) = home();
        let (_h, hub) = home();
        let mut a = Archive::open(&spoke).unwrap();
        // `x` holds the bytes, grows past them (they are dropped), then `y` holds them again.
        a.put(Harness::Pi, "x", "1", &b"same\n"[..]).unwrap();
        a.put(Harness::Pi, "x", "2", &b"same\nmore\n"[..]).unwrap();
        a.put(Harness::Pi, "y", "1", &b"same\n"[..]).unwrap();
        // Unchanged bytes with a new change signal: the same blob named again, not sent again.
        a.put(Harness::Pi, "y", "2", &b"same\n"[..]).unwrap();
        let t = pull(&spoke, &hub);
        assert_eq!((t.lines, t.blobs), (4, 3), "{t:?}");
        assert_eq!(files(&spoke.raw_dir()), files(&hub.raw_dir_of(Some("nas"))));
        let nas = Archive::open_of(&hub, Some("nas"))
            .unwrap()
            .verify()
            .unwrap();
        assert!(nas.ok(), "{nas:?}");
    }

    #[test]
    fn a_blob_another_origin_names_again_within_one_pull_is_kept() {
        let (_s, spoke) = home();
        let (_h, hub) = home();
        let mut a = Archive::open(&spoke).unwrap();
        a.put(Harness::Pi, "x", "1", &b"same\n"[..]).unwrap();
        pull(&spoke, &hub);
        // `x` grows past the bytes the hub has, `y` holds them again, and `x` grows once more.
        a.put(Harness::Pi, "x", "2", &b"same\nmore\n"[..]).unwrap();
        a.put(Harness::Pi, "y", "1", &b"same\n"[..]).unwrap();
        a.put(Harness::Pi, "x", "3", &b"same\nmore\nagain\n"[..])
            .unwrap();
        let t = pull(&spoke, &hub);
        assert_eq!(t.lines, 3, "{t:?}");
        let nas = Archive::open_of(&hub, Some("nas"))
            .unwrap()
            .verify()
            .unwrap();
        assert!(nas.ok(), "{nas:?}");
        assert_eq!(files(&spoke.raw_dir()), files(&hub.raw_dir_of(Some("nas"))));
    }

    #[test]
    fn what_an_append_drops_here_is_dropped_only_once_its_replacement_is_in_place() {
        let (_s, spoke) = home();
        let (_h, hub) = home();
        let mut a = Archive::open(&spoke).unwrap();
        a.put(Harness::Pi, "s", "1", &b"a\n"[..]).unwrap();
        pull(&spoke, &hub);
        // Two appends: the first one's blob is gone on the spoke, so its line comes alone.
        a.put(Harness::Pi, "s", "2", &b"a\nb\n"[..]).unwrap();
        a.put(Harness::Pi, "s", "3", &b"a\nb\nc\n"[..]).unwrap();
        let before = fs::read(hub.manifest_path_of(Some("nas"))).unwrap();
        // Cut after the lone line: nothing of it goes in, and the old capture stays readable.
        let bytes = export_for(&spoke, &hub);
        let cut = bytes.windows(5).position(|w| w == b"blob ").unwrap();
        let err = take(&hub, "nas", &mut &bytes[..cut], &|| false).unwrap_err();
        assert!(format!("{err:#}").contains("ended early"), "{err:#}");
        assert_eq!(fs::read(hub.manifest_path_of(Some("nas"))).unwrap(), before);
        let nas = Archive::open_of(&hub, Some("nas"))
            .unwrap()
            .verify()
            .unwrap();
        assert!(nas.ok(), "{nas:?}");
        // A pull told to stop at once stops only after both lines, with the newest in place.
        let bytes = export_for(&spoke, &hub);
        let t = take(&hub, "nas", &mut &bytes[..], &|| true).unwrap();
        assert!(t.stopped && t.lines == 2, "{t:?}");
        let nas = Archive::open_of(&hub, Some("nas"))
            .unwrap()
            .verify()
            .unwrap();
        assert!(nas.ok(), "{nas:?}");
        assert_eq!(files(&spoke.raw_dir()), files(&hub.raw_dir_of(Some("nas"))));
    }

    #[test]
    fn a_pull_counts_what_it_kept_before_a_frame_it_refuses() {
        let (_s, spoke) = home();
        let (_h, hub) = home();
        let mut a = Archive::open(&spoke).unwrap();
        a.put(Harness::Pi, "s", "1", &b"one\n"[..]).unwrap();
        let mut bytes = export_for(&spoke, &hub);
        assert!(bytes.ends_with(b"end\n"));
        bytes.truncate(bytes.len() - 4);
        let good = bytes.clone();
        // A line naming a blob by a hash too short to be one.
        let line = fs::read_to_string(spoke.manifest_path_of(None)).unwrap();
        let mut rec: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        rec["sha256"] = "a".into();
        let rec = serde_json::to_vec(&rec).unwrap();
        let short = [format!("line {}\n", rec.len()).into_bytes(), rec].concat();
        for (bad, why) in [
            (b"line 4294967296\n".to_vec(), "longer than any capture"),
            (short, "not a blob name"),
        ] {
            let mut stream = good.clone();
            stream.extend_from_slice(&bad);
            let (_h2, fresh) = home();
            let mut t = Transfer::default();
            let err = take_into(&fresh, "nas", &mut &stream[..], &|| false, &mut t).unwrap_err();
            assert!(format!("{err:#}").contains(why), "{why}: {err:#}");
            assert_eq!((t.lines, t.blobs), (1, 1), "{t:?}");
        }
    }

    #[test]
    fn a_line_cut_short_here_is_dropped_and_sent_again_whole() {
        let (_s, spoke) = home();
        let (_h, hub) = home();
        let mut a = Archive::open(&spoke).unwrap();
        a.put(Harness::Pi, "s", "1", &b"one\n"[..]).unwrap();
        pull(&spoke, &hub);
        a.put(Harness::Pi, "t", "1", &b"two\n"[..]).unwrap();
        // A write that failed partway (a full disk) left half a line.
        let manifest = hub.manifest_path_of(Some("nas"));
        let mut f = fs::OpenOptions::new().append(true).open(&manifest).unwrap();
        f.write_all(b"{\"source\":\"pi\",\"ori").unwrap();
        let t = pull(&spoke, &hub);
        assert_eq!(t.lines, 1, "{t:?}");
        assert_eq!(files(&spoke.raw_dir()), files(&hub.raw_dir_of(Some("nas"))));
    }

    #[test]
    fn a_pull_that_stops_or_is_cut_keeps_what_it_took_and_the_next_goes_on() {
        let (_s, spoke) = home();
        let (_h, hub) = home();
        let mut a = Archive::open(&spoke).unwrap();
        for (origin, said) in [("s", "one\n"), ("t", "two\n"), ("u", "three\n")] {
            a.put(Harness::Pi, origin, "1", said.as_bytes()).unwrap();
        }
        // Stopped after its first line.
        let bytes = export_for(&spoke, &hub);
        let t = take(&hub, "nas", &mut &bytes[..], &|| true).unwrap();
        assert!(t.stopped && t.lines == 1, "{t:?}");
        // Cut in the middle of the last line, after its blob: the one before it is taken.
        let bytes = export_for(&spoke, &hub);
        let cut = bytes.len() - b"end\n".len() - 20;
        let err = take(&hub, "nas", &mut &bytes[..cut], &|| false).unwrap_err();
        assert!(format!("{err:#}").contains("ended early"), "{err:#}");
        // The next pull resends what wasn't taken, and skips the blob already in place.
        let t = pull(&spoke, &hub);
        assert!(t.present == 1 && t.blobs == 0 && t.lines == 1, "{t:?}");
        assert_eq!(files(&spoke.raw_dir()), files(&hub.raw_dir_of(Some("nas"))));
    }

    #[test]
    fn a_pull_refuses_another_machine_a_copy_it_does_not_continue_and_a_changed_blob() {
        let (_s, spoke) = home();
        let (_o, other) = home();
        let (_h, hub) = home();
        Archive::open(&spoke)
            .unwrap()
            .put(Harness::Pi, "s", "1", &b"one\n"[..])
            .unwrap();
        Archive::open(&other)
            .unwrap()
            .put(Harness::Pi, "x", "1", &b"elsewhere\n"[..])
            .unwrap();
        let mut bytes = Vec::new();
        export(&spoke, "box", 0, &mut bytes, &mut Transfer::default()).unwrap();
        let err = take(&hub, "nas", &mut &bytes[..], &|| false).unwrap_err();
        assert!(
            format!("{err:#}").contains("is \"box\", not \"nas\""),
            "{err:#}"
        );

        pull(&spoke, &hub);
        // Another archive that also calls itself nas, at the same length or not.
        let since = fs::metadata(hub.manifest_path_of(Some("nas")))
            .unwrap()
            .len();
        let mut bytes = Vec::new();
        match export(&other, "nas", since, &mut bytes, &mut Transfer::default()) {
            Ok(_) => {
                let err = take(&hub, "nas", &mut &bytes[..], &|| false).unwrap_err();
                assert!(format!("{err:#}").contains("does not continue"), "{err:#}");
            }
            Err(e) => assert!(format!("{e:#}").contains("not of this machine's archive")),
        }

        // A blob changed on the way is refused, and its line is not taken.
        Archive::open(&spoke)
            .unwrap()
            .put(Harness::Pi, "s", "2", &b"one\ntwo\n"[..])
            .unwrap();
        let mut bytes = export_for(&spoke, &hub);
        let at = bytes.windows(8).position(|w| w == b"one\ntwo\n").unwrap();
        bytes[at] = b'O';
        let before = fs::read(hub.manifest_path_of(Some("nas"))).unwrap();
        let err = take(&hub, "nas", &mut &bytes[..], &|| false).unwrap_err();
        assert!(
            format!("{err:#}").contains("does not hash to its name"),
            "{err:#}"
        );
        assert_eq!(fs::read(hub.manifest_path_of(Some("nas"))).unwrap(), before);
    }

    #[test]
    fn append_replaces_and_rewrite_keeps_both() {
        let (_d, home) = home();
        let mut a = Archive::open(&home).unwrap();
        let k = Harness::Pi;
        let r1 = a.put(k, "s", "1", &b"line1\n"[..]).unwrap().unwrap();
        assert_eq!(r1.kind, CaptureKind::New);
        assert!(
            a.put(k, "s", "1", &b"line1\n"[..]).unwrap().is_none(),
            "unchanged bytes"
        );

        let r2 = a.put(k, "s", "2", &b"line1\nline2\n"[..]).unwrap().unwrap();
        assert_eq!(r2.kind, CaptureKind::Append);
        assert!(
            !a.blob_path(&r1.capture_hash).exists(),
            "appended-over blob is dropped"
        );

        let r3 = a.put(k, "s", "3", &b"compacted\n"[..]).unwrap().unwrap();
        assert_eq!(r3.kind, CaptureKind::Rewrite);
        assert!(
            a.blob_path(&r2.capture_hash).exists(),
            "rewrite keeps the old version"
        );
        assert!(a.blob_path(&r3.capture_hash).exists());

        let reopened = Archive::open(&home).unwrap();
        assert_eq!(reopened.latest()["s"].capture_hash, r3.capture_hash);
        let v = reopened.verify().unwrap();
        assert!(v.ok(), "{v:?}");
        assert_eq!(v.checked, 2);
    }

    #[test]
    fn a_manifest_and_capture_written_by_the_previous_build_are_read() {
        let (_d, home) = home();
        // Exactly what the previous build wrote: a blob at `raw/<aa>/<sha256>` and a manifest
        // line with its field names (synthetic content).
        let bytes = b"{\"type\":\"session\",\"id\":\"p1\"}\n";
        let sha = crate::util::sha256_hex(bytes);
        fs::create_dir_all(home.raw_dir().join(&sha[..2])).unwrap();
        fs::write(home.raw_dir().join(&sha[..2]).join(&sha), bytes).unwrap();
        let line = format!(
            r#"{{"source":"pi","origin":"/u/.pi/agent/sessions/p1.jsonl","sha256":"{sha}","size":{},"kind":"new","fingerprint":"{}:1794436930000000000","captured_at":"2026-11-14T10:00:00Z"}}"#,
            bytes.len(),
            bytes.len()
        );
        fs::write(home.manifest_path(), format!("{line}\n")).unwrap();

        let mut a = Archive::open(&home).unwrap();
        let rec = &a.latest()["/u/.pi/agent/sessions/p1.jsonl"];
        assert_eq!(rec.harness, Harness::Pi);
        assert_eq!(rec.capture_hash, sha);
        assert_eq!(
            rec.change_signal,
            format!("{}:1794436930000000000", bytes.len())
        );
        assert_eq!(a.read_blob(&sha).unwrap(), bytes);
        assert!(a.verify().unwrap().ok());

        // A new capture appends a line in the same format, leaving the old line as it was.
        a.put(
            Harness::Pi,
            "/u/.pi/agent/sessions/p2.jsonl",
            "9:9",
            &b"x\n"[..],
        )
        .unwrap()
        .unwrap();
        let manifest = fs::read_to_string(home.manifest_path()).unwrap();
        let lines: Vec<&str> = manifest.lines().collect();
        assert_eq!(lines[0], line);
        let added: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        let mut keys: Vec<&str> = added
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "captured_at",
                "fingerprint",
                "kind",
                "origin",
                "sha256",
                "size",
                "source"
            ]
        );
        assert_eq!(added["source"], "pi");
    }

    #[test]
    fn copying_a_locked_database_fails_instead_of_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("live.db");
        let writer = rusqlite::Connection::open(&db).unwrap();
        writer
            .execute_batch(
                "PRAGMA journal_mode=DELETE; CREATE TABLE t(x); INSERT INTO t VALUES (1);",
            )
            .unwrap();
        assert!(
            !copy_sqlite(&db).unwrap().is_empty(),
            "an idle database copies"
        );
        writer
            .execute_batch("BEGIN EXCLUSIVE; INSERT INTO t VALUES (2);")
            .unwrap();
        let started = std::time::Instant::now();
        let err = copy_sqlite(&db).unwrap_err();
        assert!(
            err.to_string().contains("locked") || err.to_string().contains("busy"),
            "{err:#}"
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(30));
        writer.execute_batch("COMMIT;").unwrap();
    }

    #[test]
    fn verify_reports_corruption() {
        let (_d, home) = home();
        let mut a = Archive::open(&home).unwrap();
        let r = a
            .put(Harness::Pi, "s", "1", &b"hello"[..])
            .unwrap()
            .unwrap();
        fs::write(a.blob_path(&r.capture_hash), b"tampered").unwrap();
        let v = Archive::open(&home).unwrap().verify().unwrap();
        assert_eq!(v.corrupt, vec![r.capture_hash]);
    }
}
