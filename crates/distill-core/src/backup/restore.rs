//! Restore and verify (ADR 0009, issue #149). A restore is the only way a backup comes back in:
//! it reads a target's newest run record, checks every object against it, and writes the backup
//! set into an empty home, `raw/` through [`archive::restore`]. The tick rebuilds the rest.
//! Verify checks that a target still holds what its newest record names, by name or, with
//! `full`, by fetching and decrypting every object.

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::BackupKey;
use super::run::{RunRecord, Scratch, last_run, read_record};
use super::store::Store;
use crate::archive;
use crate::config::BackupTarget;
use crate::home::{Home, Keep};
use crate::util::{count, create_dir_private, set_private, sync_dir, write_atomic};

/// What a restore brought back.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RestoreReport {
    /// The run restored, and when it was made.
    pub run: u64,
    pub made_at: String,
    pub manifest_lines: u64,
    pub blobs: u64,
    /// Blobs the manifest names that the target does not hold.
    pub blobs_missing: u64,
    pub files: u64,
}

impl RestoreReport {
    pub fn summary(&self) -> String {
        let mut s = format!(
            "Restored backup run {} (made {}): {}, {}, {}.",
            self.run,
            self.made_at,
            count(self.manifest_lines, "manifest line"),
            count(self.blobs, "blob"),
            count(self.files, "other file"),
        );
        if self.blobs_missing > 0 {
            s.push_str(&format!(
                " {} the manifest names {} not at the target.",
                count(self.blobs_missing, "blob"),
                if self.blobs_missing == 1 {
                    "was"
                } else {
                    "were"
                }
            ));
        }
        s.push_str(" The next `distill tick` rebuilds sessions, findings and the index.");
        s
    }
}

/// The target a restore reads, from what the owner gives without a home to read it from: a
/// folder target's `folder`, or an adapter target's `kind` and its `settings`, each `key=value`
/// as its `[[backup]]` table would have it (the value read as TOML, such as `22`, `true` or
/// `"text"`, or else as text).
pub fn restore_target(
    folder: Option<PathBuf>,
    kind: Option<&str>,
    settings: &[String],
) -> Result<BackupTarget> {
    let settings = settings
        .iter()
        .map(|s| {
            let Some((key, value)) = s.split_once('=') else {
                bail!("give a setting as key=value, not {s:?}");
            };
            let value = toml::from_str::<toml::Table>(&format!("v = {value}"))
                .ok()
                .and_then(|mut t| t.remove("v"))
                .unwrap_or_else(|| toml::Value::String(value.to_string()));
            Ok((key.trim().to_string(), value))
        })
        .collect::<Result<_>>()?;
    Ok(BackupTarget {
        name: "restore".into(),
        kind: kind.unwrap_or("folder").into(),
        path: folder,
        cards: false,
        vectors: false,
        settings,
    })
}

/// Restore the newest backup run at `store` into `home`, which must be empty or not exist. The
/// home is built beside it and moved into place only when everything checked out, so a failed
/// restore leaves nothing to clean up. A blob the manifest names that the target lacks fails it,
/// unless `allow_missing`: then the rest comes back and the report counts what did not.
pub fn restore(
    home: &Home,
    store: &dyn Store,
    key: &BackupKey,
    allow_missing: bool,
) -> Result<RestoreReport> {
    let home = &Home::new(std::path::absolute(&home.root).context("find the home's folder")?);
    if let Ok(mut entries) = fs::read_dir(&home.root)
        && entries.next().is_some()
    {
        bail!(
            "{} is not empty: a restore goes only into an empty home",
            home.root.display()
        );
    }
    let present = store.list()?;
    let rec = newest(store, key, &present)?;
    let parent = home
        .root
        .parent()
        .context("the home has no parent folder")?;
    // Made by the owner, not here: a folder this made above the home could be lost in a crash
    // after the restore said it was done.
    if !parent.is_dir() {
        bail!("{} does not exist; make it first", parent.display());
    }
    let mut b = tempfile::Builder::new();
    b.prefix(".distill-restore-");
    #[cfg(unix)]
    b.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
    let staging = b
        .tempdir_in(parent)
        .context("make a folder to restore into")?;
    let stage = Home::new(staging.path());
    stage.ensure()?;
    let mut report = RestoreReport {
        run: rec.number,
        made_at: rec.at.clone(),
        manifest_lines: rec.lines,
        ..Default::default()
    };
    {
        let scratch = Scratch::new(&stage)?;
        let manifest = manifest(store, key, &scratch, &rec)?;
        let missing = archive::restore(&stage, &manifest, |sha, to| {
            let name = key.name(&format!("blob/{sha}"));
            if !present.contains(&name) {
                return Ok(false);
            }
            fetch(store, key, &scratch, &name, to)?;
            Ok(true)
        })?;
        report.blobs_missing = missing.len() as u64;
        if !missing.is_empty() && !allow_missing {
            bail!(
                "the target lacks {} the manifest names; nothing was restored. If the home it \
                 came from still has them, a backup run from there stores them again; otherwise \
                 restore with --allow-missing to bring back the rest.",
                count(report.blobs_missing, "blob")
            );
        }
        report.blobs = archive::expected_blobs(&archive::parse_manifest(&manifest)).len() as u64
            - report.blobs_missing;
        for (rel, f) in &rec.files {
            if !restorable(rel) {
                bail!(
                    "run {}'s record names {rel:?}, which is not restorable",
                    rec.number
                );
            }
            let mut bytes = Vec::new();
            if f.object.is_empty() {
                // A log, from its parts in order.
                for part in &f.parts {
                    if part.from != bytes.len() as u64 {
                        bail!("run {}'s record has a gap in {rel}", rec.number);
                    }
                    fetch(store, key, &scratch, &part.object, &mut bytes)
                        .with_context(|| format!("restore {rel}"))?;
                }
            } else {
                fetch(store, key, &scratch, &f.object, &mut bytes)
                    .with_context(|| format!("restore {rel}"))?;
            }
            if hex::encode(Sha256::digest(&bytes)) != f.sha256 {
                bail!("the backup's copy of {rel} does not match its record");
            }
            let path = stage.root.join(rel);
            create_dir_private(path.parent().expect("a file has a folder"))?;
            write_atomic(&path, &bytes)?;
            report.files += 1;
        }
    }
    key.install(&stage)?;
    // Every folder's entries are durable before the home appears, so a crash after a restore
    // reports success loses nothing of it.
    for entry in walkdir::WalkDir::new(staging.path()).contents_first(true) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            sync_dir(entry.path())?;
        }
    }
    let staged = staging.keep();
    // Onto an empty folder or none: a rename replaces an empty one.
    if let Err(e) = fs::rename(&staged, &home.root) {
        let _ = fs::remove_dir_all(&staged);
        return Err(e).with_context(|| format!("move the restore into {}", home.root.display()));
    }
    set_private(&home.root);
    sync_dir(parent)?;
    Ok(report)
}

/// What [`verify`] found at a target.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupCheck {
    pub target: String,
    /// The newest run, and when it was made.
    pub run: u64,
    pub made_at: String,
    /// Whether every object was fetched and decrypted, not only looked up by name.
    pub full: bool,
    pub objects: u64,
    pub missing: u64,
    /// Objects that did not decrypt or did not match the record (only a full check finds these).
    pub corrupt: u64,
}

impl BackupCheck {
    pub fn ok(&self) -> bool {
        self.missing == 0 && self.corrupt == 0
    }

    pub fn summary(&self) -> String {
        let how = if self.full {
            "fetched and decrypted"
        } else {
            "found by name"
        };
        if self.ok() {
            format!(
                "{}: run {} (made {}) is whole; {} {how}.",
                self.target,
                self.run,
                self.made_at,
                count(self.objects, "object")
            )
        } else {
            format!(
                "{}: run {} (made {}) is not whole: {} missing, {} corrupt of {}.",
                self.target,
                self.run,
                self.made_at,
                self.missing,
                self.corrupt,
                count(self.objects, "object")
            )
        }
    }
}

/// Check that `store` holds every object its newest run record names: the manifest, each blob the
/// manifest says the archive holds, and each other file. With `full`, each is fetched and
/// decrypted and checked against the record; nothing is written but scratch under `state/`.
pub fn verify(
    home: &Home,
    target: &str,
    store: &dyn Store,
    key: &BackupKey,
    full: bool,
) -> Result<BackupCheck> {
    let present = store.list()?;
    let rec = newest(store, key, &present)?;
    let scratch = Scratch::new(home)?;
    let mut c = BackupCheck {
        target: target.to_string(),
        run: rec.number,
        made_at: rec.at.clone(),
        full,
        objects: 1,
        ..Default::default()
    };
    // The manifest is always opened: it says which blobs the target should hold.
    c.objects += rec.manifest.len() as u64;
    let absent = rec
        .manifest
        .iter()
        .filter(|p| !present.contains(&p.object))
        .count() as u64;
    let manifest = if absent > 0 {
        c.missing += absent;
        Vec::new()
    } else {
        match manifest(store, key, &scratch, &rec) {
            Ok(m) => m,
            Err(e) if unreadable(&e) => return Err(e),
            Err(_) => {
                c.corrupt += 1;
                Vec::new()
            }
        }
    };
    let mut check = |name: &str, want: Option<(&str, u64)>| -> Result<()> {
        c.objects += 1;
        if !present.contains(name) {
            c.missing += 1;
        } else if full {
            let mut h = Counted::default();
            let fetched = fetch(store, key, &scratch, name, &mut h);
            if let Err(e) = fetched.as_ref()
                && unreadable(e)
            {
                return Err(fetched.unwrap_err());
            }
            let good = fetched.is_ok()
                && want.is_none_or(|(sha, size)| {
                    hex::encode(h.hasher.clone().finalize()) == sha
                        && (size == u64::MAX || h.bytes == size)
                });
            if !good {
                c.corrupt += 1;
            }
        }
        Ok(())
    };
    for (sha, size) in archive::expected_blobs(&archive::parse_manifest(&manifest)) {
        check(&key.name(&format!("blob/{sha}")), Some((&sha, size)))?;
    }
    for f in rec.files.values() {
        if f.object.is_empty() {
            for part in &f.parts {
                check(
                    &part.object,
                    Some((&part.sha256, part.to.saturating_sub(part.from))),
                )?;
            }
        } else {
            check(&f.object, Some((&f.sha256, u64::MAX)))?;
        }
    }
    Ok(c)
}

/// The newest run record at a target.
fn newest(store: &dyn Store, key: &BackupKey, present: &BTreeSet<String>) -> Result<RunRecord> {
    match last_run(key, present) {
        Some(n) => read_record(store, key, n),
        None => bail!("this target holds no backup made with this backup key"),
    }
}

/// The manifest lines a run record names, fetched in order and checked against it.
fn manifest(
    store: &dyn Store,
    key: &BackupKey,
    scratch: &Scratch,
    rec: &RunRecord,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut at = 0;
    for part in &rec.manifest {
        if part.from != at {
            bail!("run {}'s record has a gap in the manifest", rec.number);
        }
        fetch(store, key, scratch, &part.object, &mut bytes)
            .with_context(|| format!("manifest lines {}-{}", part.from, part.to))?;
        at = part.to;
    }
    let lines = bytes.iter().filter(|b| **b == b'\n').count() as u64;
    if at != rec.lines
        || lines != rec.lines
        || hex::encode(Sha256::digest(&bytes)) != rec.lines_sha256
    {
        bail!(
            "the backup's manifest does not match run {}'s record",
            rec.number
        );
    }
    Ok(bytes)
}

/// Fetch the object `name` and decrypt it into `to`.
fn fetch(
    store: &dyn Store,
    key: &BackupKey,
    scratch: &Scratch,
    name: &str,
    to: &mut dyn Write,
) -> Result<()> {
    let mut sealed = scratch.file()?;
    store.get(name, &mut sealed).context(Unreadable)?;
    sealed.seek(SeekFrom::Start(0))?;
    key.open(&mut sealed as &mut dyn Read, to)
}

/// A target that could not be read, which a check reports as its error, apart from an object it
/// handed back that does not decrypt, which it counts as corrupt.
#[derive(Debug)]
struct Unreadable;

impl std::fmt::Display for Unreadable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the target could not be read")
    }
}

fn unreadable(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Unreadable>().is_some()
}

/// Whether a record's path is one a restore writes: in the backup set, outside `raw/`, and with
/// no part that leaves the home.
fn restorable(rel: &str) -> bool {
    !rel.starts_with("raw/")
        && matches!(
            Home::keep(rel),
            Some(Keep::Always | Keep::Cards | Keep::Vectors)
        )
        && rel
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
}

/// A writer that only hashes and counts.
#[derive(Default)]
struct Counted {
    hasher: Sha256,
    bytes: u64,
}

impl Write for Counted {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buf);
        self.bytes += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    use super::*;
    use crate::archive::Archive;
    use crate::backup::run::run_name;
    use crate::backup::{run, store};
    use crate::config::BackupTarget;
    use crate::harness::Harness;
    use crate::util::append_jsonl;
    use age::secrecy::ExposeSecret;

    fn session(id: &str, said: &[&str]) -> String {
        said.iter()
            .enumerate()
            .map(|(i, text)| {
                format!(
                    "{{\"type\":\"user\",\"cwd\":\"/w/app\",\"sessionId\":\"{id}\",\"timestamp\":\"2026-11-01T00:00:0{i}Z\",\"message\":{{\"role\":\"user\",\"content\":\"{text}\"}}}}\n"
                )
            })
            .collect()
    }

    /// Every file under `root`, by its path there.
    fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
        walkdir::WalkDir::new(root)
            .into_iter()
            .map(|e| e.unwrap())
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                let rel = e
                    .path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                (rel, fs::read(e.path()).unwrap())
            })
            .collect()
    }

    /// A home with an append, a rewrite and a second session, backed up to a folder beside it.
    fn backed_up() -> (tempfile::TempDir, Home, BackupTarget) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        let mut archive = Archive::open(&home).unwrap();
        let a = "/u/.claude/projects/-w-app/s1.jsonl";
        let b = "/u/.claude/projects/-w-app/s2.jsonl";
        for (origin, bytes) in [
            (a, session("s1", &["fix the widget"])),
            (a, session("s1", &["fix the widget", "and test it"])),
            (b, session("s2", &["rename the gadget"])),
            (b, session("s2", &["rename the gadget, carefully"])),
        ] {
            archive
                .put(
                    Harness::ClaudeCode,
                    origin,
                    &bytes.len().to_string(),
                    bytes.as_bytes(),
                )
                .unwrap();
        }
        let config = crate::Config {
            backups: vec![],
            ..Default::default()
        };
        config.save(&home.config_path()).unwrap();
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
        run(&home, &target).unwrap();
        (dir, home, target)
    }

    #[test]
    fn a_restore_target_reads_each_setting_as_its_table_would_have_it() {
        let settings = ["port=22", "tls=true", "dir=/mnt/a b", "bucket=\"22\""].map(String::from);
        let t = restore_target(None, Some("s3"), &settings).unwrap();
        assert_eq!(t.kind, "s3");
        let v = |k: &str| t.settings[k].clone();
        assert_eq!(v("port"), toml::Value::Integer(22));
        assert_eq!(v("tls"), toml::Value::Boolean(true));
        assert_eq!(v("dir"), toml::Value::String("/mnt/a b".into()));
        assert_eq!(v("bucket"), toml::Value::String("22".into()));
        assert!(restore_target(None, Some("s3"), &["dir".into()]).is_err());
        assert_eq!(
            restore_target(Some("/d".into()), None, &[]).unwrap().kind,
            "folder"
        );
    }

    #[test]
    fn a_restored_home_has_the_same_raw_archive_and_rebuilds_the_same_sessions() {
        let (dir, home, target) = backed_up();
        let recovery = BackupKey::existing(&home).unwrap().unwrap().recovery();
        let key = BackupKey::from_recovery(recovery.expose_secret()).unwrap();
        let store = store::open(&target, false).unwrap();

        let other = BackupKey::create(&Home::new(dir.path().join("elsewhere"))).unwrap();
        let err = restore(&Home::new(dir.path().join("x")), &*store, &other, false).unwrap_err();
        assert!(
            err.to_string()
                .contains("no backup made with this backup key"),
            "{err}"
        );
        assert!(!dir.path().join("x").exists());

        // Into an empty folder, as a new disk would have.
        let restored = Home::new(dir.path().join("restored"));
        fs::create_dir(&restored.root).unwrap();
        let r = restore(&restored, &*store, &key, false).unwrap();
        assert_eq!(
            (r.run, r.manifest_lines, r.blobs_missing),
            (0, 4, 0),
            "{r:?}"
        );
        assert_eq!(
            r.blobs, 3,
            "the append's earlier blob was superseded: {r:?}"
        );

        assert_eq!(
            files(&restored.raw_dir()),
            files(&home.raw_dir()),
            "an identical raw/"
        );
        for rel in ["config.toml", "log/reads.jsonl", "keys/backup.key"] {
            assert_eq!(
                fs::read(restored.root.join(rel)).unwrap(),
                fs::read(home.root.join(rel)).unwrap(),
                "{rel}"
            );
        }
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".distill-restore-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&restored.root).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }

        // The tick's parse step rebuilds the same sessions from it.
        crate::normalize::normalize(&home, false).unwrap();
        crate::normalize::normalize(&restored, false).unwrap();
        let sessions = |h: &Home| {
            let mut s: Vec<_> = crate::normalize::read_session_list(h)
                .unwrap()
                .into_iter()
                .map(|m| serde_json::to_value(m).unwrap())
                .collect();
            s.sort_by_key(|v| v.to_string());
            s
        };
        assert_eq!(sessions(&restored), sessions(&home));
        assert_eq!(sessions(&home).len(), 2);

        let err = restore(&restored, &*store, &key, false).unwrap_err();
        assert!(err.to_string().contains("not empty"), "{err}");
    }

    #[test]
    fn a_spokes_raw_archive_is_backed_up_by_its_appends_and_restored() {
        let (dir, home, target) = backed_up();
        let origin = "/u/.claude/projects/-w-app/s1.jsonl";
        let put = |said: &[&str]| {
            let bytes = session("s1", said);
            Archive::open_of(&home, Some("nas"))
                .unwrap()
                .put(
                    Harness::ClaudeCode,
                    origin,
                    &bytes.len().to_string(),
                    bytes.as_bytes(),
                )
                .unwrap();
        };
        put(&["sort the files"]);
        assert!(run(&home, &target).unwrap().ok);
        put(&["sort the files", "by date"]);
        let second = run(&home, &target).unwrap();
        assert!(second.ok, "{second:?}");

        let key = BackupKey::existing(&home).unwrap().unwrap();
        let store = store::open(&target, false).unwrap();
        let rec = crate::backup::run::read_record(&*store, &key, second.run).unwrap();
        let manifest = &rec.files["machines/nas/raw/manifest.jsonl"];
        let len = fs::metadata(home.manifest_path_of(Some("nas")))
            .unwrap()
            .len();
        assert!(
            manifest.object.is_empty()
                && manifest.len == len
                && manifest.parts.iter().map(|p| p.runs).sum::<u64>() == 2,
            "a spoke's manifest goes as what each run appended: {manifest:?}"
        );

        let restored = Home::new(dir.path().join("restored"));
        restore(&restored, &*store, &key, false).unwrap();
        assert_eq!(
            files(&restored.raw_dir_of(Some("nas"))),
            files(&home.raw_dir_of(Some("nas")))
        );
        crate::normalize::normalize(&home, false).unwrap();
        crate::normalize::normalize(&restored, false).unwrap();
        let keys = |h: &Home| -> Vec<String> {
            let mut k: Vec<String> = crate::normalize::read_session_list(h)
                .unwrap()
                .iter()
                .map(|m| m.key())
                .collect();
            k.sort();
            k
        };
        assert_eq!(keys(&restored), keys(&home));
        assert!(keys(&home).contains(&"nas:claude-code/s1".to_string()));
    }

    #[test]
    fn verify_finds_a_lost_object_by_name_and_a_changed_one_only_in_full() {
        let (_dir, home, target) = backed_up();
        let key = BackupKey::existing(&home).unwrap().unwrap();
        let store = store::open(&target, false).unwrap();
        let check = |full| verify(&home, "drive", &*store, &key, full).unwrap();
        let whole = check(true);
        assert!(whole.ok(), "{whole:?}");
        // The record, one manifest part, three blobs, the config and the read log.
        assert_eq!(whole.objects, 7, "{whole:?}");

        let objects = target.path.as_ref().unwrap().join("distill-backup");
        let at = |name: &str| objects.join(&name[..2]).join(name);
        let rec = read_record(&*store, &key, 0).unwrap();
        let config = at(&rec.files["config.toml"].object);
        let mut bytes = fs::read(&config).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&config, bytes).unwrap();
        assert!(check(false).ok(), "a name check does not open objects");
        assert_eq!(check(true).corrupt, 1);

        let sha = archive::expected_blobs(&archive::parse_manifest(
            &fs::read(home.manifest_path()).unwrap(),
        ))
        .into_keys()
        .next()
        .unwrap();
        fs::remove_file(at(&key.name(&format!("blob/{sha}")))).unwrap();
        let c = check(false);
        assert_eq!((c.missing, c.corrupt), (1, 0), "{c:?}");
        assert!(c.summary().contains("not whole"), "{}", c.summary());
    }

    #[test]
    fn a_target_missing_a_blob_restores_only_when_allowed_and_its_newest_run_is_found() {
        let (dir, home, target) = backed_up();
        let key = BackupKey::existing(&home).unwrap().unwrap();
        let store = store::open(&target, false).unwrap();
        for _ in 0..8 {
            run(&home, &target).unwrap();
        }
        // The target loses everything but run 8 and what it names, and one of its blobs.
        let rec = read_record(&*store, &key, 8).unwrap();
        let blobs: Vec<String> = archive::expected_blobs(&archive::parse_manifest(
            &fs::read(home.manifest_path()).unwrap(),
        ))
        .into_keys()
        .map(|sha| key.name(&format!("blob/{sha}")))
        .collect();
        let mut keep: BTreeSet<String> = rec.manifest.iter().map(|p| p.object.clone()).collect();
        for f in rec.files.values() {
            keep.insert(f.object.clone());
            keep.extend(f.parts.iter().map(|p| p.object.clone()));
        }
        keep.extend(blobs[1..].iter().cloned());
        keep.insert(run_name(&key, 8));
        let objects = target.path.as_ref().unwrap().join("distill-backup");
        for name in store.list().unwrap().difference(&keep) {
            fs::remove_file(objects.join(&name[..2]).join(name)).unwrap();
        }
        let left = store.list().unwrap();
        assert!(
            (0..8).all(|n| !left.contains(&run_name(&key, n))),
            "only run 8's record is left"
        );

        let restored = Home::new(dir.path().join("restored"));
        let err = restore(&restored, &*store, &key, false).unwrap_err();
        assert!(err.to_string().contains("lacks 1 blob"), "{err}");
        assert!(!dir.path().join("restored").exists(), "nothing restored");
        let r = restore(&restored, &*store, &key, true).unwrap();
        assert_eq!((r.run, r.blobs, r.blobs_missing), (8, 2, 1), "{r:?}");
        assert!(dir.path().join("restored/raw/manifest.jsonl").is_file());
    }
}
