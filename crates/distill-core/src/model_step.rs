//! What the steward's model steps (`crate::cards`, `crate::details`) share: whether the local
//! runtime can run the step's model, reading one session for it, and the batch it works through
//! each tick, within the tick's deadline, with the sessions it failed for kept under `state/` so
//! they are retried after the rest. Each step keeps its own report, since what it counts differs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::model::SessionMeta;
use crate::read::Read;
use crate::runtime::{LocalRuntime, RuntimeError, RuntimeState};
use crate::tick::Deadline;
use crate::util::{now_rfc3339, write_json_pretty};

/// A session whose last attempt failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub at: String,
    pub error: String,
    /// How many attempts in a row have failed.
    pub attempts: u32,
}

/// The sessions whose last attempt failed, by key, from a step's file under `state/`. A file
/// that is missing or can't be read counts as none, so it is rebuilt by trying again.
pub(crate) fn load_failures(path: &Path) -> BTreeMap<String, Failure> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Why the local runtime can't run `model`, the config's `setting`, now, if it can't.
pub(crate) fn runnable(
    runtime: &LocalRuntime,
    setting: &str,
    model: &str,
) -> std::result::Result<(), String> {
    match runtime.state() {
        RuntimeState::NotInstalled => Err("no local runtime is installed".into()),
        RuntimeState::NotRunning => Err("the local runtime is not running".into()),
        RuntimeState::Running(_) if !runtime.stored_here(model) => Err(format!(
            "{setting} \"{model}\" is not stored on this machine"
        )),
        RuntimeState::Running(_) => Ok(()),
    }
}

/// Why a step failed for one session.
pub(crate) struct Failed {
    pub error: String,
    /// What the model wrote held a secret and was discarded (cards).
    pub discarded: bool,
    /// The runtime can't be reached: stop the run, since every other session would fail the
    /// same way.
    pub stop: bool,
}

impl Failed {
    pub fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            discarded: false,
            stop: false,
        }
    }
}

/// What a step's [`run`] did.
pub(crate) struct Ran<T> {
    /// Each result, by session key, in the order run.
    pub results: Vec<(String, std::result::Result<T, Failed>)>,
    /// The deadline passed with sessions of the batch left: they wait for the next tick.
    pub stopped_at_deadline: bool,
}

/// The text the read path served the step's model, or why it served none.
pub(crate) fn served_text(read: Result<Read>) -> std::result::Result<String, Failed> {
    match read {
        Ok(Read::Text(t)) => Ok(t),
        Ok(Read::Refused(why)) => Err(Failed::new(format!("the read was refused: {why}"))),
        Ok(Read::Events(_)) => Err(Failed::new("the read path served no text")),
        Err(e) => Err(Failed::new(format!("the read failed: {e:#}"))),
    }
}

/// A failed call to the model. When the runtime is gone, the run stops.
pub(crate) fn model_failed(e: RuntimeError) -> Failed {
    Failed {
        stop: matches!(e, RuntimeError::NotRunning | RuntimeError::NotStoredHere(_)),
        ..Failed::new(format!("the model failed: {e}"))
    }
}

/// Run `one` for at most `limit` of `pending`: never tried first, then by fewest failed attempts,
/// newest first within each, taking no session once `deadline` has passed; the session under way
/// then is finished, so every session is done whole however long it takes. Failures are recorded
/// in `failures_path`, where those of sessions no longer in `all` are forgotten.
pub(crate) fn run<T>(
    failures_path: &Path,
    all: &[SessionMeta],
    mut pending: Vec<&SessionMeta>,
    limit: usize,
    deadline: Deadline,
    mut one: impl FnMut(&SessionMeta) -> std::result::Result<T, Failed>,
) -> Result<Ran<T>> {
    let mut failures = load_failures(failures_path);
    let attempts = |f: &SessionMeta| failures.get(&f.key()).map_or(0, |x| x.attempts);
    pending.sort_by(|a, b| {
        attempts(a)
            .cmp(&attempts(b))
            .then_with(|| started(b).cmp(&started(a)))
            .then_with(|| a.key().cmp(&b.key()))
    });
    pending.truncate(limit);
    let mut out = Vec::with_capacity(pending.len());
    let mut stopped_at_deadline = false;
    for facts in pending {
        if deadline.passed() {
            stopped_at_deadline = true;
            break;
        }
        let result = one(facts);
        let stop = match &result {
            Ok(_) => {
                failures.remove(&facts.key());
                false
            }
            Err(failed) => {
                let attempts = failures.get(&facts.key()).map_or(0, |x| x.attempts) + 1;
                failures.insert(
                    facts.key(),
                    Failure {
                        at: now_rfc3339(),
                        error: failed.error.clone(),
                        attempts,
                    },
                );
                failed.stop
            }
        };
        out.push((facts.key(), result));
        if stop {
            break;
        }
    }
    let keys: BTreeSet<String> = all.iter().map(SessionMeta::key).collect();
    failures.retain(|k, _| keys.contains(k));
    write_json_pretty(failures_path, &failures)?;
    Ok(Ran {
        results: out,
        stopped_at_deadline,
    })
}

fn started(f: &SessionMeta) -> Option<&str> {
    f.started_at.as_deref().or(f.ended_at.as_deref())
}
