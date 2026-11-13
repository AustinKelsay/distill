//! Datasets as recipes: a saved filter, a format, and a redaction pass, rebuilt on demand. A build
//! stays on the machine until someone approves it; only approved builds can be published.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::card::{self, Card, Outcome};
use crate::home::Home;
use crate::model::{Event, EventKind, SessionMeta};
use crate::normalize;
use crate::redact::{Findings, redact, redact_value};
use crate::runlog::{self, RunEntry};
use crate::source::SourceKind;
use crate::util::{now_rfc3339, sha256_hex, write_atomic, write_json_pretty};

/// Bump when the shape of built records changes.
const BUILD_FORMAT: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// `{"messages": [...]}` per session in the common chat fine-tuning shape, with tool calls.
    ChatMessages,
    /// Meta, card and the full event list per session.
    Trajectory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    #[serde(default)]
    pub description: String,
    pub format: Format,
    #[serde(default)]
    pub filter: Filter,
    #[serde(default = "yes")]
    pub redact: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    /// Empty means every source with a parser.
    #[serde(default)]
    pub sources: Vec<SourceKind>,
    /// Require a card with at least this quality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_quality: Option<u8>,
    /// Require a card with one of these outcomes.
    #[serde(default)]
    pub outcomes: Vec<Outcome>,
    /// Sessions started on or after this date (YYYY-MM-DD).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(default)]
    pub min_tool_calls: u64,
}

impl Filter {
    fn needs_card(&self) -> bool {
        self.min_quality.is_some() || !self.outcomes.is_empty()
    }

    pub fn accepts(&self, meta: &SessionMeta, card: Option<&Card>) -> bool {
        // Steward sessions are always left out, so Distill never trains on its own runs.
        if meta.steward {
            return false;
        }
        if !self.sources.is_empty() && !self.sources.contains(&meta.source) {
            return false;
        }
        if let Some(since) = &self.since
            && meta
                .started_at
                .as_deref()
                .is_none_or(|s| s < since.as_str())
        {
            return false;
        }
        if meta.counts.tool_calls < self.min_tool_calls {
            return false;
        }
        if self.needs_card() {
            let Some(c) = card else { return false };
            if self.min_quality.is_some_and(|q| c.quality < q) {
                return false;
            }
            if !self.outcomes.is_empty() && !self.outcomes.contains(&c.outcome) {
                return false;
            }
        }
        true
    }
}

pub fn recipe_path(home: &Home, name: &str) -> PathBuf {
    home.recipes_dir().join(format!("{name}.toml"))
}

pub fn load_recipe(home: &Home, name: &str) -> Result<Recipe> {
    let path = recipe_path(home, name);
    let text =
        fs::read_to_string(&path).with_context(|| format!("no recipe at {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

pub fn save_recipe(home: &Home, name: &str, recipe: &Recipe) -> Result<()> {
    write_atomic(
        &recipe_path(home, name),
        toml::to_string_pretty(recipe)?.as_bytes(),
    )
}

pub fn list_recipes(home: &Home) -> Result<Vec<String>> {
    let mut names: Vec<String> = fs::read_dir(home.recipes_dir())
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| {
                    e.file_name()
                        .to_str()?
                        .strip_suffix(".toml")
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    Ok(names)
}

/// The recipe `distill init` writes: good, successful, tool-using sessions as chat data.
pub fn default_recipe() -> Recipe {
    Recipe {
        description: "Successful sessions the steward rated 4 or 5, as chat fine-tuning data"
            .into(),
        format: Format::ChatMessages,
        filter: Filter {
            min_quality: Some(4),
            outcomes: vec![Outcome::Success],
            min_tool_calls: 1,
            ..Default::default()
        },
        redact: true,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildManifest {
    pub recipe: String,
    pub build: String,
    pub recipe_sha256: String,
    pub format: Format,
    pub records: u64,
    pub data_sha256: String,
    pub redactions: BTreeMap<String, u64>,
    /// Each input as source/session@raw-sha, sorted.
    pub inputs: Vec<String>,
    pub built_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub approved_at: String,
    pub by: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildSummary {
    pub recipe: String,
    pub build: String,
    pub records: u64,
    pub redactions: u64,
    pub approved: bool,
    pub built_at: String,
    pub path: PathBuf,
}

fn build_dir(home: &Home, recipe: &str, build: &str) -> PathBuf {
    home.exports_dir().join(recipe).join(build)
}

/// Build a recipe. The build id is a hash of the recipe and its inputs, so the same inputs
/// always give the same id and the same bytes; an existing build is returned as is.
pub fn build(home: &Home, name: &str) -> Result<BuildSummary> {
    let recipe = load_recipe(home, name)?;
    let recipe_sha = sha256_hex(toml::to_string(&recipe)?.as_bytes());
    let mut chosen: Vec<(SessionMeta, Option<Card>)> = normalize::read_index(home)?
        .into_iter()
        .map(|m| {
            let c = card::load(&normalize::session_dir(home, &m));
            (m, c)
        })
        .filter(|(m, c)| recipe.filter.accepts(m, c.as_ref()))
        .collect();
    chosen.sort_by_key(|(m, _)| m.key());
    let inputs: Vec<String> = chosen
        .iter()
        .map(|(m, _)| format!("{}@{}", m.key(), m.raw.sha256))
        .collect();
    let home_dir = crate::home::user_home()
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let mut findings = Findings::default();
    let mut data = Vec::new();
    for (meta, card) in &chosen {
        let mut events = normalize::read_events(home, meta)?;
        let record = match recipe.format {
            // Redact the structured events before tool inputs are serialized into argument
            // strings, so credential-named fields are still recognizable.
            Format::ChatMessages => {
                if recipe.redact {
                    for e in &mut events {
                        if let Some(t) = &e.text {
                            e.text = Some(redact(t, home_dir.as_deref(), &mut findings));
                        }
                        if let Some(i) = &mut e.input {
                            redact_value(i, home_dir.as_deref(), &mut findings);
                        }
                    }
                }
                chat_record(meta, &events)
            }
            Format::Trajectory => {
                let mut r = json!({"meta": meta, "card": card, "events": events});
                if recipe.redact {
                    redact_value(&mut r, home_dir.as_deref(), &mut findings);
                }
                r
            }
        };
        serde_json::to_writer(&mut data, &record)?;
        data.push(b'\n');
    }
    // The id is a hash of the recipe and the bytes it produced, so any change in what would
    // leave the machine (new cards, a parser fix, new redaction rules) is a new build that
    // needs its own approval, and identical output is the same build.
    let data_sha = sha256_hex(&data);
    let build_id = sha256_hex(format!("v{BUILD_FORMAT}\n{recipe_sha}\n{data_sha}").as_bytes())
        [..16]
        .to_string();
    let dir = build_dir(home, name, &build_id);
    if dir.join("manifest.json").exists() {
        return summary(home, name, &build_id);
    }
    write_atomic(&dir.join("data.jsonl"), &data)?;
    let manifest = BuildManifest {
        recipe: name.to_string(),
        build: build_id.clone(),
        recipe_sha256: recipe_sha,
        format: recipe.format,
        records: chosen.len() as u64,
        data_sha256: data_sha,
        redactions: findings.by_rule.clone(),
        inputs,
        built_at: now_rfc3339(),
    };
    write_json_pretty(&dir.join("manifest.json"), &manifest)?;
    let mut entry = RunEntry::new(
        "export",
        true,
        format!(
            "Built dataset \"{name}\": {} records, {} redactions, waiting for approval",
            manifest.records,
            findings.total()
        ),
    );
    entry.details = json!({"recipe": name, "build": build_id});
    runlog::record(home, &entry)?;
    summary(home, name, &build_id)
}

fn chat_record(meta: &SessionMeta, events: &[Event]) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    for e in events {
        match e.kind {
            EventKind::System => messages.push(json!({"role": "system", "content": e.text})),
            EventKind::User => messages.push(json!({"role": "user", "content": e.text})),
            EventKind::Assistant => messages.push(json!({"role": "assistant", "content": e.text})),
            EventKind::Reasoning => {}
            EventKind::ToolCall => {
                let call = json!({
                    "id": e.call_id,
                    "type": "function",
                    "function": {
                        "name": e.tool,
                        "arguments": e.input.as_ref().map(|i| i.to_string()).unwrap_or_default()
                    }
                });
                // Consecutive calls attach to one assistant turn.
                match messages.last_mut() {
                    Some(last) if last["role"] == "assistant" => {
                        if !last["tool_calls"].is_array() {
                            last["tool_calls"] = json!([]);
                        }
                        last["tool_calls"].as_array_mut().expect("array").push(call);
                    }
                    _ => messages
                        .push(json!({"role": "assistant", "content": null, "tool_calls": [call]})),
                }
            }
            EventKind::ToolResult => {
                messages.push(json!({"role": "tool", "tool_call_id": e.call_id, "content": e.text}))
            }
        }
    }
    json!({"id": meta.key(), "source": meta.source, "models": meta.models, "messages": messages})
}

pub fn summary(home: &Home, recipe: &str, build: &str) -> Result<BuildSummary> {
    let dir = build_dir(home, recipe, build);
    let m: BuildManifest = serde_json::from_str(
        &fs::read_to_string(dir.join("manifest.json"))
            .with_context(|| format!("no build {recipe}/{build}"))?,
    )?;
    Ok(BuildSummary {
        recipe: m.recipe,
        build: m.build,
        records: m.records,
        redactions: m.redactions.values().sum(),
        approved: dir.join("approval.json").exists(),
        built_at: m.built_at,
        path: dir,
    })
}

/// Every build, newest first.
pub fn list_builds(home: &Home) -> Result<Vec<BuildSummary>> {
    let mut out = Vec::new();
    let Ok(recipes) = fs::read_dir(home.exports_dir()) else {
        return Ok(out);
    };
    for r in recipes.filter_map(Result::ok) {
        let Ok(builds) = fs::read_dir(r.path()) else {
            continue;
        };
        for b in builds.filter_map(Result::ok) {
            let (Some(rn), Some(bn)) = (
                r.file_name().to_str().map(str::to_string),
                b.file_name().to_str().map(str::to_string),
            ) else {
                continue;
            };
            if let Ok(s) = summary(home, &rn, &bn) {
                out.push(s);
            }
        }
    }
    out.sort_by(|a, b| b.built_at.cmp(&a.built_at).then(a.recipe.cmp(&b.recipe)));
    Ok(out)
}

/// Approve a build so it may leave the machine. This is the human's decision; the steward never
/// calls it.
pub fn approve(home: &Home, recipe: &str, build: &str, by: &str) -> Result<()> {
    let dir = build_dir(home, recipe, build);
    if !dir.join("manifest.json").exists() {
        bail!("no build {recipe}/{build}");
    }
    write_json_pretty(
        &dir.join("approval.json"),
        &Approval {
            approved_at: now_rfc3339(),
            by: by.to_string(),
        },
    )?;
    runlog::record(
        home,
        &RunEntry::new(
            "export",
            true,
            format!("Approved dataset \"{recipe}\" build {build}"),
        ),
    )
}

/// Copy an approved build's data out of the home. Refuses unapproved builds.
pub fn publish(home: &Home, recipe: &str, build: &str, dest: &Path) -> Result<PathBuf> {
    let s = summary(home, recipe, build)?;
    if !s.approved {
        bail!(
            "build {recipe}/{build} is not approved; approve it in the dashboard or with `distill export approve`"
        );
    }
    fs::create_dir_all(dest)?;
    let out = dest.join(format!("{recipe}-{build}.jsonl"));
    fs::copy(s.path.join("data.jsonl"), &out)?;
    runlog::record(
        home,
        &RunEntry::new(
            "export",
            true,
            format!("Published dataset \"{recipe}\" build {build}"),
        ),
    )?;
    Ok(out)
}
