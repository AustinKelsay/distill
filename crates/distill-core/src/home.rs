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
///   sessions/index.jsonl         session_list_path: every session's facts
///   sessions/<harness>/<id>/     session_dir
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
///   log/reads.jsonl              read_log_path
///   runs/                        runs_dir: the run log
///   reveals/                     reveals_dir: one file per reveal request (reveal_path)
///   keys/fingerprint.key         fingerprint_key_path
/// ```
///
/// `sessions/`, `index/` and `state/` can be deleted and rebuilt from `raw/`. Folders a previous
/// build left in a home ([`Home::PREVIOUS_BUILD`]) are ignored, never deleted.
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
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }
    /// The session list: one line of session facts per session, rebuilt by the parse step.
    pub fn session_list_path(&self) -> PathBuf {
        self.sessions_dir().join("index.jsonl")
    }
    /// The folder holding one session's files.
    pub fn session_dir(&self, harness: Harness, session_id: &str) -> PathBuf {
        self.sessions_dir()
            .join(harness.id())
            .join(safe_component(session_id))
    }
    /// A session's facts.
    pub fn meta_path(&self, harness: Harness, session_id: &str) -> PathBuf {
        self.session_dir(harness, session_id).join("meta.json")
    }
    /// Read only by the read path (`crate::read`), so it is not public.
    pub(crate) fn events_path(&self, harness: Harness, session_id: &str) -> PathBuf {
        self.session_dir(harness, session_id).join("events.jsonl")
    }
    pub fn findings_path(&self, harness: Harness, session_id: &str) -> PathBuf {
        self.session_dir(harness, session_id).join("findings.json")
    }
    pub fn card_path(&self, harness: Harness, session_id: &str) -> PathBuf {
        self.session_dir(harness, session_id).join("card.json")
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

    pub fn load_config(&self) -> Result<Config> {
        Config::load(&self.config_path())
    }

    /// Whether a path is this home or inside it. A session that worked inside the home was run
    /// by the previous build's steward.
    pub fn is_inside(&self, path: &str) -> bool {
        let root = self.root.to_string_lossy();
        let root = root.trim_end_matches('/');
        path == root || path.starts_with(&format!("{root}/"))
    }
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
        assert_eq!(p(h.session_dir(k, id)), "/h/sessions/codex/s1");
        assert_eq!(p(h.meta_path(k, id)), "/h/sessions/codex/s1/meta.json");
        assert_eq!(p(h.events_path(k, id)), "/h/sessions/codex/s1/events.jsonl");
        assert_eq!(
            p(h.findings_path(k, id)),
            "/h/sessions/codex/s1/findings.json"
        );
        assert_eq!(p(h.card_path(k, id)), "/h/sessions/codex/s1/card.json");
        assert_eq!(p(h.index_dir()), "/h/index");
        assert_eq!(p(h.vectors_path()), "/h/index/vectors.bin");
        assert_eq!(p(h.card_failures_path()), "/h/state/cards.json");
        assert_eq!(p(h.detail_failures_path()), "/h/state/details.json");
        assert_eq!(p(h.state_dir()), "/h/state");
        assert_eq!(p(h.read_log_path()), "/h/log/reads.jsonl");
        assert_eq!(p(h.runs_dir()), "/h/runs");
        assert_eq!(p(h.reveals_dir()), "/h/reveals");
        assert_eq!(p(h.reveal_path("0a1b")), "/h/reveals/0a1b.json");
        assert_eq!(p(h.fingerprint_key_path()), "/h/keys/fingerprint.key");
        assert_eq!(
            p(h.findings_lock_path(k, id)),
            "/h/state/findings/codex/s1.lock"
        );
    }
}
