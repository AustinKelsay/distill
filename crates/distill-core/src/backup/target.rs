//! Setting up a backup target (issue #150): adding a folder target to a config, and testing one
//! before or between runs. A test writes one small file at the target and removes it; it never
//! makes the target's `distill-backup/`, which only a first run by the owner does.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::BackupKey;
use super::run::{last_run, read_record};
use super::store;
use crate::config::{BackupTarget, Config};
use crate::home::Home;

/// Add a `folder` target named `name` at `path` to `config`. The path may start with `~/`; it
/// must be absolute once that is expanded, since the timer runs from no particular folder. The
/// folder need not exist yet (a drive may be away); a test or the first run says so.
pub fn add_folder(config: &mut Config, name: &str, path: &str) -> Result<()> {
    let name = name.trim();
    let path = path.trim();
    if name.is_empty() {
        bail!("give the target a name");
    }
    if config.backups.iter().any(|t| t.name == name) {
        bail!("a backup target is already named {name:?}");
    }
    if path.is_empty() {
        bail!("give the folder's path");
    }
    let path = expand(path);
    if !path.is_absolute() {
        bail!("give the folder's full path, from /");
    }
    config.backups.push(BackupTarget {
        name: name.to_string(),
        kind: "folder".into(),
        path: Some(path),
        cards: false,
        vectors: false,
        settings: Default::default(),
    });
    Ok(())
}

/// The first target in `config` named like a folder this home has backed up to that is not the
/// one the home's saved config has under that name, and holds no run of this home's (a folder
/// removed and added back may keep its name). A target's backups go by its name, so such a
/// target would take the other folder's runs as its own.
pub fn reused_name(home: &Home, config: &Config) -> Result<Option<String>> {
    // Under any key: a backup never makes the folder of a target backed up to before.
    let log: Vec<super::run::BackupReport> = crate::util::read_jsonl(&home.backup_log_path())?;
    let saved = match home.load_config() {
        Ok(c) => c.backups,
        Err(_) => Vec::new(),
    };
    let key = BackupKey::existing(home)?;
    let ours = |t: &BackupTarget| {
        let present = store::open(t, false).and_then(|s| s.list());
        matches!((&key, present), (Some(k), Ok(p)) if last_run(k, &p).is_some())
    };
    Ok(config
        .backups
        .iter()
        .find(|t| {
            log.iter().any(|l| l.ok && l.target == t.name)
                && !saved.iter().any(|s| s.name == t.name && s.path == t.path)
                && !ours(t)
        })
        .map(|t| t.name.clone()))
}

/// `~/x` as a path under the owner's home folder; anything else as written.
fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => Path::new(&home).join(rest),
        _ => PathBuf::from(path),
    }
}

/// Test `target` for `home`: that it is there and takes a write, and what it holds. `Ok` says
/// what a run would do there; `Err` says why a run would fail or should not start.
pub fn test(home: &Home, target: &BackupTarget) -> Result<String> {
    store::probe(target)?;
    let used = super::run::backed_up_before(home, &target.name)?;
    let store = match store::open(target, false) {
        Ok(s) => s,
        // Never backed up to: the first run makes what `open` looks for.
        Err(_) if !used => {
            return Ok("Ready. Nothing is copied until the first backup.".into());
        }
        Err(e) => return Err(e),
    };
    let present = store.list()?;
    let key = BackupKey::existing(home)?;
    match key
        .as_ref()
        .and_then(|k| last_run(k, &present).map(|n| (k, n)))
    {
        Some((key, n)) => {
            let rec = read_record(&*store, key, n)?;
            Ok(format!(
                "{} holds this home's run {n}, made {}.",
                target.name, rec.at
            ))
        }
        None if present.is_empty() && !used => {
            Ok("Ready. Nothing is copied until the first backup.".into())
        }
        None if present.is_empty() => bail!(
            "{} holds none of this home's backups, though it backed up there before: is this the right folder?",
            target.name
        ),
        None if key.is_none() => bail!(
            "{} holds backups, and this home has no backup key yet to tell whether they are its own",
            target.name
        ),
        None => bail!(
            "{} holds backups made with another key; a backup there would start over beside them",
            target.name
        ),
    }
}

/// A folder on this machine a backup could go to: a mounted drive, or a folder a cloud service
/// syncs. Setup offers these so the owner need not type a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
    /// A folder a sync app uploads, not a drive.
    pub cloud: bool,
}

/// The drives mounted on this machine and the cloud folders under the user home.
pub fn places() -> Vec<Place> {
    let user = std::env::var_os("HOME").map(PathBuf::from);
    let mut mounts = vec![PathBuf::from("/Volumes")];
    if let Some(name) = std::env::var_os("USER") {
        mounts.push(Path::new("/media").join(&name));
        mounts.push(Path::new("/run/media").join(&name));
    }
    places_at(&mounts, user.as_deref())
}

/// [`places`], with drives looked for in `mounts` and cloud folders under `user`.
pub fn places_at(mounts: &[PathBuf], user: Option<&Path>) -> Vec<Place> {
    let mut found = Vec::new();
    // A drive is a folder in a mount folder that is a mount of its own: not a link (the startup
    // disk's entry in /Volumes is one) and not on the same device as the folder holding it.
    for dir in mounts {
        for (name, path) in entries(dir) {
            let mounted = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir())
                && same_device(&path, dir) == Some(false);
            if mounted {
                found.push(Place {
                    name,
                    path,
                    cloud: false,
                });
            }
        }
    }
    if let Some(user) = user {
        for (name, path) in entries(&user.join("Library/CloudStorage")) {
            let (label, path) = match name.split('-').next().unwrap_or(&name) {
                // The account's root holds only My Drive and shared drives; a backup goes in
                // My Drive.
                "GoogleDrive" if path.join("My Drive").is_dir() => {
                    ("Google Drive", path.join("My Drive"))
                }
                "Dropbox" => ("Dropbox", path),
                "OneDrive" => ("OneDrive", path),
                "Box" => ("Box", path),
                _ => continue,
            };
            found.push(Place {
                name: label.into(),
                path,
                cloud: true,
            });
        }
        let icloud = user.join("Library/Mobile Documents/com~apple~CloudDocs");
        if icloud.is_dir() {
            found.push(Place {
                name: "iCloud Drive".into(),
                path: icloud,
                cloud: true,
            });
        }
    }
    // Two of a kind (two Google accounts) get numbered names.
    let mut names = std::collections::BTreeMap::<String, u32>::new();
    for p in &mut found {
        let n = names.entry(p.name.clone()).or_default();
        *n += 1;
        if *n > 1 {
            p.name = format!("{} {n}", p.name);
        }
    }
    found
}

/// Whether `path` is in a folder a sync app uploads (one under `Library/CloudStorage` or iCloud
/// Drive), so what is there now has not necessarily reached the cloud.
pub fn synced(path: &Path) -> bool {
    let parts: Vec<_> = path.components().map(|c| c.as_os_str()).collect();
    parts
        .windows(3)
        .any(|w| w[0] == "Library" && (w[1] == "CloudStorage" || w[1] == "Mobile Documents"))
}

/// The folders in `dir` whose names don't start with a dot, by name.
fn entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<_> = read
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            (!name.starts_with('.') && e.path().is_dir()).then(|| (name, e.path()))
        })
        .collect();
    v.sort();
    v
}

#[cfg(unix)]
fn same_device(a: &Path, b: &Path) -> Option<bool> {
    use std::os::unix::fs::MetadataExt;
    Some(std::fs::metadata(a).ok()?.dev() == std::fs::metadata(b).ok()?.dev())
}

#[cfg(not(unix))]
fn same_device(_: &Path, _: &Path) -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::Archive;
    use crate::backup::run;
    use crate::harness::Harness;

    #[test]
    fn a_folder_target_needs_a_name_of_its_own_and_a_full_path() {
        let mut c = Config::default();
        add_folder(&mut c, " drive ", "/Volumes/Backup").unwrap();
        assert_eq!(c.backups[0].name, "drive");
        assert_eq!(c.backups[0].kind, "folder");
        assert_eq!(
            c.backups[0].path.as_deref(),
            Some(Path::new("/Volumes/Backup"))
        );
        for (name, path, why) in [
            ("drive", "/elsewhere", "already named"),
            ("", "/x", "a name"),
            ("other", " ", "path"),
            ("other", "relative/folder", "full path"),
        ] {
            let e = add_folder(&mut c, name, path).unwrap_err().to_string();
            assert!(e.contains(why), "{name:?} {path:?}: {e}");
        }
        if let Some(home) = std::env::var_os("HOME") {
            add_folder(&mut c, "mine", "~/backups").unwrap();
            assert_eq!(c.backups[1].path, Some(Path::new(&home).join("backups")));
        }
    }

    #[test]
    fn a_test_says_whether_a_target_is_ready_holds_this_homes_run_or_is_away() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        Archive::open(&home)
            .unwrap()
            .put(Harness::Codex, "/s/a.jsonl", "1", &b"a\n"[..])
            .unwrap();
        let mut config = Config::default();
        let drive = dir.path().join("drive");
        add_folder(&mut config, "drive", drive.to_str().unwrap()).unwrap();
        let t = &config.backups[0];

        let away = test(&home, t).unwrap_err().to_string();
        assert!(away.contains("does not exist"), "{away}");

        std::fs::create_dir(&drive).unwrap();
        let ready = test(&home, t).unwrap();
        assert_eq!(ready, "Ready. Nothing is copied until the first backup.");
        assert_eq!(
            std::fs::read_dir(&drive).unwrap().count(),
            0,
            "a test leaves nothing behind"
        );

        BackupKey::create(&home).unwrap();
        assert!(run(&home, t).unwrap().ok);
        let holds = test(&home, t).unwrap();
        assert!(holds.contains("holds this home's run 0"), "{holds}");

        // Another home's key: its backups are not this home's.
        let other = Home::new(dir.path().join("other"));
        BackupKey::create(&other).unwrap();
        let e = test(&other, t).unwrap_err().to_string();
        assert!(e.contains("another key"), "{e}");

        // The drive's folder without its backups, after a run: not the folder it backed up to.
        std::fs::rename(drive.join("distill-backup"), dir.path().join("moved")).unwrap();
        let e = test(&home, t).unwrap_err().to_string();
        assert!(e.contains("is the drive mounted"), "{e}");
    }

    #[test]
    fn a_new_folder_may_not_take_the_name_of_one_backed_up_to() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        BackupKey::create(&home).unwrap();
        let drive = dir.path().join("drive");
        std::fs::create_dir(&drive).unwrap();
        let mut config = Config::default();
        add_folder(&mut config, "drive", drive.to_str().unwrap()).unwrap();
        assert_eq!(reused_name(&home, &config).unwrap(), None, "no run yet");
        config.save(&home.config_path()).unwrap();
        assert!(run(&home, &config.backups[0]).unwrap().ok);
        assert_eq!(
            reused_name(&home, &config).unwrap(),
            None,
            "the same folder"
        );

        let mut other = Config::default();
        add_folder(&mut other, "drive", "/elsewhere").unwrap();
        assert_eq!(
            reused_name(&home, &other).unwrap().as_deref(),
            Some("drive")
        );
        add_folder(&mut config, "new", "/elsewhere").unwrap();
        assert_eq!(reused_name(&home, &config).unwrap(), None, "a new name");

        // Removed and saved, then added back: the folder holds this home's run, so it may.
        Config::default().save(&home.config_path()).unwrap();
        let mut back = Config::default();
        add_folder(&mut back, "drive", drive.to_str().unwrap()).unwrap();
        assert_eq!(reused_name(&home, &back).unwrap(), None);
    }

    #[test]
    fn places_are_mounted_drives_and_cloud_folders() {
        let dir = tempfile::tempdir().unwrap();
        // A folder in a mount folder on the same device is not a drive (the startup disk).
        let volumes = dir.path().join("Volumes");
        std::fs::create_dir_all(volumes.join("Macintosh HD")).unwrap();
        let user = dir.path().join("me");
        let cloud = user.join("Library/CloudStorage");
        for d in [
            "GoogleDrive-a@example.com/My Drive",
            "GoogleDrive-b@example.com/My Drive",
            "GoogleDrive-c@example.com",
            "Dropbox",
            "OneDrive-Personal",
            "Unknown-Thing",
            ".hidden",
        ] {
            std::fs::create_dir_all(cloud.join(d)).unwrap();
        }
        std::fs::create_dir_all(user.join("Library/Mobile Documents/com~apple~CloudDocs")).unwrap();

        let found = places_at(&[volumes, dir.path().join("absent")], Some(&user));
        let got: Vec<_> = found
            .iter()
            .map(|p| {
                (
                    p.name.as_str(),
                    p.path.strip_prefix(&user).unwrap(),
                    p.cloud,
                )
            })
            .collect();
        let l = |s: &str| Path::new("Library").join(s);
        assert_eq!(
            got,
            [
                ("Dropbox", l("CloudStorage/Dropbox").as_path(), true),
                (
                    "Google Drive",
                    l("CloudStorage/GoogleDrive-a@example.com/My Drive").as_path(),
                    true
                ),
                (
                    "Google Drive 2",
                    l("CloudStorage/GoogleDrive-b@example.com/My Drive").as_path(),
                    true
                ),
                (
                    "OneDrive",
                    l("CloudStorage/OneDrive-Personal").as_path(),
                    true
                ),
                (
                    "iCloud Drive",
                    l("Mobile Documents/com~apple~CloudDocs").as_path(),
                    true
                ),
            ]
        );
        assert!(places_at(&[], None).is_empty());
        for p in &found {
            assert!(synced(&p.path), "{p:?}");
        }
        let shared = user.join("Library/CloudStorage/GoogleDrive-a@example.com/Shared drives/x");
        assert!(synced(&shared));
        assert!(!synced(Path::new("/Volumes/Backup")) && !synced(&user.join("Library")));
    }
}
