use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::source::SourceKind;
use crate::util::write_atomic;

/// Everything set up once. The GUI's setup screens and `distill init` both write this file, and
/// it stays hand-editable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    #[serde(default)]
    pub privacy: Privacy,
    #[serde(default)]
    pub steward: StewardConfig,
    #[serde(default)]
    pub schedule: Schedule,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceConfig {
    pub kind: SourceKind,
    pub enabled: bool,
    /// Override the default data roots for this source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Privacy {
    /// Sessions whose working directory is under one of these paths are never archived.
    #[serde(default)]
    pub exclude_paths: Vec<String>,
    /// Scan and replace secrets when building datasets. Raw is never edited.
    #[serde(default = "yes")]
    pub redact_exports: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            exclude_paths: Vec::new(),
            redact_exports: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    /// Claude Code in print mode (`claude -p`).
    Claude,
    /// Codex in exec mode (`codex exec`).
    Codex,
    /// Any command that reads a prompt on stdin and prints the answer. Used by tests.
    Command,
    /// No steward runs.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StewardConfig {
    pub harness: Harness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Stop spending once this much has been spent in one day. Only harnesses that report cost
    /// (Claude Code) count toward it; others are limited by `max_sessions_per_run` alone.
    pub daily_budget_usd: f64,
    pub max_sessions_per_run: usize,
    /// Program and arguments for `harness = "command"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
}

impl Default for StewardConfig {
    fn default() -> Self {
        Self {
            harness: Harness::Claude,
            model: Some("claude-haiku-5-5".to_string()),
            daily_budget_usd: 2.0,
            max_sessions_per_run: 25,
            command: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Schedule {
    pub sweep_every_minutes: u32,
    pub enrich_every_hours: u32,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            sweep_every_minutes: 60,
            enrich_every_hours: 24,
        }
    }
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sources: SourceKind::ALL
                .iter()
                .map(|&kind| SourceConfig {
                    kind,
                    enabled: true,
                    roots: Vec::new(),
                })
                .collect(),
            privacy: Privacy::default(),
            steward: StewardConfig::default(),
            schedule: Schedule::default(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("no config at {} (run `distill init`)", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)?;
        let text =
            format!("# Distill configuration. Written by setup; safe to edit by hand.\n\n{body}");
        write_atomic(path, text.as_bytes())
    }

    pub fn source(&self, kind: SourceKind) -> Option<&SourceConfig> {
        self.sources.iter().find(|s| s.kind == kind)
    }

    pub fn source_mut(&mut self, kind: SourceKind) -> &mut SourceConfig {
        if let Some(i) = self.sources.iter().position(|s| s.kind == kind) {
            return &mut self.sources[i];
        }
        self.sources.push(SourceConfig {
            kind,
            enabled: false,
            roots: Vec::new(),
        });
        self.sources.last_mut().expect("just pushed")
    }

    /// Roots to read for a source: the override if set, else the defaults.
    pub fn roots_for(&self, kind: SourceKind) -> Vec<PathBuf> {
        match self.source(kind) {
            Some(s) if !s.roots.is_empty() => s.roots.clone(),
            _ => kind.default_roots(),
        }
    }

    pub fn is_excluded(&self, cwd: &str) -> bool {
        self.privacy.exclude_paths.iter().any(|p| {
            let p = p.trim_end_matches('/');
            !p.is_empty() && (cwd == p || cwd.starts_with(&format!("{p}/")))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.privacy.exclude_paths.push("/work/secret".into());
        c.source_mut(SourceKind::Cursor).enabled = false;
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), c);
    }

    #[test]
    fn exclusion_matches_whole_path_components() {
        let mut c = Config::default();
        c.privacy.exclude_paths.push("/work/secret/".into());
        assert!(c.is_excluded("/work/secret"));
        assert!(c.is_excluded("/work/secret/app"));
        assert!(!c.is_excluded("/work/secrets"));
    }
}
