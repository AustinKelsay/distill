//! The shapes every harness's sessions are turned into: events and session facts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::harness::Harness;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// System or developer instructions.
    System,
    User,
    Assistant,
    Reasoning,
    ToolCall,
    ToolResult,
}

/// One event: a step of a session. Tool calls and results are kept in full, not only chat text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub kind: EventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Tool name, for tool calls (and results when the harness records it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Links a tool call to its result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl Event {
    pub fn new(kind: EventKind) -> Self {
        Self {
            seq: 0,
            kind,
            ts: None,
            text: None,
            tool: None,
            call_id: None,
            input: None,
            is_error: None,
            model: None,
        }
    }
    pub fn text(kind: EventKind, text: impl Into<String>) -> Self {
        let mut e = Self::new(kind);
        e.text = Some(text.into());
        e
    }
    pub fn at(mut self, ts: Option<String>) -> Self {
        self.ts = ts;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Counts {
    pub events: u64,
    pub user: u64,
    pub assistant: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
}

impl Counts {
    pub fn of(events: &[Event]) -> Self {
        let mut c = Counts {
            events: events.len() as u64,
            ..Default::default()
        };
        for e in events {
            match e.kind {
                EventKind::User => c.user += 1,
                EventKind::Assistant => c.assistant += 1,
                EventKind::ToolCall => c.tool_calls += 1,
                EventKind::ToolResult if e.is_error == Some(true) => c.tool_errors += 1,
                _ => {}
            }
        }
        c
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRef {
    pub origin: String,
    pub sha256: String,
    pub size: u64,
    /// The capture hashes of other captures whose bytes the session's events hold (OpenCode's
    /// spilled tool outputs), sorted. They are part of what the session is made from, so
    /// anything built from its events goes out of date when one arrives ([`RawRef::source`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attached: Vec<String>,
}

impl RawRef {
    /// What the session's events are made from, as one hash: the capture hash, or, with
    /// attached captures, a hash of it and theirs. Findings, cards and the indexes record it, so
    /// a session whose events change without its capture changing is scanned again before
    /// anything is built from the new events.
    pub fn source(&self) -> String {
        if self.attached.is_empty() {
            return self.sha256.clone();
        }
        let mut all = self.sha256.clone();
        for h in &self.attached {
            all.push('\n');
            all.push_str(h);
        }
        crate::util::sha256_hex(all.as_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Producer {
    pub name: String,
    pub version: u32,
}

/// Session facts: what DISTILL records about a session without holding any of its text. Written
/// to `meta.json` and the session list, both rebuilt from the raw archive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    /// The spoke the session was captured on (ADR 0010); none for this home's own sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    #[serde(alias = "source")]
    pub harness: Harness,
    pub session_id: String,
    /// The project: the folder the session worked in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session: Option<String>,
    pub counts: Counts,
    pub raw: RawRef,
    pub normalizer: Producer,
    /// The mark on sessions the previous build's steward ran (they worked inside the home).
    /// Kept, get no card, and are left out of search unless asked for.
    #[serde(default)]
    pub steward: bool,
    /// A copy of a session DISTILL also captures from the harness's own files (a T3 Code
    /// thread run on Codex, say). Counted as a duplicate only while its original is in the
    /// archive ([`SessionMeta::is_duplicate`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror: bool,
    /// The harnesses holding the originals of a mirror. Empty on facts written before this was
    /// recorded, which count as mirrors of any harness with sessions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirror_of: Vec<Harness>,
    /// The exact sessions a mirror copies (`harness/session id`), when the harness named them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirror_refs: Vec<String>,
    /// Harnesses of a mirror's runs that name no original; those count as copied when the
    /// harness has sessions at all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirror_unresolved: Vec<Harness>,
}

impl SessionMeta {
    /// `<harness>/<id>`, or `<machine>:<harness>/<id>` for a spoke's session (ADR 0010).
    pub fn key(&self) -> String {
        match &self.machine {
            None => format!("{}/{}", self.harness.id(), self.session_id),
            Some(m) => format!("{m}:{}/{}", self.harness.id(), self.session_id),
        }
    }

    /// A mirror whose originals are in the archive and cover it.
    pub fn is_duplicate(&self, originals: &Originals) -> bool {
        self.mirror && self.copied_in(originals)
    }

    /// Whether `originals` hold what this mirror copies. With the originals named, each must be
    /// there; otherwise their harnesses must have sessions at all. A mirror of sessions that were
    /// never captured (deleted before DISTILL ran, say) is the only copy, so it counts as a
    /// session of its own.
    fn copied_in(&self, originals: &Originals) -> bool {
        // Runs whose original isn't named count as copied when their harness has sessions.
        // Facts from before originals were recorded name none, so they check every harness.
        let unresolved = if self.mirror_refs.is_empty() && self.mirror_unresolved.is_empty() {
            &self.mirror_of
        } else {
            &self.mirror_unresolved
        };
        // A named original must also reach (nearly) as far as the mirror: T3 can carry a
        // native session on after its harness stopped being captured. Cursor dates a turn by
        // when it began and T3 by when each item finished, so a T3 thread can end a whole turn
        // after its Cursor store's last time; that pair gets a turn's worth of slack.
        let slack = |k: &String| {
            if harness_of(k) == Some("cursor") && self.harness == Harness::T3Code {
                CURSOR_TURN_SLACK
            } else {
                COVER_SLACK
            }
        };
        let covers = |k: &String| {
            originals.keys.get(k).is_some_and(|ended| {
                match (
                    ended.as_deref().and_then(ts),
                    self.ended_at.as_deref().and_then(ts),
                ) {
                    (Some(orig), Some(mirror)) => orig + slack(k) >= mirror,
                    _ => true,
                }
            })
        };
        self.mirror_refs.iter().all(covers)
            && unresolved
                .iter()
                .all(|h| originals.harnesses.contains(&(self.machine.clone(), *h)))
    }
}

/// The harness a session key names: `<harness>/…`, or `<machine>:<harness>/…`.
fn harness_of(key: &str) -> Option<&str> {
    let (front, _) = key.split_once('/')?;
    Some(front.rsplit(':').next().unwrap_or(front))
}

/// How much later a mirror may end than its original and still count as a copy of it: T3
/// records a turn a little after the harness does.
const COVER_SLACK: chrono::Duration = chrono::Duration::minutes(15);

/// The same for a T3 thread against a Cursor store, whose times mark when each turn began. A
/// store is captured hourly while its agent runs, so a thread ending later than this ran on
/// after the store stopped being captured.
const CURSOR_TURN_SLACK: chrono::Duration = chrono::Duration::hours(2);

fn ts(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(s).ok()
}

/// The sessions in a session list that are not mirrors, for [`SessionMeta::is_duplicate`]. Only a
/// harness's own sessions count: a mirror is never taken as proof for another mirror, since it
/// may hold less (or more) than the session it copies. When the original is gone, two copies of
/// it both stay.
#[derive(Debug, Default)]
pub struct Originals {
    /// The harnesses with sessions, on each machine.
    pub harnesses: std::collections::HashSet<(Option<String>, Harness)>,
    /// Each original's key, with when it ended.
    pub keys: std::collections::HashMap<String, Option<String>>,
}

impl Originals {
    pub fn of(metas: &[SessionMeta]) -> Self {
        let mut o = Originals::default();
        for m in metas.iter().filter(|m| !m.mirror) {
            o.harnesses.insert((m.machine.clone(), m.harness));
            o.keys.insert(m.key(), m.ended_at.clone());
        }
        o
    }
}

/// Each session an agent started, with the session it was started from: the one at the top of
/// its chain of parents, which the owner started. A delegated task's mirror passes its parent on
/// to the native sessions it names. Sessions with no parent are not in it.
#[derive(Debug, Default)]
pub struct Roots(std::collections::HashMap<String, String>);

impl Roots {
    pub fn of(metas: &[SessionMeta]) -> Self {
        use std::collections::HashMap;
        let mut by_id: HashMap<&str, Vec<&SessionMeta>> = HashMap::new();
        for m in metas {
            by_id.entry(m.session_id.as_str()).or_default().push(m);
        }
        let keys: std::collections::HashSet<String> = metas.iter().map(|m| m.key()).collect();
        // A parent is named by its key, or by its id: one of the same harness wins. Hermes keeps
        // its database's sessions as `db-<id>`, while their parents name the bare id; the
        // database's record wins over a file's.
        // Only a session of the same machine: ids are each machine's own.
        let resolve = |named: &str, harness: Harness, machine: &Option<String>| -> Option<String> {
            if named.contains('/') {
                return keys.contains(named).then(|| named.to_string());
            }
            let db = format!("db-{named}");
            let here = |id: &str| -> Option<Vec<&SessionMeta>> {
                let found: Vec<&SessionMeta> = by_id
                    .get(id)?
                    .iter()
                    .copied()
                    .filter(|m| m.machine == *machine)
                    .collect();
                (!found.is_empty()).then_some(found)
            };
            let found = (harness == Harness::Hermes)
                .then(|| here(db.as_str()))
                .flatten()
                .or_else(|| here(named))
                .or_else(|| here(db.as_str()))?;
            found
                .iter()
                .find(|m| m.harness == harness)
                .or(found.first())
                .map(|m| m.key())
        };
        let mut parent: HashMap<String, String> = HashMap::new();
        for m in metas {
            if let Some(p) = m
                .parent_session
                .as_deref()
                .and_then(|p| resolve(p, m.harness, &m.machine))
            {
                parent.insert(m.key(), p);
            }
        }
        for m in metas.iter().filter(|m| m.mirror) {
            let Some(p) = parent.get(&m.key()).cloned() else {
                continue;
            };
            for r in &m.mirror_refs {
                if r != &p {
                    parent.entry(r.clone()).or_insert_with(|| p.clone());
                }
            }
        }
        // And a copy that names no parent of its own takes the parent of the session it copies
        // (a Hermes file of a database session).
        for m in metas
            .iter()
            .filter(|m| m.mirror && m.parent_session.is_none())
        {
            let found = m.mirror_refs.iter().find_map(|r| parent.get(r)).cloned();
            if let Some(p) = found.filter(|p| *p != m.key()) {
                parent.entry(m.key()).or_insert(p);
            }
        }
        // A session naming a parent that isn't archived: the top of its chain is unknown, and
        // so is that of the native sessions a mirror of it names.
        let mut unknown: std::collections::HashSet<String> = std::collections::HashSet::new();
        for m in metas
            .iter()
            .filter(|m| m.parent_session.is_some() && !parent.contains_key(&m.key()))
        {
            unknown.insert(m.key());
            if m.mirror {
                unknown.extend(
                    m.mirror_refs
                        .iter()
                        .filter(|r| !parent.contains_key(*r))
                        .cloned(),
                );
            }
        }
        let mut roots = HashMap::new();
        for k in parent.keys() {
            let (mut at, mut seen) = (k, std::collections::HashSet::new());
            while let Some(p) = parent.get(at) {
                if !seen.insert(at) {
                    break;
                }
                at = p;
            }
            if at != k && !unknown.contains(at) && !parent.contains_key(at) {
                roots.insert(k.clone(), at.clone());
            }
        }
        Roots(roots)
    }

    /// The key of the session `key` was started from, when an agent started it.
    pub fn of_session(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

/// What a parser returns before DISTILL adds raw references and ids.
#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub models: Vec<String>,
    pub harness_version: Option<String>,
    pub parent_session: Option<String>,
    pub events: Vec<Event>,
    /// For a copy of sessions captured natively elsewhere: the harnesses holding the originals.
    pub mirror_of: Vec<Harness>,
    /// And the originals themselves (`harness/session id`), when known.
    pub mirror_refs: Vec<String>,
    /// Harnesses of runs whose original isn't known.
    pub mirror_unresolved: Vec<Harness>,
    /// Records the parser skipped because it doesn't know their type, by type name only.
    pub skipped: BTreeMap<String, Skipped>,
}

/// What a capture holds of one record type its parser doesn't know: how many records and how
/// many bytes. Never their content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub records: u64,
    pub bytes: u64,
}

/// The longest type name kept for a skipped record.
pub const TYPE_NAME_MAX: usize = 64;

/// A record's type name as it may be stored: only `[A-Za-z0-9_.:-]`, at most
/// [`TYPE_NAME_MAX`] characters, so nothing else in the record can reach the parse step's state.
pub fn type_name(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
        .take(TYPE_NAME_MAX)
        .collect()
}

impl Parsed {
    pub fn is_mirror(&self) -> bool {
        !self.mirror_of.is_empty()
    }

    pub fn add_model(&mut self, m: &str) {
        if !m.is_empty() && !self.models.iter().any(|x| x == m) {
            self.models.push(m.to_string());
        }
    }
    /// Count a record of a type the parser doesn't know, by its type name and size. A record
    /// with no type name isn't counted.
    pub fn skip(&mut self, ty: &str, bytes: usize) {
        let name = type_name(ty);
        if name.is_empty() {
            return;
        }
        let s = self.skipped.entry(name).or_default();
        s.records += 1;
        s.bytes += bytes as u64;
    }

    /// [`Parsed::skip`] for a record held inside another (a block in a message, an item in a
    /// response), named `<within>:<type>` and sized as its JSON serializes.
    pub fn skip_in(&mut self, within: &str, ty: &str, v: &Value) {
        if type_name(ty).is_empty() {
            return;
        }
        let bytes = serde_json::to_string(v).map_or(0, |t| t.len());
        if within.is_empty() {
            self.skip(ty, bytes);
        } else {
            self.skip(&format!("{within}:{ty}"), bytes);
        }
    }

    pub fn push(&mut self, mut e: Event) {
        let empty_text = e.text.as_deref().is_none_or(|t| t.trim().is_empty());
        if empty_text && e.input.is_none() && e.tool.is_none() && e.kind != EventKind::ToolResult {
            return;
        }
        e.seq = self.events.len() as u64;
        self.events.push(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mirror(of: &str) -> SessionMeta {
        serde_json::from_str(&format!(
            r#"{{"harness":"t3code","session_id":"t1","models":[],"counts":{{"events":0,"user":0,"assistant":0,"tool_calls":0,"tool_errors":0}},
                "raw":{{"origin":"o","sha256":"s","size":0}},"normalizer":{{"name":"t3code","version":4}},
                "mirror":true{of}}}"#
        ))
        .unwrap()
    }

    fn session(harness: &str, id: &str, extra: &str) -> SessionMeta {
        serde_json::from_str(&format!(
            r#"{{"harness":"{harness}","session_id":"{id}","models":[],"counts":{{"events":0,"user":0,"assistant":0,"tool_calls":0,"tool_errors":0}},
                "raw":{{"origin":"o","sha256":"s","size":0}},"normalizer":{{"name":"x","version":1}}{extra}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn what_a_mirror_needs_to_count_as_copied() {
        let m = mirror(r#","mirror_of":["codex"]"#);
        let mut codex = Originals::default();
        codex.harnesses.insert((None, Harness::Codex));
        assert!(m.copied_in(&codex));
        // Codex never captured: the mirror is the only copy.
        assert!(!m.copied_in(&Originals::default()));
        // Facts from before `mirror_of` was recorded count as copied.
        assert!(mirror("").copied_in(&Originals::default()));

        // Named originals must each be present; another session of the harness isn't enough.
        let exact = mirror(r#","mirror_of":["codex"],"mirror_refs":["codex/c-1"]"#);
        assert!(!exact.copied_in(&codex));
        codex.keys.insert("codex/c-1".into(), None);
        assert!(exact.copied_in(&codex));

        // A named original that's missing keeps the mirror, whatever the other runs say.
        let partial = mirror(
            r#","mirror_of":["codex"],"mirror_refs":["codex/c-2"],"mirror_unresolved":["codex"]"#,
        );
        assert!(!partial.copied_in(&codex));
        codex.keys.insert("codex/c-2".into(), None);
        assert!(partial.copied_in(&codex));
        // The unresolved run's harness must have sessions too.
        let keys_only = Originals {
            keys: codex.keys.clone(),
            ..Default::default()
        };
        assert!(!partial.copied_in(&keys_only));

        // T3 carried the session on after the original was last captured: keep the mirror.
        let mut later = mirror(r#","mirror_of":["codex"],"mirror_refs":["codex/c-1"]"#);
        later.ended_at = Some("2026-11-01T12:00:00Z".into());
        codex
            .keys
            .insert("codex/c-1".into(), Some("2026-11-01T11:00:00Z".into()));
        assert!(!later.copied_in(&codex));
        codex
            .keys
            .insert("codex/c-1".into(), Some("2026-11-01T11:59:30Z".into()));
        assert!(later.copied_in(&codex));
    }

    #[test]
    fn an_agents_session_is_started_from_the_top_of_its_chain() {
        // The owner's T3 thread delegates a task (its mirror names the Codex session that ran
        // it); that session's sub-agent has a parent of its own; a Claude sub-agent names its
        // parent by id.
        let owner = session("t3code", "t1", "");
        let task = session(
            "t3code",
            "t1-delegated-task-1",
            r#","parent_session":"t1","mirror":true,"mirror_of":["codex"],"mirror_refs":["codex/c-1"]"#,
        );
        let native = session("codex", "c-1", "");
        let sub = session("codex", "c-2", r#","parent_session":"c-1""#);
        let claude = session("claude-code", "s-1", "");
        let agent = session("claude-code", "agent-a1", r#","parent_session":"s-1""#);
        let orphan = session("claude-code", "agent-a2", r#","parent_session":"gone""#);
        let hermes = session("hermes", "db-h1", "");
        let hermes_child = session("hermes", "db-h2", r#","parent_session":"h1""#);
        // Hermes's file of h1 (a copy) names no parent; the database's record wins.
        let hermes_file = session("hermes", "h1", "");
        // A Hermes file of h2 that went on past the database's copy.
        let hermes_file_2 = session(
            "hermes",
            "h2",
            r#","mirror":true,"mirror_of":["hermes"],"mirror_refs":["hermes/db-h2"]"#,
        );
        // A chain whose top names a parent that isn't archived, and a key that isn't archived.
        let lost = session("codex", "c-3", r#","parent_session":"gone""#);
        let under_lost = session("codex", "c-4", r#","parent_session":"c-3""#);
        let wrong_harness = session("droid", "missing", "");
        let qualified = session("droid", "d-1", r#","parent_session":"claude-code/missing""#);
        // A delegated task whose parent isn't archived: its worker's sub-agent has no known top.
        let lost_task = session(
            "t3code",
            "t9-delegated-task-1",
            r#","parent_session":"t9","mirror":true,"mirror_of":["codex"],"mirror_refs":["codex/w-1"]"#,
        );
        let worker = session("codex", "w-1", "");
        let worker_sub = session("codex", "w-2", r#","parent_session":"w-1""#);
        let roots = Roots::of(&[
            owner,
            task,
            native,
            sub,
            claude,
            agent,
            orphan,
            hermes,
            hermes_child,
            hermes_file,
            hermes_file_2,
            lost,
            under_lost,
            wrong_harness,
            qualified,
            lost_task,
            worker,
            worker_sub,
        ]);
        for k in ["codex/c-4", "droid/d-1", "codex/w-2"] {
            assert_eq!(roots.of_session(k), None, "{k}");
        }
        assert_eq!(roots.of_session("hermes/db-h2"), Some("hermes/db-h1"));
        assert_eq!(roots.of_session("hermes/h2"), Some("hermes/db-h1"));
        assert_eq!(
            roots.of_session("t3code/t1-delegated-task-1"),
            Some("t3code/t1")
        );
        assert_eq!(roots.of_session("codex/c-1"), Some("t3code/t1"));
        assert_eq!(roots.of_session("codex/c-2"), Some("t3code/t1"));
        assert_eq!(
            roots.of_session("claude-code/agent-a1"),
            Some("claude-code/s-1")
        );
        // Sessions the owner started, and a parent that isn't archived, have none.
        for k in ["t3code/t1", "claude-code/s-1", "claude-code/agent-a2"] {
            assert_eq!(roots.of_session(k), None, "{k}");
        }
    }

    #[test]
    fn a_spokes_sessions_name_only_sessions_of_their_machine() {
        let on = |m: &str| format!(r#","machine":"{m}""#);
        // One session id on the hub and on a spoke; the spoke's sub-agent names it by id.
        let hub = session("claude-code", "s-1", "");
        let spoke = session("claude-code", "s-1", &on("nas"));
        let agent = session(
            "claude-code",
            "agent-a1",
            &format!(r#"{},"parent_session":"s-1""#, on("nas")),
        );
        assert_eq!(spoke.key(), "nas:claude-code/s-1");
        let roots = Roots::of(&[hub.clone(), spoke, agent]);
        assert_eq!(
            roots.of_session("nas:claude-code/agent-a1"),
            Some("nas:claude-code/s-1")
        );

        // A spoke's mirror of a harness only the hub has sessions of is the only copy.
        let thread = session(
            "t3code",
            "t1",
            &format!(r#"{},"mirror":true,"mirror_of":["claude-code"]"#, on("nas")),
        );
        assert!(!thread.is_duplicate(&Originals::of(&[hub])));
        // And a Cursor original on the spoke gets a Cursor turn's slack, as one here does.
        let mut cursor = session(
            "t3code",
            "t2",
            &format!(
                r#"{},"mirror":true,"mirror_of":["cursor"],"mirror_refs":["nas:cursor/agent-1"]"#,
                on("nas")
            ),
        );
        cursor.ended_at = Some("2026-11-01T12:00:00Z".into());
        let mut store = session("cursor", "agent-1", &on("nas"));
        store.ended_at = Some("2026-11-01T11:00:00Z".into());
        assert!(cursor.is_duplicate(&Originals::of(&[store])));
    }

    #[test]
    fn only_a_harnesss_own_sessions_prove_a_copy() {
        // A Cursor agent whose store is gone: its transcript and the T3 thread that ran it.
        let transcript = session(
            "cursor",
            "transcript-agent-1",
            r#","mirror":true,"mirror_of":["cursor"],"mirror_refs":["cursor/agent-1"]"#,
        );
        let thread = session(
            "t3code",
            "t1",
            r#","mirror":true,"mirror_of":["cursor"],"mirror_refs":["cursor/agent-1"]"#,
        );
        let o = Originals::of(&[thread.clone(), transcript.clone()]);
        assert!(!transcript.is_duplicate(&o) && !thread.is_duplicate(&o));
        // With the store archived, both are copies.
        let store = session("cursor", "agent-1", r#","ended_at":"2026-11-01T10:00:00Z""#);
        let o = Originals::of(&[thread.clone(), transcript.clone(), store.clone()]);
        assert!(transcript.is_duplicate(&o) && thread.is_duplicate(&o));
        assert!(!store.is_duplicate(&o));
        // A transcript with a turn the store doesn't have (same turn clock) is kept.
        let mut ahead = transcript.clone();
        ahead.ended_at = Some("2026-11-01T11:00:00Z".into());
        assert!(!ahead.is_duplicate(&o));
        // A T3 thread may end a long turn after the store's last turn began, but not days.
        let mut long_turn = thread.clone();
        long_turn.ended_at = Some("2026-11-01T11:30:00Z".into());
        assert!(long_turn.is_duplicate(&o));
        let mut continued = thread.clone();
        continued.ended_at = Some("2026-11-03T09:00:00Z".into());
        assert!(!continued.is_duplicate(&o));
    }
}
