//! The steward: an agent, run through a harness the user already has, that keeps the derived
//! layer current without being asked. It reads sessions and answers in JSON; Distill writes the
//! files, so the archive stays out of the agent's reach.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
        Harness::Ollama => {
            let model = s
                .model
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .context("steward.model must name an installed Ollama model (see `ollama list`)")?;
            // Only a model whose weights are on this machine: a cloud model (under any name)
            // would send the excerpts to Ollama's servers, and a missing one would be
            // downloaded on the spot.
            if !ollama_model_is_local(&ollama_models_dir(), model) {
                bail!(
                    "\"{model}\" is not a model stored on this machine (see `ollama list`; cloud models are not used)"
                );
            }
            // The prompt goes in on stdin and the answer comes back on stdout, so Distill itself
            // never opens a connection. Ollama's server must be running (its app, or `ollama
            // serve`).
            let mut cmd = ollama();
            cmd.args(["run", model, "--hidethinking", "--nowordwrap"]);
            let text = exec_bounded(cmd.current_dir(&dir), prompt, LOCAL_CALL_TIMEOUT)?;
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
            let text = exec_bounded(cmd.current_dir(&dir), prompt, LOCAL_CALL_TIMEOUT)?;
            Ok(Answer {
                text,
                cost_usd: None,
                error: None,
            })
        }
    }
}

/// The models stored on this machine by the local Ollama server (`ollama list`), for setup to
/// offer and for the steward to check against. Cloud models (listed with no size, or named
/// `…-cloud` / `…:cloud`) are left out: they run on Ollama's servers. Empty when Ollama is not
/// installed, its server is not running, or it doesn't answer within a few seconds.
pub fn ollama_models() -> Vec<String> {
    let mut cmd = ollama();
    cmd.arg("list");
    let Ok(out) = exec_bounded(&mut cmd, "", Duration::from_secs(5)) else {
        return Vec::new();
    };
    let dir = ollama_models_dir();
    parse_ollama_list(&out)
        .into_iter()
        .filter(|m| ollama_model_is_local(&dir, m))
        .collect()
}

/// Where Ollama keeps its models: `$OLLAMA_MODELS`, else `~/.ollama/models`.
fn ollama_models_dir() -> PathBuf {
    std::env::var_os("OLLAMA_MODELS")
        .map(PathBuf::from)
        .or_else(|| {
            crate::home::user_home()
                .ok()
                .map(|h| h.join(".ollama/models"))
        })
        .unwrap_or_default()
}

/// Whether `model`'s weights are stored on this machine, read from Ollama's model store rather
/// than from what the CLI prints. A model named as a cloud source (a `cloud` or `…-cloud` tag)
/// is refused outright. Otherwise every manifest Ollama could resolve the name to (its older
/// `manifests/` store and its newer `manifests-v2/`, which it prefers) must name a weights layer
/// whose file is here at the recorded size and a config with no remote host or model, and at
/// least one must exist. Anything that can't be verified counts as not local.
///
/// This reads the store at `dir`. Ollama's server decides what actually runs; one started with
/// a different model directory than Distill sees is outside what this can check.
fn ollama_model_is_local(dir: &Path, model: &str) -> bool {
    let (name, tag) = match model.rsplit_once(':') {
        Some((n, t)) if !t.contains('/') => (n, t),
        _ => (model, "latest"),
    };
    if tag == "cloud" || tag.ends_with("-cloud") || name.ends_with("-cloud") {
        return false;
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == "..")
        || tag.is_empty()
        || tag.contains("..")
    {
        return false;
    }
    // Where each store keeps this name. A bare name is in the `library` namespace of the
    // default registry, and Ollama spells that registry two ways (`registry.ollama.ai`,
    // `ollama.com`), treating them as one; every spelling is checked in both stores.
    const DEFAULT_HOSTS: [&str; 2] = ["registry.ollama.ai", "ollama.com"];
    let (host, rest): (Option<&str>, Vec<&str>) = match parts.as_slice() {
        [m] => (None, vec!["library", m]),
        [ns, m] => (None, vec![ns, m]),
        [h, more @ ..] => (Some(*h), more.to_vec()),
        [] => return false,
    };
    let hosts: Vec<&str> = match host {
        Some(h) if !DEFAULT_HOSTS.contains(&h) => vec![h],
        _ => DEFAULT_HOSTS.to_vec(),
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    for store in ["manifests", "manifests-v2"] {
        for h in &hosts {
            let mut path = dir.join(store).join(h);
            for part in &rest {
                path = path.join(part);
            }
            candidates.push(path);
        }
    }
    let blob = |digest: &str| dir.join("blobs").join(digest.replace(':', "-"));
    let local = |manifest: &Value| {
        let weights = manifest["layers"].as_array().is_some_and(|layers| {
            layers.iter().any(|l| {
                l["mediaType"] == "application/vnd.ollama.image.model"
                    && l["digest"].as_str().is_some_and(|d| {
                        std::fs::metadata(blob(d))
                            .is_ok_and(|m| Some(m.len()) == l["size"].as_u64() && m.len() > 0)
                    })
            })
        });
        let remote = manifest["config"]["digest"]
            .as_str()
            .and_then(|d| std::fs::read(blob(d)).ok())
            .and_then(|c| serde_json::from_slice::<Value>(&c).ok())
            .is_none_or(|c| {
                ["remote_host", "remote_model"]
                    .iter()
                    .any(|k| c.get(*k).is_some_and(|v| !v.is_null() && v != ""))
            });
        weights && !remote
    };
    let mut found = false;
    for path in candidates.iter().map(|c| c.join(tag)) {
        if !path.exists() {
            continue;
        }
        found = true;
        let ok = std::fs::read(&path)
            .ok()
            .and_then(|m| serde_json::from_slice::<Value>(&m).ok())
            .is_some_and(|m| local(&m));
        if !ok {
            return false;
        }
    }
    found
}

fn parse_ollama_list(out: &str) -> Vec<String> {
    out.lines()
        .skip(1)
        .filter_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            let name = *cols.first()?;
            // NAME, ID, SIZE (number and unit), MODIFIED: a stored model has a size.
            let stored = cols
                .get(2)
                .is_some_and(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()));
            let cloud = name.ends_with("-cloud") || name.ends_with(":cloud");
            (stored && !cloud).then(|| name.to_string())
        })
        .collect()
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

/// How long one local model call may take before it is stopped. A card is a few hundred tokens
/// from a prompt of at most about 20,000 characters.
const LOCAL_CALL_TIMEOUT: Duration = Duration::from_secs(600);

/// Run `cmd` with `stdin`, and stop it if it hasn't finished, output included, within
/// `timeout`, so a stalled model server can't hold the steward (and the hourly tick) forever.
/// The command runs in its own process group; on a timeout the whole group is killed, so a
/// wrapper's inference subprocess doesn't outlive it (one that moves itself to another group
/// or session on purpose is outside this). Input and output run on their own
/// threads: a program that exits early still reports its error rather than a broken pipe.
fn exec_bounded(cmd: &mut Command, stdin: &str, timeout: Duration) -> Result<String> {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(cmd, 0);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("start {:?}", cmd.get_program()))?;
    let pid = child.id();
    let mut input = child.stdin.take().expect("piped");
    let prompt = stdin.to_string();
    std::thread::spawn(move || {
        let _ = input.write_all(prompt.as_bytes());
    });
    let (tx, rx) = std::sync::mpsc::channel::<(bool, Vec<u8>)>();
    for (is_out, mut r) in [
        (
            true,
            Box::new(child.stdout.take().expect("piped")) as Box<dyn std::io::Read + Send>,
        ),
        (false, Box::new(child.stderr.take().expect("piped"))),
    ] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            let _ = tx.send((is_out, buf));
        });
    }
    drop(tx);
    let deadline = Instant::now() + timeout;
    // Returns whether the whole group was signalled.
    let stop = |child: &mut std::process::Child| -> bool {
        // The group first (a wrapper's children), then the child itself. `/bin/kill` by path,
        // so a narrow PATH can't make the group signal quietly fail.
        #[cfg(unix)]
        let group = Command::new("/bin/kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        #[cfg(not(unix))]
        let group = false;
        let _ = child.kill();
        let _ = child.wait();
        // With the group gone its pipes close; give the reader threads a moment to finish.
        let until = Instant::now() + Duration::from_secs(1);
        while rx
            .recv_timeout(until.saturating_duration_since(Instant::now()))
            .is_ok()
        {}
        group
    };
    let timed_out = |group: bool| {
        anyhow::anyhow!(
            "{:?} did not finish within {} s and was stopped{}",
            cmd.get_program(),
            timeout.as_secs(),
            if group {
                ""
            } else {
                " (its process group could not be signalled; processes it started may still run)"
            }
        )
    };
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let group = stop(&mut child);
            return Err(timed_out(group));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    // The output must also be complete by the deadline: something the command left running
    // could hold its pipes open.
    let (mut stdout, mut stderr) = (None, None);
    while stdout.is_none() || stderr.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((true, buf)) => stdout = Some(buf),
            Ok((false, buf)) => stderr = Some(buf),
            Err(_) => {
                let group = stop(&mut child);
                return Err(timed_out(group));
            }
        }
    }
    let (stdout, stderr) = (stdout.unwrap_or_default(), stderr.unwrap_or_default());
    if !status.success() {
        bail!(
            "{:?} exited with {}: {}",
            cmd.get_program(),
            status,
            clip(String::from_utf8_lossy(&stderr).trim(), 400)
        );
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

/// Ollama's CLI, also where launchd's PATH may not reach it.
fn ollama() -> Command {
    let bin = ["/usr/local/bin/ollama", "/opt/homebrew/bin/ollama"]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
        .unwrap_or("ollama");
    let mut cmd = Command::new(bin);
    // Only the server on this machine: `OLLAMA_HOST` could otherwise point the CLI at another
    // one, and the excerpts would leave the machine.
    cmd.env("OLLAMA_HOST", "127.0.0.1:11434");
    cmd
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
    let metas = normalize::read_index(home)?;
    let originals = crate::model::Originals::of(&metas);
    Ok(metas
        .into_iter()
        .filter(|m| !m.steward && !m.is_duplicate(&originals) && m.counts.user > 0)
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
    if config.steward.harness == Harness::None {
        bail!(
            "the steward is off: choose a provider in Settings, or set steward.harness in config.toml"
        );
    }
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
/// Steward calls made today, by card runs and recipe proposals together.
pub(crate) fn calls_today(home: &Home) -> Result<u64> {
    Ok(runlog::today(home)?
        .iter()
        .filter(|e| e.step == "steward" || e.step == "propose")
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
    fn a_model_counts_as_local_only_when_its_weights_are_here() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path();
        let put = |name: &str, tag: &str, config: &str, weights: Option<&[u8]>| {
            let mut layers = Vec::new();
            if let Some(w) = weights {
                let d = format!("sha256:{}", crate::util::sha256_hex(w));
                std::fs::create_dir_all(store.join("blobs")).unwrap();
                std::fs::write(store.join("blobs").join(d.replace(':', "-")), w).unwrap();
                layers.push(json!({"mediaType": "application/vnd.ollama.image.model", "digest": d, "size": w.len()}));
            }
            let c = format!("sha256:{}", crate::util::sha256_hex(config.as_bytes()));
            std::fs::create_dir_all(store.join("blobs")).unwrap();
            std::fs::write(store.join("blobs").join(c.replace(':', "-")), config).unwrap();
            let m = store
                .join("manifests/registry.ollama.ai/library")
                .join(name);
            std::fs::create_dir_all(&m).unwrap();
            std::fs::write(
                m.join(tag),
                json!({"config": {"digest": c}, "layers": layers}).to_string(),
            )
            .unwrap();
        };
        put("qwen-local", "latest", "{}", Some(b"weights"));
        // A cloud model copied to a plain name: no weights, a remote host in its config.
        put(
            "reviewer",
            "latest",
            r#"{"remote_host":"https://ollama.com","remote_model":"big"}"#,
            None,
        );
        put(
            "sneaky",
            "latest",
            r#"{"remote_host":"https://ollama.com"}"#,
            Some(b"w"),
        );
        assert!(ollama_model_is_local(store, "qwen-local"));
        assert!(ollama_model_is_local(store, "qwen-local:latest"));
        assert!(!ollama_model_is_local(store, "reviewer"));
        assert!(!ollama_model_is_local(store, "sneaky"));
        assert!(!ollama_model_is_local(store, "missing:7b"));
        // Names Ollama treats as cloud sources are refused whatever the store holds.
        put("qwen-local", "cloud", "{}", Some(b"weights"));
        put("qwen-local", "8b-cloud", "{}", Some(b"weights"));
        assert!(!ollama_model_is_local(store, "qwen-local:cloud"));
        assert!(!ollama_model_is_local(store, "qwen-local:8b-cloud"));
        // A cloud entry for the same name in the newer store, which Ollama prefers, wins.
        let v2 = store.join("manifests-v2/ollama.com/library/qwen-local");
        std::fs::create_dir_all(&v2).unwrap();
        let cloud_cfg = r#"{"remote_host":"https://ollama.com"}"#;
        let d = format!("sha256:{}", crate::util::sha256_hex(cloud_cfg.as_bytes()));
        std::fs::write(store.join("blobs").join(d.replace(':', "-")), cloud_cfg).unwrap();
        std::fs::write(
            v2.join("latest"),
            json!({"config": {"digest": d}, "layers": []}).to_string(),
        )
        .unwrap();
        assert!(!ollama_model_is_local(store, "qwen-local"));
        // The same holds when the name spells out the registry Ollama treats as `ollama.com`.
        assert!(!ollama_model_is_local(
            store,
            "registry.ollama.ai/library/qwen-local"
        ));
        assert!(!ollama_model_is_local(
            store,
            "ollama.com/library/qwen-local"
        ));
        assert!(!ollama_model_is_local(store, "../../etc/passwd"));
    }

    #[test]
    fn a_timeout_covers_output_and_stops_the_whole_group() {
        // Something left running that holds the output open doesn't outlast the deadline.
        let mut lingering = Command::new("sh");
        lingering.args(["-c", "sleep 10 & printf done"]);
        let started = Instant::now();
        assert!(exec_bounded(&mut lingering, "", Duration::from_millis(300)).is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
        // A timed-out wrapper's children are stopped with it.
        let marker = tempfile::tempdir().unwrap();
        let pidfile = marker.path().join("pid");
        let mut wrapper = Command::new("sh");
        wrapper.args([
            "-c",
            &format!("sleep 30 & echo $! > {}; wait", pidfile.display()),
        ]);
        assert!(exec_bounded(&mut wrapper, "", Duration::from_millis(300)).is_err());
        let pid = std::fs::read_to_string(&pidfile).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let alive = Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .unwrap()
            .success();
        assert!(!alive, "the wrapper's child should be stopped");
    }

    #[test]
    fn only_models_stored_on_this_machine_are_offered() {
        let list = "NAME                  ID              SIZE      MODIFIED\n\
                    qwen3.6:35b-a3b       0a1b2c3d4e5f    22 GB     2 days ago\n\
                    gpt-oss:120b-cloud    1b2c3d4e5f6a    -         3 days ago\n\
                    kimi-k3:cloud         2c3d4e5f6a7b    -         1 week ago\n";
        assert_eq!(parse_ollama_list(list), ["qwen3.6:35b-a3b"]);
    }

    #[test]
    fn a_local_call_is_stopped_at_its_deadline_and_early_exits_keep_their_error() {
        let mut slow = Command::new("sh");
        slow.args(["-c", "sleep 5"]);
        let err = exec_bounded(&mut slow, "", Duration::from_millis(300)).unwrap_err();
        assert!(format!("{err:#}").contains("did not finish"), "{err:#}");
        // A program that exits before reading a large prompt reports its own error.
        let mut early = Command::new("sh");
        early.args(["-c", "echo model not found >&2; exit 3"]);
        let err =
            exec_bounded(&mut early, &"x".repeat(1 << 20), Duration::from_secs(10)).unwrap_err();
        assert!(format!("{err:#}").contains("model not found"), "{err:#}");
    }

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
                refs: None,
                git_stalled: false,
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
