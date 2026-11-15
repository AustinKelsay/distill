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
    /// Claude Code in print mode (`claude -p`). Sends session excerpts to Anthropic.
    Claude,
    /// Codex in exec mode (`codex exec`). Sends session excerpts to OpenAI.
    Codex,
    /// An open model in Ollama on this machine (`ollama run <model>`); nothing leaves it.
    Ollama,
    /// Any command that reads a prompt on stdin and prints the answer (`llama-cli -m …`, a
    /// script); where the excerpts go is up to that command.
    Command,
    /// No steward runs.
    None,
}

impl Harness {
    /// Every choice, in the order setup offers them.
    pub const ALL: [Harness; 5] = [
        Harness::Ollama,
        Harness::Command,
        Harness::Claude,
        Harness::Codex,
        Harness::None,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Harness::Claude => "Claude Code",
            Harness::Codex => "Codex",
            Harness::Ollama => "Ollama",
            Harness::Command => "Custom command",
            Harness::None => "Off",
        }
    }

    /// A one-word name, for a picker.
    pub fn short_label(self) -> &'static str {
        match self {
            Harness::Claude => "Claude",
            Harness::Command => "Command",
            h => h.label(),
        }
    }

    /// Where the session excerpts the steward reads end up.
    pub fn privacy(self) -> &'static str {
        match self {
            Harness::Claude => "Sends clipped session excerpts to Anthropic.",
            Harness::Codex => "Sends clipped session excerpts to OpenAI.",
            Harness::Ollama => "Runs an open model on this machine; nothing leaves it.",
            Harness::Command => {
                "Runs your command; excerpts go wherever it sends them (nowhere, for a local model)."
            }
            Harness::None => "No cards are written, and nothing is sent anywhere.",
        }
    }

    /// Whether only this machine sees the excerpts (a custom command is the user's call).
    pub fn local(self) -> bool {
        matches!(self, Harness::Ollama | Harness::None)
    }
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

impl StewardConfig {
    /// The custom command as one line of text, for a settings field.
    pub fn command_text(&self) -> String {
        self.command.join(" ")
    }

    /// Set the custom command from a settings field. Arguments are split on spaces; one with a
    /// space in it (`/models/My Model.gguf`) can't be typed that way, so text that still reads
    /// as the saved command keeps the saved argument list exactly.
    pub fn set_command_text(&mut self, text: &str) {
        if text != self.command_text() {
            self.command = text.split_whitespace().map(str::to_string).collect();
        }
    }
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

/// Sources added after config files were first written, newest last.
const ADDED_LATER: &[SourceKind] = &[SourceKind::Goose];

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
        let mut config: Config =
            toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        // A file that lists every source Distill knew when it was written (setup and `init`
        // write them all) gets the sources added since, switched on like the rest. A file that
        // lists only some sources means exactly those.
        let earlier = SourceKind::ALL.iter().filter(|k| !ADDED_LATER.contains(k));
        if earlier.clone().all(|&k| config.source(k).is_some()) {
            for &kind in ADDED_LATER {
                if config.source(kind).is_none() {
                    config.sources.push(SourceConfig {
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
    fn sources_added_since_the_file_was_written_are_collected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.sources.retain(|s| s.kind != SourceKind::Goose);
        c.sources[0].enabled = false;
        c.save(&path).unwrap();
        let back = Config::load(&path).unwrap();
        assert!(back.source(SourceKind::Goose).is_some_and(|s| s.enabled));
        assert!(!back.sources[0].enabled, "a source turned off stays off");

        // A file naming only some sources means just those.
        c.sources.truncate(2);
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().sources.len(), 2);
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
