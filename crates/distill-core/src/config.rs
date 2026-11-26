use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::harness::Harness;
use crate::util::write_atomic;

/// Everything set up once, in `config.toml`. Written with the spec's names (issue #72,
/// "Config"); a file the previous build wrote still loads: its `[[sources]]` are read as
/// `[[harnesses]]`, its `max_sessions_per_run` as `sessions_per_run`, and its settings for removed
/// features (the old steward's harness, model, budget and command, `redact_exports`,
/// `enrich_every_hours`) are ignored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default, alias = "sources")]
    pub harnesses: Vec<HarnessConfig>,
    #[serde(default)]
    pub privacy: Privacy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub steward: StewardConfig,
    #[serde(default)]
    pub schedule: Schedule,
    /// Where backups go (ADR 0009): `[[backup]]` tables.
    #[serde(default, rename = "backup", skip_serializing_if = "Vec::is_empty")]
    pub backups: Vec<BackupTarget>,
    /// `[tags]`: the owner's tag aliases (`crate::tags`).
    #[serde(default, skip_serializing_if = "crate::tags::TagsConfig::is_empty")]
    pub tags: crate::tags::TagsConfig,
}

/// One backup target: a `[[backup]]` table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupTarget {
    /// How the owner names it; the backup log and status use it.
    pub name: String,
    /// `folder` (built in), or any other kind, reached through its adapter
    /// `distill-backup-<kind>` on `PATH` (`docs/backup-adapters.md`).
    pub kind: String,
    /// For a `folder` target: the folder, which must exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Also carry cards, which cost model time to rebuild.
    #[serde(default)]
    pub cards: bool,
    /// Also carry vectors, which cost embedding time to rebuild.
    #[serde(default)]
    pub vectors: bool,
    /// Every other key in the table: an adapter's own settings, handed to it as they are.
    #[serde(flatten)]
    pub settings: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HarnessConfig {
    pub kind: Harness,
    pub enabled: bool,
    /// Override the default data roots for this harness.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Privacy {
    /// Sessions whose project is under one of these paths are never archived.
    #[serde(default)]
    pub exclude_paths: Vec<String>,
    /// Words and names always treated as personal details.
    #[serde(default)]
    pub personal_terms: Vec<String>,
}

/// Whoever runs a model: a runtime on this machine or a remote service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderConfig {
    pub name: String,
    pub kind: ProviderKind,
    /// For a remote provider, whether the owner has marked it trusted. Local ones always are.
    #[serde(default)]
    pub trusted: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Local,
    Remote,
}

/// The models the steward's model steps use. A model that is not set means its step is skipped.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StewardConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    /// Defaults to `card_model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_model: Option<String>,
    /// Route cards and model detection through a trusted OpenAI-compatible provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationConfig>,
    /// How many sessions the card and detail steps take per tick, newest first. The vectors step
    /// is not limited by it: it embeds until its deadline.
    #[serde(default = "default_sessions_per_run", alias = "max_sessions_per_run")]
    pub sessions_per_run: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GenerationConfig {
    pub provider: String,
    /// HTTPS API base URL, including the version path.
    pub base_url: String,
    /// File whose first line is the bearer credential.
    pub api_key_file: PathBuf,
}

impl StewardConfig {
    /// The model for the model detection pass: `detail_model`, else `card_model`.
    pub fn detail_model(&self) -> Option<&str> {
        self.detail_model.as_deref().or(self.card_model.as_deref())
    }
}

impl Default for StewardConfig {
    fn default() -> Self {
        Self {
            card_model: None,
            embedding_model: None,
            detail_model: None,
            generation: None,
            sessions_per_run: default_sessions_per_run(),
        }
    }
}

fn default_sessions_per_run() -> usize {
    25
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Schedule {
    #[serde(default = "default_sweep_every_minutes")]
    pub sweep_every_minutes: u32,
}

fn default_sweep_every_minutes() -> u32 {
    60
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            sweep_every_minutes: default_sweep_every_minutes(),
        }
    }
}

/// Harnesses added after config files were first written, newest last.
const ADDED_LATER: &[Harness] = &[Harness::Goose];

impl Default for Config {
    fn default() -> Self {
        Self {
            harnesses: Harness::ALL
                .iter()
                .map(|&kind| HarnessConfig {
                    kind,
                    enabled: true,
                    roots: Vec::new(),
                })
                .collect(),
            privacy: Privacy::default(),
            providers: Vec::new(),
            steward: StewardConfig::default(),
            schedule: Schedule::default(),
            backups: Vec::new(),
            tags: Default::default(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("no config at {} (run `distill init`)", path.display()))?;
        let mut config: Config =
            toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        // A file that lists every harness DISTILL knew when it was written (setup and `init`
        // write them all) gets the harnesses added since, switched on like the rest. A file
        // that lists only some harnesses means exactly those.
        let earlier = Harness::ALL.iter().filter(|k| !ADDED_LATER.contains(k));
        if earlier.clone().all(|&k| config.harness(k).is_some()) {
            for &kind in ADDED_LATER {
                if config.harness(kind).is_none() {
                    config.harnesses.push(HarnessConfig {
                        kind,
                        enabled: true,
                        roots: Vec::new(),
                    });
                }
            }
        }
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)?;
        let text =
            format!("# DISTILL configuration. Written by setup; safe to edit by hand.\n\n{body}");
        write_atomic(path, text.as_bytes())
    }

    pub fn harness(&self, kind: Harness) -> Option<&HarnessConfig> {
        self.harnesses.iter().find(|s| s.kind == kind)
    }

    pub fn harness_mut(&mut self, kind: Harness) -> &mut HarnessConfig {
        if let Some(i) = self.harnesses.iter().position(|s| s.kind == kind) {
            return &mut self.harnesses[i];
        }
        self.harnesses.push(HarnessConfig {
            kind,
            enabled: false,
            roots: Vec::new(),
        });
        self.harnesses.last_mut().expect("just pushed")
    }

    pub fn enabled(&self, kind: Harness) -> bool {
        self.harness(kind).is_some_and(|h| h.enabled)
    }

    /// Roots to read for a harness: the override if set, else the defaults.
    pub fn roots_for(&self, kind: Harness) -> Vec<PathBuf> {
        match self.harness(kind) {
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
        c.privacy.personal_terms.push("Acme".into());
        c.harness_mut(Harness::Cursor).enabled = false;
        c.providers.push(ProviderConfig {
            name: "example-cloud".into(),
            kind: ProviderKind::Remote,
            trusted: true,
        });
        c.steward.card_model = Some("small-model".into());
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), c);
    }

    #[test]
    fn the_file_is_written_with_the_specs_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.steward.card_model = Some("m".into());
        c.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        for name in [
            "[[harnesses]]",
            "[privacy]",
            "exclude_paths",
            "personal_terms",
            "[steward]",
            "card_model",
            "sessions_per_run",
            "[schedule]",
            "sweep_every_minutes",
        ] {
            assert!(text.contains(name), "no {name} in\n{text}");
        }
        for old in [
            "sources",
            "max_sessions_per_run",
            "redact_exports",
            "enrich_every_hours",
        ] {
            assert!(!text.contains(old), "{old} in\n{text}");
        }
    }

    #[test]
    fn a_file_written_by_the_previous_build_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // The previous build's shape, with every setting it had.
        std::fs::write(
            &path,
            r#"# DISTILL configuration. Written by setup; safe to edit by hand.

[[sources]]
kind = "claude-code"
enabled = true
roots = ["/somewhere/claude"]

[[sources]]
kind = "codex"
enabled = false

[privacy]
exclude_paths = ["/work/private"]
redact_exports = true

[steward]
harness = "ollama"
model = "some-model:8b"
daily_budget_usd = 2.0
max_sessions_per_run = 7
command = ["llama-cli", "-m", "x.gguf"]

[schedule]
sweep_every_minutes = 30
enrich_every_hours = 24
"#,
        )
        .unwrap();
        let c = Config::load(&path).unwrap();
        assert_eq!(
            c.harnesses.len(),
            2,
            "a file naming some harnesses means those"
        );
        assert!(c.enabled(Harness::ClaudeCode));
        assert!(!c.enabled(Harness::Codex));
        assert_eq!(
            c.roots_for(Harness::ClaudeCode),
            [PathBuf::from("/somewhere/claude")]
        );
        assert_eq!(c.privacy.exclude_paths, ["/work/private"]);
        assert_eq!(c.steward.sessions_per_run, 7);
        // The old steward's model is not carried over: its step stays off until set.
        assert_eq!(c.steward.card_model, None);
        assert_eq!(c.schedule.sweep_every_minutes, 30);

        // Saved again, it is written with the new names.
        c.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("[[harnesses]]") && !text.contains("[[sources]]"),
            "{text}"
        );
        assert_eq!(Config::load(&path).unwrap(), c);
    }

    #[test]
    fn detail_model_defaults_to_card_model() {
        let mut s = StewardConfig::default();
        assert_eq!(s.detail_model(), None);
        s.card_model = Some("a".into());
        assert_eq!(s.detail_model(), Some("a"));
        s.detail_model = Some("b".into());
        assert_eq!(s.detail_model(), Some("b"));
    }

    #[test]
    fn harnesses_added_since_the_file_was_written_are_collected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.harnesses.retain(|s| s.kind != Harness::Goose);
        c.harnesses[0].enabled = false;
        c.save(&path).unwrap();
        let back = Config::load(&path).unwrap();
        assert!(back.enabled(Harness::Goose));
        assert!(!back.harnesses[0].enabled, "a harness turned off stays off");

        // A file naming only some harnesses means just those.
        c.harnesses.truncate(2);
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().harnesses.len(), 2);
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
