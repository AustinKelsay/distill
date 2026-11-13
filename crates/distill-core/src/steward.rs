//! The steward: an agent, run through a harness the user already has, that keeps the derived
//! layer current without being asked. It reads sessions and answers in JSON; Distill writes the
//! files, so the archive stays out of the agent's reach.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::card::{self, CardDraft, CardProducer, PROMPT_VERSION};
use crate::config::{Config, Harness};
use crate::home::Home;
use crate::model::{Event, EventKind, SessionMeta};
use crate::normalize;
use crate::runlog::{self, RunEntry};
use crate::source::SourceKind;
use crate::util::{clip, now_rfc3339, write_atomic};

pub struct Answer {
    pub text: String,
    /// What the call cost, when the harness reports it. Set even when the call failed, so
    /// spend is always counted.
    pub cost_usd: Option<f64>,
    pub error: Option<String>,
}

/// Ask the configured harness one question with no tools.
pub fn ask(home: &Home, config: &Config, prompt: &str, budget_usd: f64) -> Result<Answer> {
    let s = &config.steward;
    let dir = home.steward_dir();
    std::fs::create_dir_all(&dir)?;
    match s.harness {
        Harness::None => bail!("steward is turned off (harness = \"none\")"),
        Harness::Claude => {
            let mut cmd = Command::new("claude");
            cmd.args([
                "-p",
                "--output-format",
                "json",
                "--no-session-persistence",
                "--tools",
                "",
            ])
            .args(["--max-budget-usd", &format!("{budget_usd:.2}")]);
            if let Some(m) = &s.model {
                cmd.args(["--model", m]);
            }
            claude_answer(exec_raw(cmd.current_dir(&dir), prompt)?)
        }
        Harness::Codex => {
            let last = dir.join(".codex-last-message");
            let mut cmd = Command::new("codex");
            cmd.args([
                "exec",
                "--skip-git-repo-check",
                "--sandbox",
                "read-only",
                "-o",
            ])
            .arg(&last);
            if let Some(m) = &s.model {
                cmd.args(["-m", m]);
            }
            cmd.arg("-");
            exec(cmd.current_dir(&dir), prompt)?;
            let text = std::fs::read_to_string(&last).context("codex wrote no answer")?;
            let _ = std::fs::remove_file(&last);
            Ok(Answer {
                text,
                cost_usd: None,
                error: None,
            })
        }
        Harness::Command => {
            let (prog, args) = s
                .command
                .split_first()
                .context("steward.command is empty")?;
            let mut cmd = Command::new(prog);
            cmd.args(args);
            let text = exec(cmd.current_dir(&dir), prompt)?;
            Ok(Answer {
                text,
                cost_usd: None,
                error: None,
            })
        }
    }
}

/// Read `claude -p --output-format json` output. Its cost is kept even when the call failed.
fn claude_answer((ok, stdout, stderr): (bool, String, String)) -> Result<Answer> {
    let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) else {
        bail!("claude did not return JSON: {}", clip(stderr.trim(), 400));
    };
    let text = v
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let cost_usd = v.get("total_cost_usd").and_then(Value::as_f64);
    let error = if !ok || v.get("is_error").and_then(Value::as_bool) == Some(true) {
        Some(format!("claude reported an error: {}", clip(&text, 300)))
    } else {
        None
    };
    Ok(Answer {
        text,
        cost_usd,
        error,
    })
}

/// Run a command with `stdin`, returning whether it succeeded, its stdout and its stderr.
fn exec_raw(cmd: &mut Command, stdin: &str) -> Result<(bool, String, String)> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("start {:?}", cmd.get_program()))?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(stdin.as_bytes())?;
    let out = child.wait_with_output()?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

fn exec(cmd: &mut Command, stdin: &str) -> Result<String> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("start {:?}", cmd.get_program()))?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(stdin.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "{:?} exited with {}: {}",
            cmd.get_program(),
            out.status,
            clip(String::from_utf8_lossy(&out.stderr).trim(), 400)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub const CARD_PROMPT: &str = r#"You are Distill's steward. You review one coding-agent session so it can be used as training data and for personal analysis.

Read the session digest below and answer with only a JSON object, no other text:

{
  "task": "one sentence: what the user wanted",
  "outcome": "success | partial | failed | abandoned | unknown",
  "quality": 1-5,
  "corrections": ["each place the user corrected, redirected or overrode the agent, one short line each"],
  "tags": ["2-5 short lowercase topic tags"],
  "summary": "two or three sentences on what happened"
}

Quality is the session's value as an example of good agent work. Use the whole scale; most sessions are a 3:
- 5: rare. A hard task done correctly and efficiently, with clear evidence it worked (tests, verification, a commit that stuck). Worth imitating as is.
- 4: correct and verified, with only minor detours or waste.
- 3: the task got done, but with detours, unverified claims, or a trivial task.
- 2: partly done, much flailing, user had to step in repeatedly, or the result is doubtful.
- 1: failed, wrong, harmful, or nothing useful happened.
Routine read-only checks, lookups and one-line answers are at most 3 unless the reasoning itself is exemplary.
"Corrections" are only places a human (or the delegating agent) pushed back on the agent's work, not the original instructions.
If the user turns are another agent delegating the task (a sub-agent brief, a review request from an orchestrator), add the tag "delegated".
Git commits on the session's branch are evidence of outcome but may include other work on that branch; a commit later reverted is evidence against. Judge only from the digest.

SESSION DIGEST
"#;

/// A compact, bounded view of a session for the steward to read.
pub fn digest(meta: &SessionMeta, events: &[Event]) -> String {
    let mut out = String::new();
    out.push_str(&format!("source: {}\n", meta.source.label()));
    if let Some(t) = &meta.title {
        out.push_str(&format!("title: {}\n", clip(t, 200)));
    }
    if let Some(c) = &meta.cwd {
        out.push_str(&format!("project: {}\n", c.rsplit('/').next().unwrap_or(c)));
    }
    out.push_str(&format!("models: {}\n", meta.models.join(", ")));
    let c = &meta.counts;
    out.push_str(&format!(
        "counts: {} user turns, {} assistant messages, {} tool calls, {} tool errors\n",
        c.user, c.assistant, c.tool_calls, c.tool_errors
    ));
    if let (Some(a), Some(b)) = (&meta.started_at, &meta.ended_at) {
        out.push_str(&format!("time: {a} to {b}\n"));
    }
    match &meta.git {
        // The branch resolved: either it had a commit before the session or the session made
        // its first ones.
        Some(g) if g.branch.is_some() && (g.sha_at_start.is_some() || !g.commits.is_empty()) => out.push_str(&format!(
            "git: branch {}: {} commits on it while the session ran (may include other work), {} later reverted\n",
            g.branch.as_deref().unwrap_or(""),
            g.commits.len(),
            g.reverted
        )),
        Some(_) => out.push_str("git: repository known, branch unknown or gone; commits not attributed\n"),
        None => out.push_str("git: no repository information\n"),
    }
    out.push_str("\nTRANSCRIPT (clipped; tool calls shown by name)\n");
    let mut lines = Vec::new();
    for e in events {
        let line = match e.kind {
            EventKind::User => format!("USER: {}", clip(e.text.as_deref().unwrap_or(""), 800)),
            EventKind::Assistant => {
                format!("AGENT: {}", clip(e.text.as_deref().unwrap_or(""), 400))
            }
            EventKind::ToolCall => format!("  -> {}", e.tool.as_deref().unwrap_or("tool")),
            EventKind::ToolResult if e.is_error == Some(true) => {
                format!(
                    "  !! tool error: {}",
                    clip(e.text.as_deref().unwrap_or(""), 160)
                )
            }
            _ => continue,
        };
        lines.push(line);
    }
    // Keep the start and the end, where the task and the outcome are.
    const MAX: usize = 20_000;
    let total: usize = lines.iter().map(|l| l.len() + 1).sum();
    if total <= MAX {
        out.push_str(&lines.join("\n"));
    } else {
        let (mut head, mut tail) = (Vec::new(), Vec::new());
        let (mut hl, mut tl) = (0, 0);
        for l in &lines {
            if hl + l.len() > MAX / 2 {
                break;
            }
            hl += l.len() + 1;
            head.push(l.as_str());
        }
        for l in lines.iter().rev() {
            if tl + l.len() > MAX / 2 {
                break;
            }
            tl += l.len() + 1;
            tail.push(l.as_str());
        }
        tail.reverse();
        out.push_str(&head.join("\n"));
        out.push_str(&format!(
            "\n... [{} lines omitted] ...\n",
            lines.len() - head.len() - tail.len()
        ));
        out.push_str(&tail.join("\n"));
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StewardReport {
    pub carded: u64,
    pub failed: u64,
    pub remaining: u64,
    pub cost_usd: f64,
    pub stopped_for_budget: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub limit: Option<usize>,
    /// Only report what would be done.
    pub dry_run: bool,
}

/// Sessions that need a card, newest first.
pub fn pending(home: &Home) -> Result<Vec<SessionMeta>> {
    Ok(normalize::read_index(home)?
        .into_iter()
        .filter(|m| !m.steward && m.counts.user > 0)
        .filter(|m| {
            card::load(&normalize::session_dir(home, m))
                .is_none_or(|c| !card::is_current(&c, &m.raw.sha256))
        })
        .collect())
}

/// Write cards for pending sessions within the day's budget.
pub fn run(home: &Home, config: &Config, opts: &RunOptions) -> Result<StewardReport> {
    // One steward at a time, so two runs can't both spend what is left of the day's budget.
    let _lock = home.lock("steward")?;
    let mut report = StewardReport::default();
    let todo = pending(home)?;
    let limit = opts.limit.unwrap_or(config.steward.max_sessions_per_run);
    let counts_cost = config.steward.harness == Harness::Claude;
    let mut remaining_budget = config.steward.daily_budget_usd - runlog::spent_today(home)?;
    // Harnesses that don't report cost get a daily call limit instead of a dollar limit.
    let mut calls_left = config
        .steward
        .max_sessions_per_run
        .saturating_sub(calls_today(home)? as usize);
    let mut calls = 0u64;

    if opts.dry_run {
        report.remaining = todo.len() as u64;
        return Ok(report);
    }
    for meta in todo.iter().take(limit) {
        if (counts_cost && remaining_budget < 0.01) || (!counts_cost && calls_left == 0) {
            report.stopped_for_budget = true;
            break;
        }
        let events = normalize::read_events(home, meta)?;
        let prompt = format!("{CARD_PROMPT}{}", digest(meta, &events));
        let per_call = if counts_cost {
            remaining_budget.min(0.50)
        } else {
            0.0
        };
        calls += 1;
        calls_left = calls_left.saturating_sub(1);
        let result = ask(home, config, &prompt, per_call).and_then(|a| {
            // Count what the call cost before checking its answer.
            if let Some(c) = a.cost_usd {
                report.cost_usd += c;
                remaining_budget -= c;
            }
            if let Some(e) = a.error {
                bail!(e);
            }
            let draft = CardDraft::from_answer(&a.text)?;
            Ok((draft, a.cost_usd))
        });
        match result {
            Ok((draft, cost)) => {
                let card = draft.into_card(CardProducer {
                    harness: format!("{:?}", config.steward.harness).to_lowercase(),
                    model: config.steward.model.clone(),
                    prompt_version: PROMPT_VERSION,
                    written_at: now_rfc3339(),
                    cost_usd: cost,
                    input_sha256: Some(meta.raw.sha256.clone()),
                });
                card::save(&normalize::session_dir(home, meta), &card)?;
                report.carded += 1;
            }
            Err(e) => {
                report.failed += 1;
                report.errors.push(format!("{}: {e:#}", meta.key()));
                // A harness that can't start will fail every session the same way.
                if report.carded == 0 && report.failed >= 3 {
                    break;
                }
            }
        }
    }
    report.remaining = pending(home)?.len() as u64;
    let ok = report.failed == 0 || report.carded > 0;
    let summary = if report.stopped_for_budget && report.carded == 0 {
        "Daily steward budget reached; no cards written".to_string()
    } else {
        format!(
            "Wrote {} cards{}{}; {} sessions still need one",
            report.carded,
            if report.failed > 0 {
                format!(", {} failed", report.failed)
            } else {
                String::new()
            },
            if report.stopped_for_budget {
                ", stopped at the daily budget"
            } else {
                ""
            },
            report.remaining
        )
    };
    let mut entry = RunEntry::new("steward", ok, summary);
    entry.cost_usd = Some(report.cost_usd);
    entry.details = json!({"carded": report.carded, "failed": report.failed, "calls": calls, "errors": report.errors.iter().take(5).collect::<Vec<_>>()});
    runlog::record(home, &entry)?;
    Ok(report)
}

/// Harness calls the steward made today (UTC), from the run log.
fn calls_today(home: &Home) -> Result<u64> {
    Ok(runlog::today(home)?
        .iter()
        .filter(|e| e.step == "steward")
        .filter_map(|e| e.details.get("calls").and_then(Value::as_u64))
        .sum())
}

/// Write a brief an agent can follow to repair a source's parser, from the captures that fail.
pub fn repair_brief(home: &Home, source: SourceKind) -> Result<PathBuf> {
    let failures = normalize::failure_list(home, source);
    if failures.is_empty() {
        bail!("{} has no normalize failures to repair", source.label());
    }
    let archive = crate::archive::Archive::open(home)?;
    let mut brief = format!(
        "# Repair the {label} parser\n\n{n} archived {label} captures fail to normalize. Fix \
         `crates/distill-core/src/normalize/{id}.rs` so they parse, without changing output for \
         captures that already parse.\n\nRules:\n\n- Work on a new git branch. Do not merge or push.\n\
         - Add a test with a small synthetic sample of the new format (never paste real session text).\n\
         - Bump `parser_version` for {id} in `normalize/mod.rs`.\n- Run `cargo test -p distill-core`.\n\
         - Then run `distill normalize --force` against this home and confirm the failures are gone.\n\n\
         ## Failures\n\n",
        label = source.label(),
        id = source.id(),
        n = failures.len()
    );
    for (origin, err) in failures.iter().take(20) {
        let blob = archive
            .latest()
            .get(origin)
            .map(|r| archive.blob_path(&r.sha256));
        brief.push_str(&format!(
            "- `{}`\n  - error: {}\n  - raw bytes: `{}`\n",
            origin,
            clip(err, 300),
            blob.map(|p| p.display().to_string()).unwrap_or_default()
        ));
    }
    let path = home.state_dir().join(format!("repair-{}.md", source.id()));
    write_atomic(&path, brief.as_bytes())?;
    Ok(path)
}

/// Start an agent on a parser repair in a checkout of Distill's source. The agent works on its
/// own branch; merging that branch is the approval.
pub fn repair(home: &Home, config: &Config, source: SourceKind, repo: &Path) -> Result<String> {
    let _lock = home.lock("steward")?;
    let brief_path = repair_brief(home, source)?;
    let brief = std::fs::read_to_string(&brief_path)?;
    let branch = format!("steward/repair-{}-{}", source.id(), &now_rfc3339()[..10]);
    let prompt = format!(
        "{brief}\n\nStart with `git switch -c {branch}`. Use DISTILL_HOME={} when running distill. \
         Commit your fix on that branch and stop.",
        home.root.display()
    );
    let mut cost_usd = None;
    let (ok, summary) = match config.steward.harness {
        Harness::Claude => {
            let remaining = config.steward.daily_budget_usd - runlog::spent_today(home)?;
            if remaining < 0.01 {
                bail!("daily steward budget reached; repair not started");
            }
            let mut cmd = Command::new("claude");
            // Not persisted: the session runs outside the home, so it could not be tagged as
            // the steward's and would otherwise end up in datasets.
            cmd.args(["-p", "--output-format", "json", "--no-session-persistence"])
                .args(["--permission-mode", "acceptEdits"])
                .args([
                    "--allowedTools",
                    "Read",
                    "Edit",
                    "Write",
                    "Glob",
                    "Grep",
                    "Bash(cargo:*)",
                    "Bash(git:*)",
                    "Bash(distill:*)",
                ])
                .args(["--max-budget-usd", &format!("{remaining:.2}")])
                .current_dir(repo);
            match exec_raw(&mut cmd, &prompt).and_then(claude_answer) {
                Ok(a) if a.error.is_some() => {
                    cost_usd = a.cost_usd;
                    (
                        false,
                        format!(
                            "Repair for {} failed: {}",
                            source.label(),
                            a.error.unwrap_or_default()
                        ),
                    )
                }
                Ok(a) => {
                    cost_usd = a.cost_usd;
                    (
                        true,
                        format!(
                            "Repair for {} is on branch {branch}; merge it to approve",
                            source.label()
                        ),
                    )
                }
                Err(e) => (
                    false,
                    format!("Repair for {} failed: {e:#}", source.label()),
                ),
            }
        }
        Harness::Command => {
            let (prog, args) = config
                .steward
                .command
                .split_first()
                .context("steward.command is empty")?;
            let mut cmd = Command::new(prog);
            cmd.args(args)
                .current_dir(repo)
                .env("DISTILL_REPAIR_BRANCH", &branch);
            match exec(&mut cmd, &prompt) {
                Ok(_) => (
                    true,
                    format!(
                        "Repair for {} is on branch {branch}; merge it to approve",
                        source.label()
                    ),
                ),
                Err(e) => (
                    false,
                    format!("Repair for {} failed: {e:#}", source.label()),
                ),
            }
        }
        h => bail!("parser repair needs harness claude or command, not {h:?}"),
    };
    let mut entry = RunEntry::new("repair", ok, summary.clone());
    entry.cost_usd = cost_usd;
    runlog::record(home, &entry)?;
    if !ok {
        bail!("{summary}");
    }
    Ok(branch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::Archive;
    use crate::normalize::NormalizeState;
    use std::collections::BTreeMap;

    #[test]
    fn repair_writes_a_brief_and_runs_the_agent_on_its_own_branch() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        let mut archive = Archive::open(&home).unwrap();
        let rec = archive
            .put(SourceKind::Pi, "/src/a.jsonl", "1", &b"garbage"[..])
            .unwrap()
            .unwrap();
        let mut state = BTreeMap::new();
        state.insert(
            "/src/a.jsonl".to_string(),
            NormalizeState {
                source: SourceKind::Pi,
                sha256: rec.sha256,
                version: 1,
                meta_version: crate::normalize::META_VERSION,
                session: None,
                error: Some("unexpected record".into()),
                empty: false,
            },
        );
        crate::util::write_json_pretty(&home.state_dir().join("normalize.json"), &state).unwrap();

        let brief = std::fs::read_to_string(repair_brief(&home, SourceKind::Pi).unwrap()).unwrap();
        assert!(brief.contains("unexpected record") && brief.contains("/src/a.jsonl"));

        // The stub agent records the branch it was told to use and the prompt it got.
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let mut config = Config::default();
        config.steward.harness = Harness::Command;
        config.steward.command = vec![
            "sh".into(),
            "-c".into(),
            "cat > prompt.txt; echo \"$DISTILL_REPAIR_BRANCH\" > branch.txt".into(),
        ];
        let branch = repair(&home, &config, SourceKind::Pi, &repo).unwrap();
        assert!(branch.starts_with("steward/repair-pi-"));
        assert_eq!(
            std::fs::read_to_string(repo.join("branch.txt"))
                .unwrap()
                .trim(),
            branch
        );
        let prompt = std::fs::read_to_string(repo.join("prompt.txt")).unwrap();
        assert!(prompt.contains(&format!("git switch -c {branch}")));
        assert!(prompt.contains("Do not merge or push"));
        let runs = runlog::recent(&home, 5).unwrap();
        assert_eq!(runs[0].step, "repair");
        assert!(runs[0].ok);
    }

    #[test]
    fn digest_keeps_the_start_and_end_of_long_sessions() {
        let meta: SessionMeta = serde_json::from_value(json!({
            "source": "pi", "session_id": "s", "counts": {"events": 0, "user": 0, "assistant": 0, "tool_calls": 0, "tool_errors": 0},
            "raw": {"origin": "o", "sha256": "x", "size": 1}, "normalizer": {"name": "pi", "version": 1}
        }))
        .unwrap();
        let mut events = vec![Event::text(EventKind::User, "FIRST TASK")];
        for i in 0..2000 {
            events.push(Event::text(
                EventKind::Assistant,
                format!("step {i} {}", "x".repeat(50)),
            ));
        }
        events.push(Event::text(EventKind::User, "LAST WORD"));
        let d = digest(&meta, &events);
        assert!(d.contains("FIRST TASK") && d.contains("LAST WORD") && d.contains("lines omitted"));
        assert!(d.len() < 23_000, "{}", d.len());
    }
}
