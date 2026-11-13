use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::Config;
use crate::util::set_private;

/// A Distill home directory. `DISTILL_HOME` overrides the default `~/distill`.
#[derive(Debug, Clone)]
pub struct Home {
    pub root: PathBuf,
}

impl Home {
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

    /// Create the directory tree with owner-only permissions.
    pub fn ensure(&self) -> Result<()> {
        for dir in [
            self.root.clone(),
            self.raw_dir(),
            self.sessions_dir(),
            self.recipes_dir(),
            self.exports_dir(),
            self.runs_dir(),
            self.state_dir(),
            self.steward_dir(),
        ] {
            fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
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
    pub fn index_path(&self) -> PathBuf {
        self.sessions_dir().join("index.jsonl")
    }
    pub fn recipes_dir(&self) -> PathBuf {
        self.root.join("recipes")
    }
    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
    }
    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }
    /// Working directory for steward agent runs. Sessions whose cwd is inside the home are the
    /// steward's own, and are tagged so datasets leave them out.
    pub fn steward_dir(&self) -> PathBuf {
        self.root.join("steward")
    }

    /// Hold the archive writer lock until the returned file is dropped. Sweeps from the timer
    /// and from a person can overlap; only one may write `raw/` at a time.
    pub fn lock_archive(&self) -> Result<fs::File> {
        self.lock("archive")
    }

    /// Hold a named lock under `state/` until the returned file is dropped.
    pub fn lock(&self, name: &str) -> Result<fs::File> {
        fs::create_dir_all(self.state_dir())?;
        let f = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.state_dir().join(format!("{name}.lock")))?;
        f.lock().with_context(|| format!("take the {name} lock"))?;
        Ok(f)
    }

    pub fn load_config(&self) -> Result<Config> {
        Config::load(&self.config_path())
    }

    pub fn is_inside(&self, path: &str) -> bool {
        let root = self.root.to_string_lossy();
        let root = root.trim_end_matches('/');
        path == root || path.starts_with(&format!("{root}/"))
    }
}

pub fn user_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}
