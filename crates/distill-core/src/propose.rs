//! The steward proposes dataset recipes. It reads a digest of what the archive holds (sources,
//! card outcomes, quality and tags, the recipes that already exist) and answers with up to three
//! recipe drafts. Distill checks each one against the real sessions and writes the ones worth
//! having to `recipes/proposed/`. Builds ignore that folder: a proposal becomes a recipe only
//! when someone accepts it, and redaction is always on whatever the draft says.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{Config, Harness};
use crate::export::{self, Filter, Format, Recipe};
use crate::home::Home;
use crate::runlog::{self, RunEntry};
use crate::util::{now_rfc3339, write_atomic, write_json_pretty};

/// Bump when the prompt changes.
pub const PROMPT_VERSION: u32 = 1;
/// Most proposals kept from one answer.
const MAX_PROPOSALS: usize = 3;
/// A proposal must match at least this many sessions to be worth reviewing.
const MIN_MATCHES: u64 = 5;
/// The most one proposal call may spend.
const CALL_BUDGET_USD: f64 = 0.25;
/// How often the timer asks for proposals.
const EVERY_DAYS: i64 = 7;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub name: String,
    pub recipe: Recipe,
    /// Sessions the recipe matched when it was proposed.
    pub matches: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProposeReport {
    pub proposed: Vec<Proposal>,
    /// Drafts that were turned down, with why.
    pub rejected: Vec<String>,
    pub cost_usd: Option<f64>,
}

impl ProposeReport {
    pub fn summary(&self) -> String {
        format!(
            "Proposed {} recipes{}",
            self.proposed.len(),
            if self.rejected.is_empty() {
                String::new()
            } else {
                format!(" ({} drafts turned down)", self.rejected.len())
            }
        )
    }
}

/// What the agent drafts. `redact` is not its call.
#[derive(Debug, Clone, Deserialize)]
struct Draft {
    name: String,
    #[serde(default)]
    description: String,
    format: Format,
    #[serde(default)]
    filter: Filter,
}

pub fn proposals_dir(home: &Home) -> PathBuf {
    home.recipes_dir().join("proposed")
}

fn proposal_path(home: &Home, name: &str) -> PathBuf {
    proposals_dir(home).join(format!("{name}.toml"))
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 40
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Pending proposals, by name.
pub fn list(home: &Home) -> Result<Vec<Proposal>> {
    let Ok(rd) = fs::read_dir(proposals_dir(home)) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for e in rd.filter_map(Result::ok) {
        let path = e.path();
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".toml"))
            .map(str::to_string)
        else {
            continue;
        };
        let text = fs::read_to_string(&path)?;
        let stored: Stored =
            toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        out.push(Proposal {
            name,
            recipe: stored.recipe,
            matches: stored.matches,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// A proposal file: the recipe plus how many sessions it matched.
#[derive(Serialize, Deserialize)]
struct Stored {
    matches: u64,
    #[serde(flatten)]
    recipe: Recipe,
}

/// Turn a proposal into a recipe. The next build includes it.
pub fn accept(home: &Home, name: &str) -> Result<()> {
    if !valid_name(name) {
        bail!("{name:?} is not a recipe name");
    }
    let _lock = home.lock("exports")?;
    let from = proposal_path(home, name);
    let text = fs::read_to_string(&from).with_context(|| format!("no proposal named {name}"))?;
    let stored: Stored = toml::from_str(&text)?;
    if export::recipe_path(home, name).exists() {
        bail!("a recipe named {name} already exists");
    }
    export::save_recipe(home, name, &stored.recipe)?;
    fs::remove_file(&from)?;
    runlog::record(
        home,
        &RunEntry::new("propose", true, format!("Accepted proposed recipe {name}")),
    )
}

pub fn dismiss(home: &Home, name: &str) -> Result<()> {
    if !valid_name(name) {
        bail!("{name:?} is not a recipe name");
    }
    fs::remove_file(proposal_path(home, name)).with_context(|| format!("no proposal named {name}"))
}

/// What the agent sees: counts only, never session text.
pub fn digest(home: &Home) -> Result<Value> {
    let candidates = export::candidates(home)?;
    let mut by_source: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut outcomes: BTreeMap<String, u64> = BTreeMap::new();
    let mut quality: BTreeMap<u8, u64> = BTreeMap::new();
    let mut tags: BTreeMap<String, u64> = BTreeMap::new();
    let mut eligible = 0u64;
    for (m, c) in &candidates {
        if m.steward || m.counts.user == 0 {
            continue;
        }
        eligible += 1;
        let entry = by_source.entry(m.source.id().to_string()).or_default();
        entry.0 += 1;
        if let Some(c) = c {
            entry.1 += 1;
            *outcomes
                .entry(
                    serde_json::to_value(c.outcome)?
                        .as_str()
                        .unwrap_or("")
                        .into(),
                )
                .or_default() += 1;
            *quality.entry(c.quality).or_default() += 1;
            for t in &c.tags {
                *tags.entry(t.clone()).or_default() += 1;
            }
        }
    }
    let mut top: Vec<(String, u64)> = tags.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(40);
    let mut existing = serde_json::Map::new();
    for name in export::list_recipes(home)? {
        if let Ok(r) = export::load_recipe(home, &name) {
            existing.insert(
                name,
                json!({"description": r.description, "filter": r.filter}),
            );
        }
    }
    for p in list(home)? {
        existing.insert(
            p.name,
            json!({"description": p.recipe.description, "filter": p.recipe.filter, "pending": true}),
        );
    }
    Ok(json!({
        "sessions": eligible,
        "by_source": by_source
            .into_iter()
            .map(|(k, (s, c))| (k, json!({"sessions": s, "carded": c})))
            .collect::<serde_json::Map<_, _>>(),
        "card_outcomes": outcomes,
        "card_quality": quality,
        "top_tags": top,
        "existing_recipes": existing,
    }))
}

fn prompt(digest: &Value) -> String {
    format!(
        "You curate datasets from one developer's archive of coding-agent sessions, for \
fine-tuning models and for the developer's own analysis. Below is a summary of the archive \
(counts only) and the recipes that already exist. Propose up to {MAX_PROPOSALS} new recipes \
that would each make a distinct, useful dataset. Don't repeat an existing recipe. Prefer \
recipes built on the steward's cards (quality, outcomes, tags), since those mark good \
examples.\n\n\
A recipe is JSON: {{\"name\": \"kebab-case\", \"description\": \"one sentence\", \
\"format\": \"chat-messages\" | \"trajectory\", \"filter\": {{\"sources\": [source ids, \
empty = all], \"min_quality\": 1-5, \"outcomes\": [\"success\" | \"partial\" | \"failed\" | \
\"abandoned\" | \"unknown\"], \"tags\": [any one of these card tags], \"since\": \
\"YYYY-MM-DD\", \"min_tool_calls\": n}}}}. Every filter field is optional. Use \
chat-messages for fine-tuning data and trajectory for analysis.\n\n\
Answer with only a JSON array of recipes.\n\nArchive summary:\n{}",
        serde_json::to_string_pretty(digest).unwrap_or_default()
    )
}

/// Pull the JSON array out of an answer, tolerating prose or code fences around it.
fn drafts(answer: &str) -> Result<Vec<Value>> {
    let start = answer.find('[').context("answer has no JSON array")?;
    let end = answer.rfind(']').context("answer has no JSON array")?;
    if end < start {
        bail!("answer has no JSON array");
    }
    serde_json::from_str(&answer[start..=end]).context("answer is not a JSON array")
}

pub fn propose(home: &Home, config: &Config) -> Result<ProposeReport> {
    // Shares the day's budget with card writing, so the two never spend it at once.
    let _lock = home.lock("steward")?;
    let mut budget = CALL_BUDGET_USD;
    if config.steward.harness == Harness::Claude {
        let left = config.steward.daily_budget_usd - runlog::spent_today(home)?;
        if left < 0.01 {
            bail!("no steward budget left today");
        }
        budget = budget.min(left);
    }
    let digest = digest(home)?;
    let answer = crate::steward::ask(home, config, &prompt(&digest), budget)?;
    let mut report = ProposeReport {
        cost_usd: answer.cost_usd,
        ..Default::default()
    };
    let result = match &answer.error {
        Some(e) => Err(anyhow::anyhow!("{e}")),
        None => drafts(&answer.text).map(|d| {
            let (kept, rejected) = check(home, d);
            report.proposed = kept;
            report.rejected = rejected;
        }),
    };
    if result.is_ok() {
        for p in &report.proposed {
            write_atomic(
                &proposal_path(home, &p.name),
                toml::to_string_pretty(&Stored {
                    matches: p.matches,
                    recipe: p.recipe.clone(),
                })?
                .as_bytes(),
            )?;
        }
        write_json_pretty(
            &home.state_dir().join("propose.json"),
            &json!({"last_at": now_rfc3339(), "prompt_version": PROMPT_VERSION}),
        )?;
    }
    let mut entry = RunEntry::new(
        "propose",
        result.is_ok(),
        match &result {
            Ok(()) => report.summary(),
            Err(e) => format!("Recipe proposal failed: {e:#}"),
        },
    );
    entry.cost_usd = answer.cost_usd;
    entry.details = serde_json::to_value(&report)?;
    runlog::record(home, &entry)?;
    result.map(|()| report)
}

/// Keep the drafts that are well formed, new, and match enough sessions.
fn check(home: &Home, drafts: Vec<Value>) -> (Vec<Proposal>, Vec<String>) {
    let candidates = export::candidates(home).unwrap_or_default();
    let mut taken: HashSet<String> = export::list_recipes(home)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let mut filters: Vec<Filter> = taken
        .iter()
        .filter_map(|n| export::load_recipe(home, n).ok())
        .map(|r| r.filter)
        .collect();
    for p in list(home).unwrap_or_default() {
        taken.insert(p.name);
        filters.push(p.recipe.filter);
    }
    let mut kept = Vec::new();
    let mut rejected = Vec::new();
    for v in drafts {
        let label = v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("(unnamed)")
            .to_string();
        let mut reject = |why: &str| rejected.push(format!("{label}: {why}"));
        if kept.len() >= MAX_PROPOSALS {
            reject("more than the limit");
            continue;
        }
        let d: Draft = match serde_json::from_value(v) {
            Ok(d) => d,
            Err(e) => {
                reject(&format!("not a recipe ({e})"));
                continue;
            }
        };
        if !valid_name(&d.name) {
            reject("name must be lowercase kebab-case, at most 40 characters");
            continue;
        }
        if taken.contains(&d.name) {
            reject("a recipe or proposal with this name exists");
            continue;
        }
        if d.filter.min_quality.is_some_and(|q| !(1..=5).contains(&q)) {
            reject("min_quality must be 1 to 5");
            continue;
        }
        if let Some(s) = &d.filter.since
            && NaiveDate::parse_from_str(s, "%Y-%m-%d").is_err()
        {
            reject("since must be YYYY-MM-DD");
            continue;
        }
        if filters.contains(&d.filter) {
            reject("same filter as an existing recipe");
            continue;
        }
        let matches = candidates
            .iter()
            .filter(|(m, c)| d.filter.accepts(m, c.as_ref()))
            .count() as u64;
        if matches < MIN_MATCHES {
            reject(&format!("matches only {matches} sessions"));
            continue;
        }
        taken.insert(d.name.clone());
        filters.push(d.filter.clone());
        kept.push(Proposal {
            name: d.name,
            recipe: Recipe {
                description: d.description,
                format: d.format,
                filter: d.filter,
                redact: true,
            },
            matches,
        });
    }
    (kept, rejected)
}

/// Whether the timer should ask for proposals: weekly, and only once the last ones are dealt
/// with.
pub fn due(home: &Home, config: &Config) -> bool {
    if config.steward.harness == Harness::None || !list(home).unwrap_or_default().is_empty() {
        return false;
    }
    let last = fs::read_to_string(home.state_dir().join("propose.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("last_at")?.as_str().map(str::to_string))
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok());
    last.is_none_or(|t| Utc::now() - t.with_timezone(&Utc) >= Duration::days(EVERY_DAYS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::Archive;
    use crate::card::{Card, CardProducer, Outcome};
    use crate::source::SourceKind;

    fn session(i: usize) -> String {
        format!(
            r#"{{"type":"user","cwd":"/w","sessionId":"s{i}","timestamp":"2026-11-0{}T00:00:00Z","message":{{"role":"user","content":"task {i}"}}}}
{{"type":"assistant","timestamp":"2026-11-01T00:00:01Z","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t","name":"Bash","input":{{"command":"ls"}}}}]}}}}
"#,
            1 + i % 9
        )
    }

    /// A home with ten Claude Code sessions, each carded; half tagged `rust`, half `docs`.
    fn home_with_cards(dir: &std::path::Path) -> Home {
        let home = Home::new(dir.join("home"));
        home.ensure().unwrap();
        {
            let _lock = home.lock_archive().unwrap();
            let mut archive = Archive::open(&home).unwrap();
            for i in 0..10 {
                archive
                    .put(
                        SourceKind::ClaudeCode,
                        &format!("/u/.claude/projects/-w/s{i}.jsonl"),
                        "f",
                        session(i).as_bytes(),
                    )
                    .unwrap();
            }
        }
        crate::normalize::normalize(&home, false).unwrap();
        for (i, m) in crate::normalize::read_index(&home)
            .unwrap()
            .iter()
            .enumerate()
        {
            let card = Card {
                task: "t".into(),
                outcome: Outcome::Success,
                quality: 4,
                corrections: vec![],
                tags: if i % 2 == 0 {
                    vec!["rust".into()]
                } else {
                    vec!["docs".into()]
                },
                summary: "s".into(),
                produced_by: CardProducer {
                    harness: "command".into(),
                    model: None,
                    prompt_version: crate::card::PROMPT_VERSION,
                    written_at: now_rfc3339(),
                    cost_usd: None,
                    input_sha256: Some(m.raw.sha256.clone()),
                },
            };
            crate::card::save(&crate::normalize::session_dir(&home, m), &card).unwrap();
        }
        home
    }

    fn stub_config(dir: &std::path::Path, answer: &str) -> Config {
        let script = dir.join("stub.sh");
        fs::write(
            &script,
            format!("#!/bin/sh\ncat > /dev/null\ncat <<'EOF'\n{answer}\nEOF\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut c = Config::default();
        c.steward.harness = Harness::Command;
        c.steward.command = vec![script.to_string_lossy().into_owned()];
        c
    }

    #[test]
    fn keeps_drafts_that_match_enough_sessions_and_accepting_makes_a_recipe() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_with_cards(dir.path());
        export::save_recipe(&home, "good-sessions", &export::default_recipe()).unwrap();
        let answer = r#"Here you go:
```json
[
  {"name": "all-successes", "description": "Every success", "format": "trajectory",
   "filter": {"outcomes": ["success"]}},
  {"name": "good-sessions", "format": "chat-messages", "filter": {}},
  {"name": "rust-only", "description": "Rust work", "format": "chat-messages",
   "filter": {"tags": ["rust"], "min_quality": 4}},
  {"name": "Bad Name", "format": "trajectory"},
  {"name": "nothing", "format": "trajectory", "filter": {"tags": ["haskell"]}},
  {"name": "no-redact", "format": "chat-messages", "redact": false,
   "filter": {"tags": ["docs"]}}
]
```"#;
        let r = propose(&home, &stub_config(dir.path(), answer)).unwrap();
        let names: Vec<_> = r.proposed.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["all-successes", "rust-only", "no-redact"]);
        assert_eq!(r.proposed[0].matches, 10);
        assert_eq!(r.proposed[1].matches, 5);
        // The draft can't turn redaction off.
        assert!(r.proposed[2].recipe.redact);
        assert_eq!(r.rejected.len(), 3, "{:?}", r.rejected);
        assert!(
            r.rejected
                .iter()
                .any(|w| w.starts_with("nothing: matches only 0"))
        );

        // Proposals wait in their own folder; builds don't see them.
        assert_eq!(export::list_recipes(&home).unwrap(), vec!["good-sessions"]);
        assert_eq!(list(&home).unwrap().len(), 3);
        assert!(!due(&home, &Config::default()), "pending proposals first");

        accept(&home, "all-successes").unwrap();
        assert!(export::load_recipe(&home, "all-successes").unwrap().redact);
        assert_eq!(list(&home).unwrap().len(), 2);
        dismiss(&home, "rust-only").unwrap();
        dismiss(&home, "no-redact").unwrap();
        assert!(list(&home).unwrap().is_empty());
        // Asked this week already.
        assert!(!due(&home, &Config::default()));
        assert!(accept(&home, "rust-only").is_err());
    }
}
