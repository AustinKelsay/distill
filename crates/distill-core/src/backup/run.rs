//! A backup run (ADR 0009, issue #143): copy what one target lacks of the backup set, encrypted.
//!
//! 1. **Blobs** the target lacks, each sealed and named `name("blob/<capture hash>")`.
//! 2. **Manifest lines** the target lacks, as one new object,
//!    `name("manifest/<from>-<to>/<sha256>")`.
//! 3. **Every other file** of the backup set ([`Home::keep`]) whose content the target lacks,
//!    named `name("file/<path>/<sha256>")`.
//! 4. **The run's record**, under [`run_name`] for the next `n`: every manifest object in
//!    order and the object holding each file now. It is written last, so a run cut short leaves
//!    no record and the next run redoes it; a restore reads only the newest record.
//!
//! Each run adds a line to the backup log. Nothing on the target is overwritten or deleted.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::BackupKey;
use super::store::{self, Store};
use crate::archive::{Archive, expected_blobs, parse_manifest};
use crate::config::BackupTarget;
use crate::home::{Home, Keep};
use crate::tick::Deadline;
use crate::util::{append_jsonl, count, now_rfc3339, private_file};

/// Version 2 stores the logs in parts (`FileRef::parts`); a version 1 record still reads.
const RECORD_VERSION: u32 = 2;

/// What a run stored, written last and read by a restore.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub version: u32,
    pub number: u64,
    pub at: String,
    /// The manifest objects, oldest first; together they hold its first `lines` lines.
    pub manifest: Vec<ManifestPart>,
    pub lines: u64,
    /// The SHA-256 of those lines, so a later run can tell this home's manifest from another's.
    pub lines_sha256: String,
    /// Each other file of the backup set, by its path in the home.
    pub files: BTreeMap<String, FileRef>,
    /// When a run here first found the read log rewritten (issue #161), carried by every run
    /// after it, so a backup log line that could not be written, or was taken out, is put back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_log_rewritten: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestPart {
    pub object: String,
    pub from: u64,
    pub to: u64,
    /// How many runs' lines the part holds ([`merge_tail`]).
    #[serde(default = "one")]
    pub runs: u64,
}

fn one() -> u64 {
    1
}

/// How many of the newest parts, the newest being one run's, a run stores again as one: the
/// newest joins the part before it while that holds as many runs, as a binary counter carries.
/// So a record lists at most log2(runs) + 1 parts of each, and each line or byte is stored again
/// at most that many times: a record that listed one part per run would grow with every run,
/// and the records together with the square of the run count.
fn merge_tail(runs: impl DoubleEndedIterator<Item = u64>) -> usize {
    let mut runs = runs.rev();
    let mut held = runs.next().unwrap_or(0);
    let mut merged = 1;
    for r in runs {
        if r != held {
            break;
        }
        held += r;
        merged += 1;
    }
    merged
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileRef {
    /// The object holding the whole file; empty for a log held in `parts`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub object: String,
    /// The SHA-256 of the file as a restore writes it.
    pub sha256: String,
    /// An append-only log (the read log, the backup log): what each run appended, oldest first,
    /// together the log's first `len` bytes, its whole lines. Stored whole on every run, a log
    /// that grows each tick would cost the square of the run count.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<LogPart>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub len: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Bytes `from..to` of a log, as one object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogPart {
    pub object: String,
    pub from: u64,
    pub to: u64,
    pub sha256: String,
    /// How many runs' appends the part holds ([`merge_tail`]).
    #[serde(default = "one")]
    pub runs: u64,
}

/// What a run did, as one line of the backup log.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupReport {
    pub at: String,
    pub target: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The run's number at the target.
    pub run: u64,
    pub blobs_sent: u64,
    pub manifest_lines_sent: u64,
    pub files_sent: u64,
    /// Bytes stored at the target, encrypted.
    pub bytes_sent: u64,
    /// Blobs the manifest says the archive holds that neither it nor the target had: lost on
    /// this machine before any backup took them (`distill verify` names them).
    pub blobs_missing: u64,
    /// The manifest lines the run covers, from the first: the captures the target holds once
    /// the run is whole. Status counts the captures behind from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_lines: Option<u64>,
    /// A run of the tick's that reached the tick's time limit before or while sending blobs: it
    /// wrote no record and is not in the backup log, and the next run goes on from the blobs it
    /// sent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopped_at_deadline: bool,
    /// Not a run: the owner made a new backup key ([`BackupKey::replace`]), so every target
    /// starts over. The runs above it were made under an older key.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub new_key: bool,
    /// When a run to this target first found the read log no longer starting with what the run
    /// before stored of it, or gone: something rewrote it in between (issue #161). That run
    /// stored it again from its start; every run after it says so again ([`RunRecord`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_log_rewritten: Option<String>,
}

/// Whether `target` has had a whole run under any key: then a missing `distill-backup/` there
/// means the drive is away, and no run makes it again.
pub(crate) fn backed_up_before(home: &Home, target: &str) -> Result<bool> {
    let log: Vec<BackupReport> = crate::util::read_jsonl(&home.backup_log_path())?;
    Ok(log.iter().any(|l| l.ok && l.target == target))
}

/// The backup log's runs under this home's backup key: the lines after the last new key.
pub fn log(home: &Home) -> Result<Vec<BackupReport>> {
    let mut log: Vec<BackupReport> = crate::util::read_jsonl(&home.backup_log_path())?;
    if let Some(i) = log.iter().rposition(|l| l.new_key) {
        log.drain(..=i);
    }
    Ok(log)
}

impl BackupReport {
    pub fn summary(&self) -> String {
        match &self.error {
            Some(e) => format!(
                "Backup to {} (run {}) failed: {e} It stored {}, {}, {}; {}.",
                self.target,
                self.run,
                count(self.blobs_sent, "new blob"),
                count(self.manifest_lines_sent, "manifest line"),
                count(self.files_sent, "file"),
                size(self.bytes_sent)
            ),
            None if self.stopped_at_deadline => format!(
                "Backup to {} (run {}) stopped at the tick's time limit after {} ({}); the next tick goes on.",
                self.target,
                self.run,
                count(self.blobs_sent, "new blob"),
                size(self.bytes_sent)
            ),
            None => format!(
                "Backed up to {} (run {}): {}, {}, {}; {} stored, encrypted.",
                self.target,
                self.run,
                count(self.blobs_sent, "new blob"),
                count(self.manifest_lines_sent, "manifest line"),
                count(self.files_sent, "file"),
                size(self.bytes_sent)
            ),
        }
    }
}

/// `812 bytes`, `4.2 MB`.
fn size(n: u64) -> String {
    match n {
        0..1_000 => count(n, "byte"),
        1_000..1_000_000 => format!("{:.1} KB", n as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1} MB", n as f64 / 1e6),
        _ => format!("{:.1} GB", n as f64 / 1e9),
    }
}

/// Back up `home` to `target`, and log the run either way. A run that failed, or could not be
/// logged, is a report with `ok` false and its error.
pub fn run(home: &Home, target: &BackupTarget) -> Result<BackupReport> {
    let backup = home.lock("backup")?;
    // A sweep waits while a run reads the archive, so no blob the run's manifest lines need is
    // dropped under it by a longer append.
    let archive = home.lock_archive()?;
    run_locked(home, target, None, (backup, archive))
}

/// Back up as the tick does: never making a target's folder, nor starting over at one that had a
/// run of this home's, starting
/// nothing once `deadline` has passed and sending no more blobs after it (the blobs sent stay,
/// and the next run goes on from them). A run stopped so wrote no record and is not logged.
/// `None` when another run or a sweep holds a lock it needs.
pub(crate) fn run_scheduled(
    home: &Home,
    target: &BackupTarget,
    deadline: Deadline,
) -> Result<Option<BackupReport>> {
    let stopped = || BackupReport {
        at: now_rfc3339(),
        target: target.name.clone(),
        stopped_at_deadline: true,
        ..Default::default()
    };
    if deadline.passed() {
        return Ok(Some(stopped()));
    }
    let Some(backup) = home.try_lock("backup")? else {
        return Ok(None);
    };
    let Some(archive) = home.try_lock("archive")? else {
        return Ok(None);
    };
    run_locked(home, target, Some(deadline), (backup, archive)).map(Some)
}

/// The run, under the backup and archive locks, held until its line is in the backup log.
fn run_locked(
    home: &Home,
    target: &BackupTarget,
    scheduled: Option<Deadline>,
    _locks: (fs::File, fs::File),
) -> Result<BackupReport> {
    let mut r = BackupReport {
        at: now_rfc3339(),
        target: target.name.clone(),
        ..Default::default()
    };
    let result = run_into(home, target, &mut r, scheduled);
    r.ok = result.is_ok() && !r.stopped_at_deadline;
    r.error = result.as_ref().err().map(|e| format!("{e:#}"));
    if r.stopped_at_deadline && r.error.is_none() {
        return Ok(r);
    }
    if let Err(e) = append_jsonl(&home.backup_log_path(), &r) {
        // The run's outcome stands; failing to log it is a failure too.
        let what = r.error.take().map_or(String::new(), |e| format!("{e}; "));
        r.ok = false;
        r.stopped_at_deadline = false;
        r.error = Some(format!("{what}the backup log could not be written: {e:#}"));
    }
    Ok(r)
}

fn run_into(
    home: &Home,
    target: &BackupTarget,
    r: &mut BackupReport,
    scheduled: Option<Deadline>,
) -> Result<()> {
    let key = BackupKey::existing(home)?
        .context("this home has no backup key yet: run `distill backup init` first")?;
    let used = log(home)?.iter().any(|l| l.ok && l.target == target.name);
    let store = store::open(
        target,
        scheduled.is_none() && !backed_up_before(home, &target.name)?,
    )?;
    let present = store.list()?;
    let last_run = last_run(&key, &present);
    if scheduled.is_some() && used && last_run.is_none() {
        // Another folder under the same name, or a target that lost its runs: starting over is
        // the owner's to do.
        bail!(
            "backup target {:?} holds no run of this home's: run `distill backup run --target {}` to start it",
            target.name,
            target.name
        );
    }
    let number = last_run.map_or(0, |n| n + 1);
    r.run = number;
    let last = match last_run {
        Some(n) => Some(read_record(&*store, &key, n)?),
        None => None,
    };
    // A rewrite a run here saw before, said again, even by a run that fails from here on.
    r.read_log_rewritten = last.as_ref().and_then(|rec| rec.read_log_rewritten.clone());
    let scratch = Scratch::new(home)?;

    // The manifest's whole lines as they are now; lines appended later go in the next run.
    let manifest = match fs::read(home.manifest_path()) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).context("read the archive's manifest"),
    };
    let whole = manifest
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |i| i + 1);
    let manifest = &manifest[..whole];
    let starts: Vec<usize> = std::iter::once(0)
        .chain(
            manifest
                .iter()
                .enumerate()
                .filter(|(_, b)| **b == b'\n')
                .map(|(i, _)| i + 1),
        )
        .collect();
    let lines = (starts.len() - 1) as u64;
    r.manifest_lines = Some(lines);
    let (mut parts, sent_lines) = match &last {
        Some(rec) => {
            let upto = usize::try_from(rec.lines).unwrap_or(usize::MAX);
            if upto > starts.len() - 1
                || hex::encode(Sha256::digest(&manifest[..starts[upto]])) != rec.lines_sha256
            {
                bail!(
                    "backup target {:?} holds another home's archive, or this home's manifest \
                     lost lines; nothing was sent",
                    target.name
                );
            }
            (rec.manifest.clone(), rec.lines)
        }
        None => (Vec::new(), 0),
    };
    // A run of the tick's starts no upload once the tick's time is over, checked again after
    // the slow part of preparing one: the objects it stored stay, and the next run, finding
    // them, goes on from there.
    let late = || scheduled.is_some_and(|d| d.passed());
    macro_rules! stop_if_late {
        () => {
            if late() {
                r.stopped_at_deadline = true;
                return Ok(());
            }
        };
    }

    // 1. Blobs the lines this run covers say the archive holds, that the target lacks.
    let archive = Archive::open(home)?;
    for sha in expected_blobs(&parse_manifest(manifest)).keys() {
        let name = key.name(&format!("blob/{sha}"));
        if present.contains(&name) {
            continue;
        }
        stop_if_late!();
        let mut blob = match archive.open_blob(sha) {
            Ok(b) => b,
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                r.blobs_missing += 1;
                continue;
            }
            Err(e) => return Err(e),
        };
        let mut sealed = scratch.file()?;
        key.seal(&mut blob, &mut sealed)?;
        // A blob that doesn't hash to its name is corrupt here; don't send it as if it were good.
        blob.finish()?;
        stop_if_late!();
        r.bytes_sent += put(&*store, &name, &mut sealed)?;
        r.blobs_sent += 1;
    }

    // 2. The manifest lines the target lacks, as one more part, merged with the parts before it
    // as `merge_tail` says. A part the target lost is sent again from the same lines.
    // A version 1 record listed one part per run: the first run after it stores them as one.
    if last.as_ref().is_some_and(|l| l.version == 1) && parts.len() > 1 {
        let (from, to) = (parts[0].from, parts[parts.len() - 1].to);
        let bytes = &manifest[starts[from as usize]..starts[to as usize]];
        parts = vec![ManifestPart {
            object: key.name(&format!(
                "manifest/{from}-{to}/{}",
                hex::encode(Sha256::digest(bytes))
            )),
            from,
            to,
            runs: parts.iter().map(|p| p.runs).sum(),
        }];
    }
    if lines > sent_lines {
        parts.push(ManifestPart {
            object: String::new(),
            from: sent_lines,
            to: lines,
            runs: 1,
        });
        let tail = parts.split_off(parts.len() - merge_tail(parts.iter().map(|p| p.runs)));
        let (from, to) = (tail[0].from, lines);
        // Named by content too: another home with this key and the same earlier lines never
        // shares a part with this one.
        let bytes = &manifest[starts[from as usize]..starts[to as usize]];
        parts.push(ManifestPart {
            object: key.name(&format!(
                "manifest/{from}-{to}/{}",
                hex::encode(Sha256::digest(bytes))
            )),
            from,
            to,
            runs: tail.iter().map(|p| p.runs).sum(),
        });
    }
    for part in &parts {
        if !present.contains(&part.object) {
            stop_if_late!();
            let lines = &manifest[starts[part.from as usize]..starts[part.to as usize]];
            let mut sealed = scratch.file()?;
            key.seal(lines, &mut sealed)?;
            stop_if_late!();
            r.bytes_sent += put(&*store, &part.object, &mut sealed)?;
        }
    }
    r.manifest_lines_sent = lines - sent_lines;

    // 3. Every other file of the backup set; the logs as what was appended since.
    let logs = [home.read_log_path(), home.backup_log_path()].map(|p| {
        p.strip_prefix(&home.root)
            .expect("under the home")
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/")
    });
    let mut files = BTreeMap::new();
    for entry in walkdir::WalkDir::new(&home.root).sort_by_file_name() {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        // Joined from its parts, so a `\\` in a name stays part of that name. DISTILL names
        // everything in the home; a name that isn't UTF-8 is refused, not changed into another.
        let parts: Option<Vec<&str>> = entry
            .path()
            .strip_prefix(&home.root)
            .expect("under the home")
            .components()
            .map(|c| c.as_os_str().to_str())
            .collect();
        let Some(parts) = parts else {
            bail!(
                "can't back up {}: its name isn't UTF-8",
                entry.path().display()
            );
        };
        let rel = parts.join("/");
        let wanted = match Home::keep(&rel) {
            // This home's own raw archive went above, by its manifest's lines.
            Some(Keep::Always) => !rel.starts_with("raw/"),
            Some(Keep::Cards) => target.cards,
            Some(Keep::Vectors) => target.vectors,
            Some(Keep::Never) | None => false,
        };
        // The backup log as it stands: this run's own line is written after it, for the next.
        if !wanted {
            continue;
        }
        let bytes = fs::read(entry.path()).with_context(|| format!("read {rel}"))?;
        // A spoke's manifest grows by appends as a log does (ADR 0010).
        let spoke_manifest = rel.starts_with("machines/") && rel.ends_with("/raw/manifest.jsonl");
        if logs.contains(&rel) || spoke_manifest {
            // Only the whole lines, and of them only what the last run didn't cover, unless the
            // log no longer starts with what it covered.
            let whole = &bytes[..bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1)];
            let earlier = last
                .as_ref()
                .and_then(|rec| rec.files.get(&rel))
                .filter(|f| {
                    f.object.is_empty()
                        && usize::try_from(f.len).is_ok_and(|n| n <= whole.len())
                        && hex::encode(Sha256::digest(&whole[..f.len as usize])) == f.sha256
                });
            let stored = last.as_ref().and_then(|rec| rec.files.get(&rel));
            if rel == "log/reads.jsonl"
                && earlier.is_none()
                && stored.is_some_and(|f| f.object.is_empty())
            {
                r.read_log_rewritten.get_or_insert_with(|| r.at.clone());
            }
            let (mut parts, from) =
                earlier.map_or((Vec::new(), 0), |f| (f.parts.clone(), f.len as usize));
            if whole.len() > from {
                parts.push(LogPart {
                    object: String::new(),
                    from: from as u64,
                    to: whole.len() as u64,
                    sha256: String::new(),
                    runs: 1,
                });
                let tail = parts.split_off(parts.len() - merge_tail(parts.iter().map(|p| p.runs)));
                let (from, to) = (tail[0].from, whole.len() as u64);
                let sha = hex::encode(Sha256::digest(&whole[from as usize..]));
                parts.push(LogPart {
                    object: key.name(&format!("log/{rel}/{from}-{to}/{sha}")),
                    from,
                    to,
                    sha256: sha,
                    runs: tail.iter().map(|p| p.runs).sum(),
                });
            }
            // Each part the target lacks: the new one, or an earlier one it lost.
            for part in &parts {
                if !present.contains(&part.object) {
                    stop_if_late!();
                    let mut sealed = scratch.file()?;
                    key.seal(&whole[part.from as usize..part.to as usize], &mut sealed)?;
                    stop_if_late!();
                    r.bytes_sent += put(&*store, &part.object, &mut sealed)?;
                    r.files_sent += 1;
                }
            }
            files.insert(
                rel,
                FileRef {
                    object: String::new(),
                    sha256: hex::encode(Sha256::digest(whole)),
                    parts,
                    len: whole.len() as u64,
                },
            );
            continue;
        }
        let sha = hex::encode(Sha256::digest(&bytes));
        let name = key.name(&format!("file/{rel}/{sha}"));
        if !present.contains(&name) {
            stop_if_late!();
            let mut sealed = scratch.file()?;
            key.seal(bytes.as_slice(), &mut sealed)?;
            stop_if_late!();
            r.bytes_sent += put(&*store, &name, &mut sealed)?;
            r.files_sent += 1;
        }
        files.insert(
            rel,
            FileRef {
                object: name,
                sha256: sha,
                parts: Vec::new(),
                len: 0,
            },
        );
    }

    // A read log the run before stored and that is gone now was taken away, not rewritten in
    // place, and the next run would compare with nothing.
    if !files.contains_key("log/reads.jsonl")
        && last
            .as_ref()
            .and_then(|rec| rec.files.get("log/reads.jsonl"))
            .is_some_and(|f| f.object.is_empty() && f.len > 0)
    {
        r.read_log_rewritten.get_or_insert_with(|| r.at.clone());
    }

    // 4. The record, last.
    stop_if_late!();
    let record = RunRecord {
        version: RECORD_VERSION,
        number,
        at: r.at.clone(),
        manifest: parts,
        lines,
        lines_sha256: hex::encode(Sha256::digest(manifest)),
        files,
        read_log_rewritten: r.read_log_rewritten.clone(),
    };
    let mut sealed = scratch.file()?;
    key.seal(serde_json::to_vec(&record)?.as_slice(), &mut sealed)?;
    let name = run_name(&key, number);
    store.sync()?;
    sealed.seek(SeekFrom::Start(0))?;
    // A sync can be slow; a record that would start after the deadline waits for the next run.
    stop_if_late!();
    if !store.put(&name, &mut sealed)? {
        bail!(
            "another backup run to {:?} finished meanwhile; nothing was lost",
            target.name
        );
    }
    // The record too: a target that makes objects durable only on a sync (an adapter's) could
    // otherwise lose the one object a restore starts from.
    store.sync()?;
    // Everything else is stored, but the archive itself lost bytes: that is a failure to see.
    if r.blobs_missing > 0 {
        bail!(
            "{} the manifest names {} missing from this archive, so this run lacks {}; \
             everything else was stored. Run `distill verify`.",
            count(r.blobs_missing, "blob"),
            if r.blobs_missing == 1 { "is" } else { "are" },
            if r.blobs_missing == 1 { "it" } else { "them" }
        );
    }
    Ok(())
}

/// The name run `n`'s record is stored under. Unlike other names the key can read it back: a
/// keyed value of `n` (40 hex), then `n` masked by a keyed value of that (16 hex), then a keyed
/// check (8 hex). [`last_run`] so finds the newest record from a listing alone, however many
/// objects a target lost, while to the target every name looks alike: it can't pick the records
/// out, or put them in order, without the key.
pub fn run_name(key: &BackupKey, n: u64) -> String {
    let lead = run_lead(key, n);
    let check = &key.name(&format!("run check/{n}"))[..8];
    format!("{lead}{:016x}{check}", n ^ run_mask(key, &lead))
}

fn run_lead(key: &BackupKey, n: u64) -> String {
    key.name(&format!("run/{n}"))[..40].to_string()
}

fn run_mask(key: &BackupKey, lead: &str) -> u64 {
    u64::from_str_radix(&key.name(&format!("run mask/{lead}"))[..16], 16).expect("hex")
}

/// The run number a name holds, if it is one of this key's run records.
fn run_number(key: &BackupKey, name: &str) -> Option<u64> {
    let (lead, masked) = (name.get(..40)?, name.get(40..56)?);
    let n = u64::from_str_radix(masked, 16).ok()? ^ run_mask(key, lead);
    (run_lead(key, n) == lead && run_name(key, n) == name).then_some(n)
}

/// The highest run number with a record at a target.
pub fn last_run(key: &BackupKey, present: &BTreeSet<String>) -> Option<u64> {
    present
        .iter()
        .filter_map(|name| run_number(key, name))
        .max()
}

/// The record of run `n` at a target.
pub fn read_record(store: &dyn Store, key: &BackupKey, n: u64) -> Result<RunRecord> {
    let mut sealed = Vec::new();
    store.get(&run_name(key, n), &mut sealed)?;
    let mut plain = Vec::new();
    key.open(sealed.as_slice(), &mut plain)
        .with_context(|| format!("open run {n}'s record"))?;
    let rec: RunRecord =
        serde_json::from_slice(&plain).map_err(|e| anyhow!("run {n}'s record: {e}"))?;
    if !(1..=RECORD_VERSION).contains(&rec.version) || rec.number != n {
        bail!("run {n}'s record is not one this version reads");
    }
    Ok(rec)
}

/// Store a sealed file from its start; the bytes stored, or 0 when the target had it already.
fn put(store: &dyn Store, name: &str, sealed: &mut fs::File) -> Result<u64> {
    let len = sealed.seek(SeekFrom::End(0))?;
    sealed.seek(SeekFrom::Start(0))?;
    Ok(if store.put(name, &mut (sealed as &mut dyn Read))? {
        len
    } else {
        0
    })
}

/// Owner-only files under `state/` to seal into before storing, removed when dropped.
pub(super) struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    pub(super) fn new(home: &Home) -> Result<Self> {
        let parent = home.state_dir();
        crate::util::create_dir_private(&parent)?;
        let mut b = tempfile::Builder::new();
        b.prefix("backup-");
        // Owner-only whatever the umask.
        #[cfg(unix)]
        b.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
        Ok(Self {
            dir: b
                .tempdir_in(&parent)
                .context("make a scratch folder for the backup")?,
        })
    }

    pub(super) fn file(&self) -> Result<fs::File> {
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|e| anyhow!("generate a file name: {e}"))?;
        let path = self.dir.path().join(hex::encode(nonce));
        let f = private_file()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("create {}", path.display()))?;
        // Unlinked at once: the open handle is all a run needs, and nothing is left behind.
        fs::remove_file(&path)?;
        Ok(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Harness;

    /// A home with two captures, a config and a read log, and a folder target beside it.
    fn setup() -> (tempfile::TempDir, Home, BackupTarget) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        let mut archive = Archive::open(&home).unwrap();
        archive
            .put(
                Harness::ClaudeCode,
                "/s/a.jsonl",
                "1",
                secret_session().as_bytes(),
            )
            .unwrap();
        archive
            .put(Harness::Codex, "/s/b.jsonl", "1", &b"another session\n"[..])
            .unwrap();
        crate::Config::default().save(&home.config_path()).unwrap();
        append_jsonl(
            &home.read_log_path(),
            &serde_json::json!({"reader": "owner"}),
        )
        .unwrap();
        BackupKey::create(&home).unwrap();
        fs::create_dir(dir.path().join("drive")).unwrap();
        let target = BackupTarget {
            name: "drive".into(),
            kind: "folder".into(),
            path: Some(dir.path().join("drive")),
            cards: false,
            vectors: false,
            settings: Default::default(),
        };
        (dir, home, target)
    }

    fn secret_session() -> String {
        format!(
            "deploy with {}\n",
            ["sk-", "live-", &"Zq9".repeat(8)].concat()
        )
    }

    #[test]
    fn the_logs_are_stored_as_what_each_run_appended() {
        let (_dir, home, target) = setup();
        let log: String = (0..2000)
            .map(|i| format!("{{\"reader\":\"owner\",\"i\":{i}}}\n"))
            .collect();
        fs::write(home.read_log_path(), log).unwrap();
        let first = run(&home, &target).unwrap();
        assert!(first.ok && first.bytes_sent > 50_000, "{first:?}");

        // One line more: the second run's part joins the first's, as one part of two runs.
        append_jsonl(&home.read_log_path(), &serde_json::json!({"reader": "x"})).unwrap();
        let second = run(&home, &target).unwrap();
        assert!(second.ok, "{second:?}");
        assert_eq!(
            second.files_sent, 2,
            "the read log and the backup log: {second:?}"
        );
        let key = BackupKey::existing(&home).unwrap().unwrap();
        let store = store::open(&target, false).unwrap();
        let rec = read_record(&*store, &key, 1).unwrap();
        let reads = &rec.files["log/reads.jsonl"];
        assert!(reads.object.is_empty(), "{reads:?}");
        assert_eq!(
            (reads.parts.len(), reads.parts[0].runs),
            (1, 2),
            "{reads:?}"
        );
        assert_eq!(reads.len, fs::metadata(home.read_log_path()).unwrap().len());
        assert_eq!(rec.files["log/backups.jsonl"].parts.len(), 1);

        // A part the target lost is sent again.
        let lost = &reads.parts[0].object;
        let objects = target.path.as_ref().unwrap().join("distill-backup");
        fs::remove_file(objects.join(&lost[..2]).join(lost)).unwrap();
        let third = run(&home, &target).unwrap();
        assert!(
            third.ok && store.list().unwrap().contains(lost),
            "{third:?}"
        );

        // Forty more runs, each after a capture and a read: each list stays at most log2(runs) + 1
        // parts long, so a record stays small, and a restore brings back each log whole.
        let mut archive = Archive::open(&home).unwrap();
        let mut sent = 0;
        for i in 0..40 {
            let text = format!("session {i}\n");
            archive
                .put(
                    Harness::Codex,
                    &format!("/s/{i}.jsonl"),
                    "1",
                    text.as_bytes(),
                )
                .unwrap();
            append_jsonl(&home.read_log_path(), &serde_json::json!({"reader": i})).unwrap();
            let r = run(&home, &target).unwrap();
            assert!(r.ok, "{r:?}");
            sent += r.bytes_sent;
        }
        // Whole copies would have stored the read log forty times again.
        assert!(sent < 8 * first.bytes_sent, "{sent} bytes for forty runs");
        let rec = read_record(&*store, &key, 42).unwrap();
        assert!(rec.manifest.len() <= 7, "{:?}", rec.manifest);
        for log in ["log/reads.jsonl", "log/backups.jsonl"] {
            assert!(rec.files[log].parts.len() <= 7, "{:?}", rec.files[log]);
        }
        let mut sealed = Vec::new();
        store.get(&run_name(&key, 42), &mut sealed).unwrap();
        assert!(sealed.len() < 6_000, "the record is {} bytes", sealed.len());
        let restored = Home::new(_dir.path().join("restored"));
        crate::backup::restore(&restored, &*store, &key, false).unwrap();
        for p in [
            Home::read_log_path,
            Home::backup_log_path,
            Home::manifest_path,
        ] {
            let (a, b) = (fs::read(p(&home)).unwrap(), fs::read(p(&restored)).unwrap());
            // The restored backup log lacks the newest run's own line.
            assert!(
                a.starts_with(&b) && b.len() + 400 > a.len(),
                "{}",
                p(&home).display()
            );
        }

        // A log that no longer starts with what was stored is stored over from its start, and
        // the run says the read log was rewritten (#161), as status does from then on.
        assert_eq!(crate::readlog::edited(&home).unwrap(), None);
        fs::write(home.read_log_path(), "{\"reader\":\"y\"}\n").unwrap();
        let rewritten = run(&home, &target).unwrap();
        assert!(
            rewritten.ok && rewritten.read_log_rewritten.as_ref() == Some(&rewritten.at),
            "{rewritten:?}"
        );
        let again = run(&home, &target).unwrap();
        // Every run after it says so again, from its record: a backup log line lost is put back.
        assert_eq!(again.read_log_rewritten, rewritten.read_log_rewritten);
        let log: Vec<String> = fs::read_to_string(home.backup_log_path())
            .unwrap()
            .lines()
            .filter(|l| !l.contains("read_log_rewritten"))
            .map(str::to_string)
            .collect();
        fs::write(home.backup_log_path(), log.join("\n") + "\n").unwrap();
        assert_eq!(crate::readlog::edited(&home).unwrap(), None);
        let after = run(&home, &target).unwrap();
        assert!(after.ok, "{after:?}");
        let said = crate::readlog::edited(&home).unwrap().unwrap();
        assert!(said.contains("the backup to drive at"), "{said}");
        // So does a read log taken away, at a target that had seen nothing.
        let other = BackupTarget {
            name: "other".into(),
            path: Some(_dir.path().join("other")),
            ..target.clone()
        };
        fs::create_dir(other.path.as_ref().unwrap()).unwrap();
        let first_there = run(&home, &other).unwrap();
        assert!(first_there.ok && first_there.read_log_rewritten.is_none());
        let kept = fs::read(home.read_log_path()).unwrap();
        fs::remove_file(home.read_log_path()).unwrap();
        let gone = run(&home, &other).unwrap();
        assert!(gone.ok && gone.read_log_rewritten.is_some(), "{gone:?}");
        fs::write(home.read_log_path(), kept).unwrap();
        let rec = read_record(&*store, &key, 43).unwrap();
        let reads = &rec.files["log/reads.jsonl"];
        assert_eq!(
            (reads.parts.len(), reads.parts[0].from, reads.len),
            (1, 0, 15),
            "{reads:?}"
        );
    }

    /// Every file under `dir`, read whole.
    fn stored(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
        walkdir::WalkDir::new(dir)
            .into_iter()
            .map(|e| e.unwrap())
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                (
                    e.file_name().to_string_lossy().to_string(),
                    fs::read(e.path()).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn a_run_stores_the_backup_set_encrypted_and_the_next_only_what_changed() {
        let (dir, home, target) = setup();
        let r = run(&home, &target).unwrap();
        assert_eq!(
            (r.run, r.blobs_sent, r.manifest_lines_sent, r.files_sent),
            (0, 2, 2, 2),
            "two blobs, two lines, the config and the read log: {r:?}"
        );
        assert!(r.ok && r.bytes_sent > 0);

        // Only ciphertext under keyed names: no capture hash, no session text, no path.
        let archive = Archive::open(&home).unwrap();
        let hashes: Vec<String> = archive
            .records()
            .iter()
            .map(|r| r.capture_hash.clone())
            .collect();
        let files = stored(&dir.path().join("drive"));
        assert_eq!(
            files.len(),
            2 + 1 + 2 + 1,
            "blobs, a manifest part, files, a record"
        );
        for (name, bytes) in &files {
            assert!(!hashes.contains(name), "{name} is a capture hash");
            for plain in [
                secret_session().trim().as_bytes(),
                b"another session",
                b"claude-code",
                b"config.toml",
            ] {
                assert!(
                    !bytes.windows(plain.len()).any(|w| w == plain),
                    "{name} holds {}",
                    String::from_utf8_lossy(plain)
                );
            }
        }

        // Nothing changed: a record, and nothing else.
        let r = run(&home, &target).unwrap();
        assert_eq!(
            (r.run, r.blobs_sent, r.manifest_lines_sent, r.files_sent),
            (1, 0, 0, 1),
            "only the backup log, which the first run wrote: {r:?}"
        );
        // A new capture: its blob and its manifest line.
        let mut archive = Archive::open(&home).unwrap();
        archive
            .put(Harness::Codex, "/s/c.jsonl", "1", &b"a third session\n"[..])
            .unwrap();
        let r = run(&home, &target).unwrap();
        assert_eq!(
            (r.run, r.blobs_sent, r.manifest_lines_sent),
            (2, 1, 1),
            "{r:?}"
        );
        let key = BackupKey::existing(&home).unwrap().unwrap();
        let store = store::open(&target, true).unwrap();
        let rec = read_record(&*store, &key, 2).unwrap();
        assert_eq!(rec.lines, 3);
        // Run 2's part, of one run, joined run 0's as one part of two runs.
        assert_eq!(
            rec.manifest
                .iter()
                .map(|p| (p.from, p.to, p.runs))
                .collect::<Vec<_>>(),
            [(0, 3, 2)]
        );
        assert!(rec.files.contains_key("config.toml") && rec.files.contains_key("log/reads.jsonl"));
        assert!(
            rec.files.contains_key("keys/fingerprint.key") || !home.fingerprint_key_path().exists()
        );
        assert!(
            !rec.files.keys().any(|k| k.starts_with("keys/backup")),
            "never the backup key"
        );

        let log: Vec<BackupReport> = crate::util::read_jsonl(&home.backup_log_path()).unwrap();
        assert_eq!(log.len(), 3);
        assert!(log.iter().all(|l| l.ok && l.target == "drive"));
    }

    #[test]
    fn a_missing_folder_or_key_or_another_homes_target_is_refused_and_logged() {
        let (dir, home, mut target) = setup();
        let drive = target.path.clone().unwrap();

        target.path = Some(dir.path().join("unplugged"));
        let err = run(&home, &target).unwrap().error.unwrap();
        assert!(err.contains("does not exist"), "{err}");
        assert!(
            !dir.path().join("unplugged").exists(),
            "not made on this disk"
        );

        target.path = Some(drive.clone());
        run(&home, &target).unwrap();
        // Once used, a target whose drive is gone (its mount point left empty) is refused.
        fs::rename(&drive, dir.path().join("away")).unwrap();
        fs::create_dir(&drive).unwrap();
        let err = run(&home, &target).unwrap().error.unwrap();
        assert!(err.contains("is the drive mounted"), "{err}");
        assert!(
            !drive.join("distill-backup").exists(),
            "not made on this disk"
        );
        fs::remove_dir(&drive).unwrap();
        fs::rename(dir.path().join("away"), &drive).unwrap();
        // Another home with the same key (a restored copy that then diverged) at the same target.
        let other = Home::new(dir.path().join("other"));
        other.ensure().unwrap();
        Archive::open(&other)
            .unwrap()
            .put(Harness::Codex, "/s/x.jsonl", "1", &b"something else\n"[..])
            .unwrap();
        fs::create_dir_all(other.keys_dir()).unwrap();
        fs::copy(home.backup_key_path(), other.backup_key_path()).unwrap();
        let err = run(&other, &target).unwrap().error.unwrap();
        assert!(err.contains("another home's archive"), "{err}");

        fs::remove_file(home.backup_key_path()).unwrap();
        let err = run(&home, &target).unwrap().error.unwrap();
        assert!(err.contains("no backup key"), "{err}");

        let log: Vec<BackupReport> = crate::util::read_jsonl(&home.backup_log_path()).unwrap();
        assert_eq!(
            log.iter().map(|l| l.ok).collect::<Vec<_>>(),
            [false, true, false, false]
        );
    }

    /// The path a folder target keeps the object `name` at.
    fn object(target: &BackupTarget, name: &str) -> std::path::PathBuf {
        let root = target.path.as_ref().unwrap().join("distill-backup");
        root.join(&name[..2]).join(name)
    }

    #[test]
    fn a_target_that_lost_an_object_is_repaired_and_never_numbered_backwards() {
        let (_dir, home, target) = setup();
        let key = BackupKey::existing(&home).unwrap().unwrap();
        let store = store::open(&target, true).unwrap();
        for n in 0..3 {
            assert_eq!(run(&home, &target).unwrap().run, n);
        }
        // A lost record in the middle: the next run still comes after the newest.
        fs::remove_file(object(&target, &run_name(&key, 1))).unwrap();
        assert_eq!(run(&home, &target).unwrap().run, 3);

        // A lost manifest part is sent again from this home's own lines.
        let part = read_record(&*store, &key, 3).unwrap().manifest[0]
            .object
            .clone();
        fs::remove_file(object(&target, &part)).unwrap();
        let r = run(&home, &target).unwrap();
        assert_eq!(r.manifest_lines_sent, 0, "{r:?}");
        let mut sealed = Vec::new();
        store.get(&part, &mut sealed).unwrap();
        let mut lines = Vec::new();
        key.open(sealed.as_slice(), &mut lines).unwrap();
        assert_eq!(lines, fs::read(home.manifest_path()).unwrap());

        // A target that lost all but its newest record still numbers after the runs this home
        // logged.
        let newest = run_name(&key, 4);
        for name in store.list().unwrap() {
            if name != newest {
                fs::remove_file(object(&target, &name)).unwrap();
            }
        }
        assert_eq!(run(&home, &target).unwrap().run, 5);
    }

    #[test]
    fn only_blobs_the_manifest_still_needs_are_sent_and_a_lost_one_is_named() {
        let (_dir, home, target) = setup();
        let mut archive = Archive::open(&home).unwrap();
        // A longer append drops the blob it supersedes: that one is not missing.
        archive
            .put(
                Harness::Codex,
                "/s/b.jsonl",
                "2",
                &b"another session\nmore\n"[..],
            )
            .unwrap();
        let blob = archive.blob_path(&archive.latest()["/s/a.jsonl"].capture_hash);
        let key = BackupKey::existing(&home).unwrap().unwrap();
        let store = store::open(&target, true).unwrap();

        // A blob that can't be read is not a missing one: the run stops before its record.
        // A link to itself: unreadable whoever runs the test.
        fs::remove_file(&blob).unwrap();
        std::os::unix::fs::symlink(&blob, &blob).unwrap();
        let err = run(&home, &target).unwrap().error.unwrap();
        assert!(!err.contains("missing from this archive"), "{err}");
        assert!(read_record(&*store, &key, 0).is_err());

        // A lost one fails the run too, once everything else is stored under its record.
        fs::remove_file(&blob).unwrap();
        let err = run(&home, &target).unwrap().error.unwrap();
        assert!(
            err.contains("1 blob the manifest names is missing"),
            "{err}"
        );
        let log: Vec<BackupReport> = crate::util::read_jsonl(&home.backup_log_path()).unwrap();
        let r = log.last().unwrap();
        let summary = r.summary();
        assert!(
            summary.contains("(run 0) failed") && summary.contains("3 files"),
            "{summary}"
        );
        assert_eq!((r.ok, r.blobs_missing), (false, 1), "{r:?}");
        assert_eq!(read_record(&*store, &key, 0).unwrap().lines, 3);
    }

    #[test]
    fn a_run_records_name_gives_its_number_back_to_its_own_key_only() {
        let dir = tempfile::tempdir().unwrap();
        let key = BackupKey::create(&Home::new(dir.path().join("a"))).unwrap();
        let other = BackupKey::create(&Home::new(dir.path().join("b"))).unwrap();
        let present = BTreeSet::from([
            run_name(&key, 3),
            run_name(&key, 70_000),
            run_name(&other, 90_000),
            key.name("blob/ab"),
            format!("{}{}", &run_name(&key, 7)[..40], "0".repeat(24)),
        ]);
        assert_eq!(
            last_run(&key, &present),
            Some(70_000),
            "no ceiling, no other key's"
        );
        assert_eq!(last_run(&other, &present), Some(90_000));
        assert_ne!(run_name(&key, 3), key.name("run/3"));
        assert_eq!(run_name(&key, 3).len(), 64);
        // Nothing in common between records that a target could sort or group them by.
        let (a, b) = (run_name(&key, 4), run_name(&key, 5));
        assert_ne!(a[..8], b[..8]);
        assert_ne!(a[56..], b[56..]);
    }
}
