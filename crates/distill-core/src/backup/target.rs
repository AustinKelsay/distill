//! Setting up a backup target (issue #150): adding a folder target to a config, and testing one
//! before or between runs. A test writes one small file at the target and removes it; it never
//! makes the target's `distill-backup/`, which only a first run by the owner does.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::BackupKey;
use super::run::{BackupReport, last_run, read_record};
use super::store;
use crate::config::{BackupTarget, Config};
use crate::home::Home;
use crate::util::read_jsonl;

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
    });
    Ok(())
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
    let used = read_jsonl::<BackupReport>(&home.backup_log_path())?
        .iter()
        .any(|l| l.ok && l.target == target.name);
    let store = match store::open(target, false) {
        Ok(s) => s,
        // Never backed up to: the first run makes what `open` looks for.
        Err(_) if !used => {
            return Ok(format!(
                "{} can be written to. The first backup starts it.",
                target.name
            ));
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
        None if present.is_empty() && !used => Ok(format!(
            "{} can be written to. The first backup starts it.",
            target.name
        )),
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
        assert!(ready.contains("first backup starts it"), "{ready}");
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
}
