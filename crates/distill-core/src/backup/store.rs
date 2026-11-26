//! The store contract (ADR 0009, issue #146): what moves opaque, encrypted objects to and from
//! one backup target. A store never sees a key or knows what DISTILL is, and `put` never
//! overwrites, so a half-finished run is safe to repeat. The `folder` kind is built in (issue
//! #148): a local path, which covers external drives and mounted shares. Any other kind is an
//! adapter, `distill-backup-<kind>` on `PATH` (issue #177, [`super::adapter`]).

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use super::adapter::{Adapter, Open};
use crate::config::BackupTarget;
use crate::util::{private_file, sync_dir};

/// One backup target's objects.
pub trait Store {
    /// The names of every object the target holds.
    fn list(&self) -> Result<BTreeSet<String>>;
    /// Store `from` under `name` unless the target holds that name already; whether it stored it.
    fn put(&self, name: &str, from: &mut dyn Read) -> Result<bool>;
    /// Write the object `name` into `to`.
    fn get(&self, name: &str, to: &mut dyn Write) -> Result<()>;
    /// Make every object the target lists durable. A run calls it before its record, so a
    /// record never outlives an object it names, even one an earlier, failed run left.
    fn sync(&self) -> Result<()>;
}

/// The store for `target`'s kind. `first` says whether this is its first use: only then may a
/// folder target's `distill-backup/` be made.
pub fn open(target: &BackupTarget, first: bool) -> Result<Box<dyn Store>> {
    match target.kind.as_str() {
        "folder" => {
            let path = target
                .path
                .as_ref()
                .with_context(|| format!("backup target {:?} needs a path", target.name))?;
            Ok(Box::new(Folder::open(path, first)?))
        }
        _ => {
            let open = if first { Open::First } else { Open::Again };
            Ok(Box::new(Adapter::open(target, open)?))
        }
    }
}

/// Check that `target` is there and takes a write, by writing a small file at it and removing
/// it. Nothing else is made: a folder target's `distill-backup/` waits for its first run.
pub fn probe(target: &BackupTarget) -> Result<()> {
    match target.kind.as_str() {
        "folder" => {
            let path = target
                .path
                .as_ref()
                .with_context(|| format!("backup target {:?} needs a path", target.name))?;
            Folder::probe(path)
        }
        _ => Adapter::open(target, Open::Probe).map(|_| ()),
    }
}

/// A folder target: `<path>/distill-backup/<aa>/<name>`, each object written once.
pub struct Folder {
    objects: PathBuf,
}

impl Folder {
    /// The target at `path`, which must exist: a drive that isn't mounted must not turn into a
    /// folder on this machine's own disk. Only on a target's first use (`first`) is its
    /// `distill-backup/` made; after that a missing one means the drive isn't there, even when
    /// its mount point is (an empty folder, on Linux).
    pub fn open(path: &Path, first: bool) -> Result<Self> {
        if !path.is_dir() {
            bail!(
                "the backup folder {} does not exist (is the drive connected?)",
                path.display()
            );
        }
        let objects = path.join("distill-backup");
        if !first {
            // Never made here: the drive could go away between a check and a `mkdir`.
            if !objects.is_dir() {
                bail!(
                    "{} is not there, though this home backed up to it before (is the drive mounted?)",
                    objects.display()
                );
            }
        } else if make_dir(&objects)? {
            sync_dir(path)?;
        }
        Ok(Self { objects })
    }

    /// Write a file in `path` and remove it.
    fn probe(path: &Path) -> Result<()> {
        if !path.is_dir() {
            bail!(
                "the backup folder {} does not exist (is the drive connected?)",
                path.display()
            );
        }
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|e| anyhow!("generate a file name: {e}"))?;
        let tmp = path.join(format!(".distill-test-{}", hex::encode(nonce)));
        let wrote = (|| -> Result<()> {
            let mut f = private_file().write(true).create_new(true).open(&tmp)?;
            f.write_all(b"distill")?;
            f.sync_all()?;
            Ok(())
        })();
        let _ = fs::remove_file(&tmp);
        wrote.with_context(|| format!("the backup folder {} can't be written to", path.display()))
    }

    fn path(&self, name: &str) -> Result<PathBuf> {
        object_name(name)?;
        Ok(self.objects.join(&name[..2]).join(name))
    }
}

/// Refuse anything but an object's name, a keyed hash, before it becomes a path or a line.
pub(super) fn object_name(name: &str) -> Result<()> {
    if name.len() != 64 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("not a backup object name: {name:?}");
    }
    Ok(())
}

impl Store for Folder {
    fn list(&self) -> Result<BTreeSet<String>> {
        let mut names = BTreeSet::new();
        for dir in fs::read_dir(&self.objects)? {
            let dir = dir?;
            if !dir.file_type()?.is_dir() {
                continue;
            }
            for f in fs::read_dir(dir.path())? {
                let f = f?;
                // Only where `get` looks: a regular file in its own name's shard.
                if let Some(name) = f.file_name().to_str()
                    && self.path(name).is_ok()
                    && dir.file_name().to_str() == Some(&name[..2])
                    && f.file_type()?.is_file()
                {
                    names.insert(name.to_string());
                }
            }
        }
        Ok(names)
    }

    fn put(&self, name: &str, from: &mut dyn Read) -> Result<bool> {
        let path = self.path(name)?;
        if stored(&path)? {
            return Ok(false);
        }
        let dir = path.parent().expect("an object has a folder");
        let new_shard = make_dir(dir)?;
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|e| anyhow!("generate a file name: {e}"))?;
        let tmp = dir.join(format!(".{name}.tmp-{}", hex::encode(nonce)));
        let placed = (|| -> Result<bool> {
            let mut f = private_file()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .with_context(|| format!("create {}", tmp.display()))?;
            std::io::copy(from, &mut f)?;
            f.sync_all()?;
            // A hard link never replaces: if another run stored the name meanwhile, keep theirs.
            match fs::hard_link(&tmp, &path) {
                Ok(()) => Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    stored(&path)?;
                    Ok(false)
                }
                // A file system without hard links (exFAT gives ENOTSUP, FAT on Linux EPERM).
                // Any other failure is an error: falling back then could race a put whose link
                // succeeds.
                // (ENOTSUP and EOPNOTSUPP are one number on Linux, two on macOS.)
                Err(e)
                    if e.raw_os_error().is_some_and(|c| {
                        [libc::ENOTSUP, libc::EOPNOTSUPP, libc::EPERM].contains(&c)
                    }) =>
                {
                    claim_and_rename(&tmp, &path)
                }
                Err(e) => Err(e).with_context(|| format!("store {}", path.display())),
            }
        })();
        let _ = fs::remove_file(&tmp);
        let stored = placed?;
        sync_dir(dir)?;
        if new_shard {
            sync_dir(&self.objects)?;
        }
        Ok(stored)
    }

    fn get(&self, name: &str, to: &mut dyn Write) -> Result<()> {
        let path = self.path(name)?;
        let mut f = fs::File::open(&path).with_context(|| format!("read {}", path.display()))?;
        std::io::copy(&mut f, to)?;
        Ok(())
    }

    fn sync(&self) -> Result<()> {
        for dir in fs::read_dir(&self.objects)? {
            let dir = dir?;
            if dir.file_type()?.is_dir() {
                sync_dir(&dir.path())?;
            }
        }
        sync_dir(&self.objects)?;
        sync_dir(self.objects.parent().expect("a target has a folder"))
    }
}

/// Whether an object is stored at `path`. Anything there but a file is an error, so a run never
/// counts it as stored.
fn stored(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => Ok(true),
        Ok(_) => bail!("{} is in the way of a backup object", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("look for {}", path.display())),
    }
}

/// Make the folder `dir`, owner-only, but not the folders above it: if the target went away, this
/// fails rather than building it again on this machine's disk. Whether it made it.
fn make_dir(dir: &Path) -> Result<bool> {
    let mut b = fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    match b.create(dir) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e).with_context(|| format!("create {}", dir.display())),
    }
}

/// Store `tmp` at `path` on a drive without hard links (exFAT, FAT32), which has no rename that
/// refuses to replace either: puts in one shard take turns under a lock on its `.put.lock`, which
/// the system releases when a run stops, however it stops.
fn claim_and_rename(tmp: &Path, path: &Path) -> Result<bool> {
    let lock = path.with_file_name(".put.lock");
    let f = private_file()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .with_context(|| format!("create {}", lock.display()))?;
    f.lock()
        .with_context(|| format!("lock {}", lock.display()))?;
    let placed = (|| -> Result<bool> {
        if stored(path)? {
            return Ok(false);
        }
        fs::rename(tmp, path).with_context(|| format!("store {}", path.display()))?;
        Ok(true)
    })();
    drop(f);
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_lists_only_what_get_finds_and_never_rebuilds_a_target_that_went_away() {
        let dir = tempfile::tempdir().unwrap();
        let drive = dir.path().join("drive");
        fs::create_dir(&drive).unwrap();
        assert!(
            Folder::open(&drive, false).is_err(),
            "used before, so not made again"
        );
        let store = Folder::open(&drive, true).unwrap();
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        assert!(store.put(&a, &mut &b"first"[..]).unwrap());
        assert!(!store.put(&a, &mut &b"other"[..]).unwrap(), "write-once");
        let mut got = Vec::new();
        store.get(&a, &mut got).unwrap();
        assert_eq!(got, b"first");

        // A name in another shard, or a folder with an object's name, is not an object.
        let objects = drive.join("distill-backup");
        fs::create_dir(objects.join("cc")).unwrap();
        fs::write(objects.join("cc").join(&b), b"x").unwrap();
        fs::create_dir(objects.join("aa").join("a".repeat(63) + "b")).unwrap();
        assert_eq!(store.list().unwrap(), BTreeSet::from([a.clone()]));

        // A folder where an object belongs is in the way, not a stored object.
        let c = "c".repeat(64);
        fs::create_dir_all(objects.join("cc").join(&c)).unwrap();
        assert!(store.put(&c, &mut &b"x"[..]).is_err());

        // Without hard links, puts take turns under the shard's lock, and still never replace.
        let d = "d".repeat(64);
        fs::create_dir(objects.join("dd")).unwrap();
        let (tmp, path) = (objects.join("dd").join(".t"), objects.join("dd").join(&d));
        fs::write(&tmp, b"first").unwrap();
        assert!(claim_and_rename(&tmp, &path).unwrap());
        fs::write(&tmp, b"other").unwrap();
        assert!(!claim_and_rename(&tmp, &path).unwrap(), "write-once");
        assert_eq!(fs::read(&path).unwrap(), b"first");
        assert_eq!(
            store.list().unwrap(),
            BTreeSet::from([a.clone(), d.clone()])
        );

        fs::rename(&drive, dir.path().join("elsewhere")).unwrap();
        assert!(store.put(&b, &mut &b"x"[..]).is_err());
        assert!(!drive.exists(), "not made again on this disk");
    }
}
