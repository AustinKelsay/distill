//! Backups on the timer, and where each target stands (issue #150). The tick keeps current only
//! the targets the owner has started backing up to with `distill backup run` (or the window): only
//! the owner can say that a folder target is its drive and not an empty mount point. So the tick
//! never makes a target's folder, but it finishes a first run that stopped early.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::run::{BackupReport, run_scheduled};
use crate::config::{BackupTarget, Config};
use crate::home::Home;
use crate::tick::Deadline;

/// The `[[backup]]` targets in `config`, or only the one named; refused when two share a name,
/// since the backup log tells targets apart by name.
pub fn targets(config: &Config, only: Option<&str>) -> Result<Vec<BackupTarget>> {
    let all = &config.backups;
    if let Some((_, t)) = all
        .iter()
        .enumerate()
        .find(|(i, t)| all[..*i].iter().any(|u| u.name == t.name))
    {
        bail!(
            "config.toml names two [[backup]] targets {:?}; give each its own name",
            t.name
        );
    }
    let targets: Vec<_> = all
        .iter()
        .filter(|t| only.is_none_or(|n| t.name == n))
        .cloned()
        .collect();
    if targets.is_empty() {
        match only {
            Some(n) => bail!("no [[backup]] target named {n:?} in config.toml"),
            None => bail!("no [[backup]] targets in config.toml"),
        }
    }
    Ok(targets)
}

/// What the tick's backup step did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TickBackups {
    /// One report per target the step backed up to.
    pub runs: Vec<BackupReport>,
    /// Targets that wait for a first run by hand.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waiting: Vec<String>,
    /// Targets passed over because another backup run or a sweep held a lock the run needs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub busy: Vec<String>,
    /// A run reached the tick's time limit; the next tick goes on from it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopped_at_deadline: bool,
    /// Why the step did nothing, when it did nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

impl TickBackups {
    pub fn ok(&self) -> bool {
        self.runs.iter().all(|r| r.ok || r.stopped_at_deadline)
    }

    pub fn summary(&self) -> String {
        if let Some(s) = &self.skipped {
            return format!("backup skipped: {s}");
        }
        let mut parts: Vec<String> = self.runs.iter().map(BackupReport::summary).collect();
        if !self.waiting.is_empty() {
            parts.push(format!(
                "{} {} for a first run by hand (`distill backup run`).",
                self.waiting.join(", "),
                if self.waiting.len() == 1 {
                    "waits"
                } else {
                    "wait"
                }
            ));
        }
        if !self.busy.is_empty() {
            parts.push(format!(
                "{}: passed over, another backup run or a sweep was under way.",
                self.busy.join(", ")
            ));
        }
        parts.join(" ")
    }
}

/// Back up to every target that has had a whole run, or whose folder holds `distill-backup/`
/// already (a first run made it and stopped early, or another home backs up there), as the tick
/// does, sending no more blobs once `deadline` passes.
pub fn tick(home: &Home, config: &Config, deadline: Deadline) -> Result<TickBackups> {
    if config.backups.is_empty() {
        return Ok(TickBackups {
            skipped: Some("no [[backup]] target is set up".into()),
            ..Default::default()
        });
    }
    let log = super::run::log(home)?;
    let mut done = TickBackups::default();
    for t in targets(config, None)? {
        let started = || super::store::open(&t, false).is_ok();
        if log.iter().any(|l| l.ok && l.target == t.name) || started() {
            match run_scheduled(home, &t, deadline)? {
                Some(r) => {
                    done.stopped_at_deadline |= r.stopped_at_deadline;
                    done.runs.push(r);
                }
                None => done.busy.push(t.name),
            }
        } else {
            done.waiting.push(t.name);
        }
    }
    if done.runs.is_empty() && done.busy.is_empty() {
        done.skipped = Some(format!(
            "no target has had a first run yet; run `distill backup run` once ({})",
            done.waiting.join(", ")
        ));
    }
    Ok(done)
}

/// Where one target stands, from the backup log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetStatus {
    pub name: String,
    pub kind: String,
    /// When the newest whole run finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ok: Option<String>,
    /// The newest run's error, if it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// When that failed run was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failed: Option<String>,
    /// Captures in the archive the newest whole run did not cover, when that run says how many
    /// it covered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captures_behind: Option<u64>,
}

/// Each `[[backup]]` target of `config` as the backup log has it, with the archive holding
/// `captures` captures. Refused, as by the tick, when two targets share a name.
pub fn status(home: &Home, config: &Config, captures: u64) -> Result<Vec<TargetStatus>> {
    if config.backups.is_empty() {
        return Ok(Vec::new());
    }
    let targets = targets(config, None)?;
    let log = super::run::log(home)?;
    Ok(targets
        .iter()
        .map(|t| {
            let mine = || log.iter().rev().filter(|l| l.target == t.name);
            let ok = mine().find(|l| l.ok);
            let failed = mine().next().filter(|l| !l.ok);
            TargetStatus {
                name: t.name.clone(),
                kind: t.kind.clone(),
                last_ok: ok.map(|l| l.at.clone()),
                last_error: failed.and_then(|l| l.error.clone()),
                last_failed: failed.map(|l| l.at.clone()),
                captures_behind: ok
                    .and_then(|l| l.manifest_lines)
                    .map(|n| captures.saturating_sub(n)),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::archive::Archive;
    use crate::backup::{BackupKey, run};
    use crate::harness::Harness;

    fn capture(home: &Home, path: &str) {
        Archive::open(home)
            .unwrap()
            .put(Harness::Codex, path, "1", format!("{path}\n").as_bytes())
            .unwrap();
    }

    fn folder(name: &str, path: std::path::PathBuf) -> BackupTarget {
        BackupTarget {
            name: name.into(),
            kind: "folder".into(),
            path: Some(path),
            cards: false,
            vectors: false,
        }
    }

    /// A home with one capture and a key, and `config` naming `targets`.
    fn setup(targets: &[(&str, &str)]) -> (tempfile::TempDir, Home, Config) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        capture(&home, "/s/a.jsonl");
        let config = Config {
            backups: targets
                .iter()
                .map(|(n, p)| folder(n, dir.path().join(p)))
                .collect(),
            ..Default::default()
        };
        config.save(&home.config_path()).unwrap();
        BackupKey::create(&home).unwrap();
        (dir, home, config)
    }

    #[test]
    fn the_tick_keeps_current_only_targets_backed_up_by_hand_and_status_says_where_each_stands() {
        let (dir, home, config) = setup(&[("drive", "drive"), ("share", "share")]);
        std::fs::create_dir(dir.path().join("drive")).unwrap();

        // No target has had a run by hand: the tick makes nothing at either.
        let t = tick(&home, &config, Deadline::NONE).unwrap();
        assert!(t.ok() && t.runs.is_empty(), "{t:?}");
        assert_eq!(t.waiting, ["drive", "share"]);
        assert!(t.skipped.is_some(), "{t:?}");
        assert!(!dir.path().join("drive/distill-backup").exists());
        assert!(!dir.path().join("share").exists());
        let s = status(&home, &config, 1).unwrap();
        assert_eq!((s[0].last_ok.as_ref(), s[0].captures_behind), (None, None));

        // After a run by hand, the tick sends only what is new.
        assert!(run(&home, &config.backups[0]).unwrap().ok);
        capture(&home, "/s/b.jsonl");
        let t = tick(&home, &config, Deadline::NONE).unwrap();
        assert!(t.ok(), "{t:?}");
        assert_eq!(t.runs.len(), 1);
        assert_eq!(
            (t.runs[0].target.as_str(), t.runs[0].blobs_sent),
            ("drive", 1)
        );
        assert_eq!(t.waiting, ["share"]);
        assert!(t.summary().contains("share waits"), "{}", t.summary());
        let s = status(&home, &config, 3).unwrap();
        assert!(s[0].last_ok.is_some() && s[0].last_error.is_none(), "{s:?}");
        assert_eq!(
            s[0].captures_behind,
            Some(1),
            "two of three captures are backed up"
        );
        assert_eq!(s[1].last_ok, None);

        // A target that went away fails the step, and status keeps its last whole run.
        std::fs::rename(dir.path().join("drive"), dir.path().join("away")).unwrap();
        let t = tick(&home, &config, Deadline::NONE).unwrap();
        assert!(!t.ok(), "{t:?}");
        let s = status(&home, &config, 2).unwrap();
        assert!(
            s[0].last_ok.is_some() && s[0].last_failed.is_some(),
            "{s:?}"
        );
        assert!(s[0].last_error.is_some(), "{s:?}");
        assert_eq!(s[0].captures_behind, Some(0));
        assert!(
            !dir.path().join("drive").exists(),
            "the tick made no folder"
        );

        // Another folder under the same name, even one with an empty `distill-backup/`, is not
        // started over by the tick.
        let mut moved = config.clone();
        moved.backups[0].path = Some(dir.path().join("new"));
        std::fs::create_dir_all(dir.path().join("new/distill-backup")).unwrap();
        let t = tick(&home, &moved, Deadline::NONE).unwrap();
        assert!(!t.ok(), "{t:?}");
        assert!(
            t.runs[0]
                .error
                .as_deref()
                .unwrap()
                .contains("holds no run of this home's"),
            "{t:?}"
        );
        assert_eq!(
            std::fs::read_dir(dir.path().join("new/distill-backup"))
                .unwrap()
                .count(),
            0,
            "nothing was sent"
        );
    }

    #[test]
    fn a_new_key_starts_every_target_over_and_the_old_key_still_opens_its_runs() {
        use crate::backup::run::{last_run, read_record};
        use crate::backup::store;
        use age::secrecy::ExposeSecret;

        let (dir, home, config) = setup(&[("drive", "drive")]);
        std::fs::create_dir(dir.path().join("drive")).unwrap();
        assert!(run(&home, &config.backups[0]).unwrap().ok);
        let old = BackupKey::existing(&home).unwrap().unwrap();

        // Refused while a backup runs.
        let held = home.lock("backup").unwrap();
        let err = BackupKey::replace(&home).err().unwrap().to_string();
        assert!(err.contains("a backup is running"), "{err}");
        drop(held);

        let new = BackupKey::replace(&home).unwrap();
        let again = BackupKey::existing(&home).unwrap().unwrap();
        assert_eq!(
            again.recovery().expose_secret(),
            new.recovery().expose_secret()
        );
        assert_ne!(
            new.recovery().expose_secret(),
            old.recovery().expose_secret()
        );
        assert_eq!(status(&home, &config, 1).unwrap()[0].last_ok, None);

        // The tick sends everything again under the new key; the old run stays as it was.
        let t = tick(&home, &config, Deadline::NONE).unwrap();
        assert!(t.ok(), "{t:?}");
        assert_eq!(
            (t.runs.len(), t.runs[0].run, t.runs[0].blobs_sent),
            (1, 0, 1)
        );
        let present = store::open(&config.backups[0], false)
            .unwrap()
            .list()
            .unwrap();
        let target = store::open(&config.backups[0], false).unwrap();
        for key in [&old, &new] {
            assert_eq!(last_run(key, &present), Some(0));
            assert!(read_record(&*target, key, 0).is_ok());
        }
        assert!(status(&home, &config, 1).unwrap()[0].last_ok.is_some());
    }

    #[test]
    fn the_tick_finishes_a_first_run_that_stopped_early() {
        let (dir, home, config) = setup(&[("drive", "drive")]);
        // A first run made `distill-backup/` and stopped before its record (the window closed).
        std::fs::create_dir_all(dir.path().join("drive/distill-backup")).unwrap();
        let t = tick(&home, &config, Deadline::NONE).unwrap();
        assert!(t.ok() && t.waiting.is_empty(), "{t:?}");
        assert_eq!((t.runs.len(), t.runs[0].run), (1, 0));
        assert!(status(&home, &config, 1).unwrap()[0].last_ok.is_some());
    }

    #[test]
    fn a_tick_backup_stops_at_the_deadline_and_passes_over_a_run_under_way() {
        let (dir, home, config) = setup(&[("drive", "drive")]);
        std::fs::create_dir(dir.path().join("drive")).unwrap();
        assert!(run(&home, &config.backups[0]).unwrap().ok);
        capture(&home, "/s/b.jsonl");

        let lines = || {
            std::fs::read_to_string(home.backup_log_path())
                .unwrap()
                .lines()
                .count()
        };
        let logged = lines();

        // Out of time: nothing sent, no record, not a failure, and not in the backup log.
        let t = tick(&home, &config, Deadline::after(Duration::ZERO)).unwrap();
        assert!(t.ok() && t.stopped_at_deadline, "{t:?}");
        assert_eq!((t.runs[0].ok, t.runs[0].blobs_sent), (false, 0), "{t:?}");
        assert!(t.runs[0].summary().contains("time limit"), "{t:?}");
        assert_eq!(lines(), logged);
        let s = status(&home, &config, 2).unwrap();
        assert_eq!(
            (s[0].last_error.as_ref(), s[0].captures_behind),
            (None, Some(1))
        );

        // A sweep or another run holds a lock the run needs: the tick doesn't wait for it.
        for name in ["backup", "archive"] {
            let lock = home.lock(name).unwrap();
            let t = tick(&home, &config, Deadline::NONE).unwrap();
            assert!(t.ok() && t.runs.is_empty(), "{name}: {t:?}");
            assert_eq!(t.busy, ["drive"]);
            drop(lock);
        }
        assert_eq!(lines(), logged);

        // The next tick goes on.
        let t = tick(&home, &config, Deadline::NONE).unwrap();
        assert!(t.ok() && t.runs[0].ok, "{t:?}");
        assert_eq!(t.runs[0].blobs_sent, 1);
        let s = status(&home, &config, 2).unwrap();
        assert_eq!(s[0].captures_behind, Some(0));
    }

    #[test]
    fn two_targets_with_one_name_are_refused() {
        let config = Config {
            backups: vec![folder("a", "/x".into()), folder("a", "/y".into())],
            ..Default::default()
        };
        assert!(targets(&config, None).is_err());
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        assert!(tick(&home, &config, Deadline::NONE).is_err());
        assert!(status(&home, &config, 0).is_err());
    }
}
