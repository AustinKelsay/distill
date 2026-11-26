use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::Config;
use crate::harness::Harness;
use crate::util::{create_dir_private, private_file, safe_component, set_private};

/// A DISTILL home directory. `DISTILL_HOME` overrides the default `~/distill`.
///
/// The layout is fixed by the spec (issue #72, "On disk"):
///
/// ```text
/// <home>/
///   config.toml                  config_path
///   raw/                         raw_dir, manifest_path: captures and their manifest
///   machines/<machine>/raw/      raw_dir_of, manifest_path_of: a spoke's (ADR 0010)
///   sessions/index.jsonl         session_list_path: every session's facts
///   sessions/<harness>/<id>/     session_dir
///   sessions/machines/<machine>/<harness>/<id>/   session_dir, for a spoke's session
///     meta.json                  meta_path: session facts
///     events.jsonl               events_path
///     findings.json              findings_path
///     card.json                  card_path
///   index/                       index_dir: word index and vectors
///     words/                     words_dir: the word index
///       manifest.json            words_manifest_path
///       seg-<n>.bin              words_segment_path
///     vectors.bin                vectors_path: vectors for search by meaning
///   state/                       state_dir: locks and progress markers
///   state/machines/<machine>/    state_dir_of: a spoke's archive's progress markers
///   log/reads.jsonl              read_log_path
///   log/backups.jsonl            backup_log_path
///   log/exports.jsonl            export_log_path: on a spoke, what each pull took
///   runs/                        runs_dir: the run log
///   reveals/                     reveals_dir: one file per reveal request (reveal_path)
///   keys/fingerprint.key         fingerprint_key_path
///   keys/backup.key              backup_key_path
///   tags.json                    owner_tags_path: the owner's tag edits
/// ```
///
/// `sessions/`, `index/` and `state/` can be deleted and rebuilt from the raw archives (`raw/`
/// and each spoke's). What a backup holds
/// is [`Home::keep`]'s answer for each path (ADR 0009). Folders a previous
/// build left in a home ([`Home::PREVIOUS_BUILD`]) are ignored, never deleted.
/// Whether a backup holds a file of the home (ADR 0009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// In every backup: nothing can rebuild it.
    Always,
    /// Only when the target carries cards: rebuildable, but at the cost of model time.
    Cards,
    /// Only when the target carries vectors: rebuildable, but at the cost of embedding again.
    Vectors,
    /// Never: rebuilt from `raw/`, not needed (locks, the run log), or the backup key itself.
    Never,
}

#[derive(Debug, Clone)]
pub struct Home {
    pub root: PathBuf,
}

impl Home {
    /// The folders a previous build left in a home: ignored, never deleted, and checked by the
    /// exposure check (`previous_build_dirs`).
    pub const PREVIOUS_BUILD: [&'static str; 3] = ["exports", "recipes", "steward"];

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn resolve(explicit: Option<&Path>) -> Result<Self> {
        if let Some(p) = explicit {
            return Ok(Self::new(p));
        }
        if let Ok(p) = std::env::var("DISTILL_HOME")
            && !p.is_empty()
        {
            return Ok(Self::new(p));
        }
        Ok(Self::new(user_home()?.join("distill")))
    }

    /// Create the folders capture and the parsers write, with owner-only permissions.
    pub fn ensure(&self) -> Result<()> {
        for dir in [
            self.root.clone(),
            self.raw_dir(),
            self.sessions_dir(),
            self.runs_dir(),
            self.state_dir(),
        ] {
            create_dir_private(&dir).with_context(|| format!("create {}", dir.display()))?;
            set_private(&dir);
        }
        Ok(())
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.toml")
    }
    pub fn raw_dir(&self) -> PathBuf {
        self.root.join("raw")
    }
    pub fn manifest_path(&self) -> PathBuf {
        self.raw_dir().join("manifest.jsonl")
    }
    /// Each spoke's raw archive, in a folder named for its machine (ADR 0010).
    pub fn machines_dir(&self) -> PathBuf {
        self.root.join("machines")
    }
    /// A raw archive: this home's own (`raw/`) with no machine, else that spoke's.
    pub fn raw_dir_of(&self, machine: Option<&str>) -> PathBuf {
        match machine {
            None => self.raw_dir(),
            Some(m) => self.machines_dir().join(m).join("raw"),
        }
    }
    pub fn manifest_path_of(&self, machine: Option<&str>) -> PathBuf {
        self.raw_dir_of(machine).join("manifest.jsonl")
    }
    /// Every raw archive this home holds: its own (`None`) first, then each spoke's.
    pub fn archives(&self) -> Result<Vec<Option<String>>> {
        Ok(std::iter::once(None)
            .chain(self.machines()?.into_iter().map(Some))
            .collect())
    }
    /// The spokes whose raw archive this home holds, by name, sorted.
    pub fn machines(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let rd = match fs::read_dir(self.machines_dir()) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(names),
            Err(e) => return Err(e).context("read machines/"),
        };
        for entry in rd {
            let entry = entry?;
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if machine_name(name).is_err() {
                continue;
            }
            // A spoke folder that can't be read is an error, not a spoke left out of verify.
            match fs::metadata(entry.path().join("raw")) {
                Ok(m) if m.is_dir() => names.push(name.to_string()),
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("read machines/{name}/raw")),
            }
        }
        names.sort();
        Ok(names)
    }
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }
    /// The session list: one line of session facts per session, rebuilt by the parse step.
    pub fn session_list_path(&self) -> PathBuf {
        self.sessions_dir().join("index.jsonl")
    }
    /// The folders of one machine's sessions: this home's own with no machine, else a spoke's.
    pub fn sessions_dir_of(&self, machine: Option<&str>) -> PathBuf {
        match machine {
            None => self.sessions_dir(),
            Some(m) => self.sessions_dir().join("machines").join(m),
        }
    }
    /// The folder holding one session's files.
    pub fn session_dir(
        &self,
        machine: Option<&str>,
        harness: Harness,
        session_id: &str,
    ) -> PathBuf {
        self.sessions_dir_of(machine)
            .join(harness.id())
            .join(safe_component(session_id))
    }
    /// A session's facts.
    pub fn meta_path(&self, machine: Option<&str>, harness: Harness, session_id: &str) -> PathBuf {
        self.session_dir(machine, harness, session_id)
            .join("meta.json")
    }
    /// Read only by the read path (`crate::read`), so it is not public.
    pub(crate) fn events_path(
        &self,
        machine: Option<&str>,
        harness: Harness,
        session_id: &str,
    ) -> PathBuf {
        self.session_dir(machine, harness, session_id)
            .join("events.jsonl")
    }
    pub fn findings_path(
        &self,
        machine: Option<&str>,
        harness: Harness,
        session_id: &str,
    ) -> PathBuf {
        self.session_dir(machine, harness, session_id)
            .join("findings.json")
    }
    pub fn card_path(&self, machine: Option<&str>, harness: Harness, session_id: &str) -> PathBuf {
        self.session_dir(machine, harness, session_id)
            .join("card.json")
    }
    /// The search index: word index and vectors.
    pub fn index_dir(&self) -> PathBuf {
        self.root.join("index")
    }
    /// The word index: a manifest and its segments.
    pub fn words_dir(&self) -> PathBuf {
        self.index_dir().join("words")
    }
    pub fn words_manifest_path(&self) -> PathBuf {
        self.words_dir().join("manifest.json")
    }
    /// One segment of the word index, by its number.
    pub fn words_segment_path(&self, id: u64) -> PathBuf {
        self.words_dir().join(format!("seg-{id}.bin"))
    }
    /// Each of [`Home::PREVIOUS_BUILD`] with its path in this home, whether it exists or not.
    pub fn previous_build_dirs(&self) -> impl Iterator<Item = (&'static str, PathBuf)> + '_ {
        Self::PREVIOUS_BUILD
            .into_iter()
            .map(|name| (name, self.root.join(name)))
    }
    /// The vectors for search by meaning: one flat file, searched directly.
    pub fn vectors_path(&self) -> PathBuf {
        self.index_dir().join("vectors.bin")
    }
    /// Progress markers for one raw archive: this home's own with no machine, else a spoke's.
    pub fn state_dir_of(&self, machine: Option<&str>) -> PathBuf {
        match machine {
            None => self.state_dir(),
            Some(m) => self.state_dir().join("machines").join(m),
        }
    }
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }
    pub fn log_dir(&self) -> PathBuf {
        self.root.join("log")
    }
    pub fn read_log_path(&self) -> PathBuf {
        self.log_dir().join("reads.jsonl")
    }
    /// The run log.
    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }
    pub fn reveals_dir(&self) -> PathBuf {
        self.root.join("reveals")
    }
    /// One reveal request's file, by its ID.
    pub fn reveal_path(&self, id: &str) -> PathBuf {
        self.reveals_dir()
            .join(format!("{}.json", safe_component(id)))
    }
    pub fn keys_dir(&self) -> PathBuf {
        self.root.join("keys")
    }
    pub fn fingerprint_key_path(&self) -> PathBuf {
        self.keys_dir().join("fingerprint.key")
    }
    /// The key that encrypts and names this home's backups (ADR 0009). Never in a backup: the
    /// owner keeps a copy of it elsewhere.
    pub fn backup_key_path(&self) -> PathBuf {
        self.keys_dir().join("backup.key")
    }
    /// The backup log: one line per backup run per target.
    pub fn backup_log_path(&self) -> PathBuf {
        self.log_dir().join("backups.jsonl")
    }
    /// On a spoke, one line per export: what each pull of the hub took, and when (ADR 0010).
    pub fn export_log_path(&self) -> PathBuf {
        self.log_dir().join("exports.jsonl")
    }
    /// The tags the owner added to or hid on sessions (issue #158): nothing rebuilds them.
    pub fn owner_tags_path(&self) -> PathBuf {
        self.root.join("tags.json")
    }

    /// Whether a backup holds the file at `rel`, a path relative to the home's root with `/`
    /// between its parts (ADR 0009, issue #145): always, only when the target carries cards or
    /// vectors, or never. None for a path the layout doesn't know, which a test catches, so a new
    /// kind of file can't be left out of backups unnoticed.
    pub fn keep(rel: &str) -> Option<Keep> {
        let parts: Vec<&str> = rel.split('/').collect();
        Some(match parts.as_slice() {
            ["config.toml" | "tags.json"] => Keep::Always,
            ["raw", _, ..] | ["machines", _, "raw", _, ..] => Keep::Always,
            ["keys", "fingerprint.key"] => Keep::Always,
            ["keys", "backup.key"] => Keep::Never,
            ["log", "reads.jsonl" | "backups.jsonl" | "exports.jsonl"] => Keep::Always,
            ["reveals", _] => Keep::Always,
            ["sessions", "machines", _, _, _, "card.json"] => Keep::Cards,
            ["sessions", _, _, "card.json"] => Keep::Cards,
            ["sessions", _, ..] => Keep::Never,
            ["index", "vectors.bin"] => Keep::Vectors,
            ["index", _, ..] | ["state", _, ..] | ["runs", _, ..] => Keep::Never,
            [dir, ..] if Self::PREVIOUS_BUILD.contains(dir) => Keep::Never,
            _ => return None,
        })
    }

    /// The machine whose raw archive's manifest is at `rel` (`machines/<machine>/raw/
    /// manifest.jsonl`), a path as [`Home::keep`] takes it.
    pub fn spoke_manifest(rel: &str) -> Option<&str> {
        match rel.split('/').collect::<Vec<_>>().as_slice() {
            ["machines", m, "raw", "manifest.jsonl"] => Some(m),
            _ => None,
        }
    }

    /// Whether `rel` is in a spoke's raw archive (`machines/<machine>/raw/`).
    pub fn in_spoke_raw(rel: &str) -> bool {
        matches!(
            rel.split('/').collect::<Vec<_>>().as_slice(),
            ["machines", _, "raw", _, ..]
        )
    }

    /// The sessions whose last card attempt failed, so they are retried after the rest.
    pub fn card_failures_path(&self) -> PathBuf {
        self.state_dir().join("cards.json")
    }

    /// The sessions whose last model detection pass failed, so they are retried after the rest.
    pub fn detail_failures_path(&self) -> PathBuf {
        self.state_dir().join("details.json")
    }

    /// Hold the archive writer lock until the returned file is dropped. Sweeps from the timer
    /// and from a person can overlap; only one may write `raw/` at a time.
    pub fn lock_archive(&self) -> Result<fs::File> {
        self.lock("archive")
    }

    /// Hold a named lock under `state/` until the returned file is dropped.
    pub fn lock(&self, name: &str) -> Result<fs::File> {
        lock_at(&self.state_dir().join(format!("{name}.lock")), name)
    }

    /// As [`Home::lock`], but `None` at once when someone else holds it.
    pub fn try_lock(&self, name: &str) -> Result<Option<fs::File>> {
        let path = self.state_dir().join(format!("{name}.lock"));
        create_dir_private(&self.state_dir())?;
        let f = private_file()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        match f.try_lock() {
            Ok(()) => Ok(Some(f)),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(e)) => {
                Err(e).with_context(|| format!("take the {name} lock"))
            }
        }
    }

    /// The lock a session's findings file is written under.
    pub fn findings_lock_path(&self, harness: Harness, session_id: &str) -> PathBuf {
        self.state_dir()
            .join("findings")
            .join(harness.id())
            .join(format!("{}.lock", safe_component(session_id)))
    }

    /// Hold a session's findings lock until the returned file is dropped. The tick and the read
    /// path can both write the findings file; each holds this only for the write.
    pub fn lock_findings(&self, harness: Harness, session_id: &str) -> Result<fs::File> {
        lock_at(&self.findings_lock_path(harness, session_id), "findings")
    }

    /// Whether the home has a config: anything at its path, a link whose target is gone
    /// included.
    pub fn has_config(&self) -> bool {
        !matches!(
            fs::symlink_metadata(self.config_path()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound
        )
    }

    /// This home's `[spoke]`, if it is one (ADR 0010); none with no config. A config that is
    /// there but can't be read is an error, since it may name a spoke.
    pub fn spoke(&self) -> Result<Option<crate::config::Spoke>> {
        if !self.has_config() {
            return Ok(None);
        }
        Ok(Config::load(&self.config_path())
            .context("can't tell whether this home is a spoke")?
            .spoke)
    }

    /// The config, with the owner's tag edits from `tags.json` (`crate::tags::OwnerTags`);
    /// none when that file can't be read.
    pub fn load_config(&self) -> Result<Config> {
        let mut config = Config::load(&self.config_path())?;
        config.tags.owner = crate::tags::OwnerTags::load(self).unwrap_or_default();
        Ok(config)
    }

    /// Whether a path is this home or inside it. A session that worked inside the home was run
    /// by the previous build's steward.
    pub fn is_inside(&self, path: &str) -> bool {
        let root = self.root.to_string_lossy();
        let root = root.trim_end_matches('/');
        path == root || path.starts_with(&format!("{root}/"))
    }
}

/// Refuse a machine name that isn't lowercase letters, digits and `-` (ADR 0010).
pub fn machine_name(name: &str) -> Result<()> {
    let fits = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
    if name.is_empty() || name.len() > 64 || name.starts_with('-') || !name.bytes().all(fits) {
        anyhow::bail!(
            "{name:?} is not a machine name: lowercase letters, digits and -, at most 64"
        );
    }
    Ok(())
}

fn lock_at(path: &Path, name: &str) -> Result<fs::File> {
    if let Some(dir) = path.parent() {
        create_dir_private(dir)?;
    }
    let f = private_file()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    f.lock().with_context(|| format!("take the {name} lock"))?;
    Ok(f)
}

pub fn user_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_in_the_layout_has_its_path() {
        let h = Home::new("/h");
        let p = |p: PathBuf| p.to_string_lossy().into_owned();
        assert_eq!(p(h.config_path()), "/h/config.toml");
        assert_eq!(p(h.manifest_path()), "/h/raw/manifest.jsonl");
        assert_eq!(p(h.session_list_path()), "/h/sessions/index.jsonl");
        let (k, id) = (Harness::Codex, "s1");
        assert_eq!(p(h.session_dir(None, k, id)), "/h/sessions/codex/s1");
        assert_eq!(
            p(h.meta_path(None, k, id)),
            "/h/sessions/codex/s1/meta.json"
        );
        assert_eq!(
            p(h.events_path(None, k, id)),
            "/h/sessions/codex/s1/events.jsonl"
        );
        assert_eq!(
            p(h.findings_path(None, k, id)),
            "/h/sessions/codex/s1/findings.json"
        );
        assert_eq!(
            p(h.card_path(None, k, id)),
            "/h/sessions/codex/s1/card.json"
        );
        let nas = Some("nas");
        assert_eq!(
            p(h.manifest_path_of(nas)),
            "/h/machines/nas/raw/manifest.jsonl"
        );
        assert_eq!(
            p(h.card_path(nas, k, id)),
            "/h/sessions/machines/nas/codex/s1/card.json"
        );
        assert_eq!(p(h.index_dir()), "/h/index");
        assert_eq!(p(h.vectors_path()), "/h/index/vectors.bin");
        assert_eq!(p(h.card_failures_path()), "/h/state/cards.json");
        assert_eq!(p(h.detail_failures_path()), "/h/state/details.json");
        assert_eq!(p(h.state_dir()), "/h/state");
        assert_eq!(p(h.read_log_path()), "/h/log/reads.jsonl");
        assert_eq!(p(h.export_log_path()), "/h/log/exports.jsonl");
        assert_eq!(p(h.runs_dir()), "/h/runs");
        assert_eq!(p(h.reveals_dir()), "/h/reveals");
        assert_eq!(p(h.reveal_path("0a1b")), "/h/reveals/0a1b.json");
        assert_eq!(p(h.fingerprint_key_path()), "/h/keys/fingerprint.key");
        assert_eq!(
            p(h.findings_lock_path(k, id)),
            "/h/state/findings/codex/s1.lock"
        );
    }

    #[test]
    fn a_machine_name_is_lowercase_letters_digits_and_dashes() {
        for ok in ["nas", "build-box-2"] {
            assert!(machine_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-nas", "Nas", "nas/x", "..", "a:b", &"x".repeat(65)] {
            assert!(machine_name(bad).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_spoke_folder_that_cannot_be_read_is_an_error_not_a_spoke_left_out() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let h = Home::new(dir.path());
        for m in ["nas", "box"] {
            fs::create_dir_all(h.raw_dir_of(Some(m))).unwrap();
        }
        fs::create_dir_all(h.machines_dir().join("empty")).unwrap();
        assert_eq!(h.machines().unwrap(), ["box", "nas"]);
        let nas = h.machines_dir().join("nas");
        fs::set_permissions(&nas, fs::Permissions::from_mode(0o000)).unwrap();
        let got = h.machines();
        fs::set_permissions(&nas, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(got.is_err(), "{got:?}");
    }

    #[test]
    fn a_backup_holds_what_cannot_be_rebuilt_and_cards_and_vectors_only_on_request() {
        use Keep::*;
        for (rel, want) in [
            ("config.toml", Some(Always)),
            ("raw/ab/abcdef", Some(Always)),
            ("raw/manifest.jsonl", Some(Always)),
            ("keys/fingerprint.key", Some(Always)),
            ("keys/backup.key", Some(Never)),
            ("log/reads.jsonl", Some(Always)),
            ("log/backups.jsonl", Some(Always)),
            ("log/exports.jsonl", Some(Always)),
            ("reveals/r1.json", Some(Always)),
            ("machines/nas/raw/ab/abcdef", Some(Always)),
            ("machines/nas/raw/manifest.jsonl", Some(Always)),
            ("machines/nas/other", None),
            ("sessions/machines/nas/codex/s1/card.json", Some(Cards)),
            ("sessions/machines/nas/codex/s1/meta.json", Some(Never)),
            ("sessions/codex/s1/card.json", Some(Cards)),
            ("sessions/codex/s1/events.jsonl", Some(Never)),
            ("sessions/index.jsonl", Some(Never)),
            ("index/vectors.bin", Some(Vectors)),
            ("index/words/seg-1.bin", Some(Never)),
            ("state/cards.json", Some(Never)),
            ("runs/2026-11-22.jsonl", Some(Never)),
            ("exports/old", Some(Never)),
            ("raw", None),
            ("keys/other.key", None),
            ("log/other.jsonl", None),
            ("stray.txt", None),
        ] {
            assert_eq!(Home::keep(rel), want, "{rel}");
        }
    }
}
