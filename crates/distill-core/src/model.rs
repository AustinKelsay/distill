//! The normalized shapes every source is turned into.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::source::SourceKind;

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

/// One step of a trajectory. Tool calls and results are kept in full, not only chat text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub kind: EventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Tool name, for tool calls (and results when the source records it).
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Producer {
    pub name: String,
    pub version: u32,
}

/// What git says happened in the session's repository while it ran.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GitOutcome {
    pub repo: String,
    /// The branch the session recorded. Commits are only attributed when it is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha_at_start: Option<String>,
    /// Commits on `branch` between the session's start and an hour after its end. Other work
    /// on the same branch in that window is counted too.
    pub commits: Vec<String>,
    /// Of those, how many were later reverted.
    pub reverted: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub source: SourceKind,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
    /// True for sessions Distill's own steward ran. Datasets leave them out.
    #[serde(default)]
    pub steward: bool,
    /// A copy of a session Distill also captures from the harness's own files (a T3 Code
    /// thread run on Codex, say). Searchable, but left out of datasets and steward cards.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror: bool,
    /// The sources holding the originals of a mirror. Empty on metas written before this was
    /// recorded, which count as mirrors of enabled sources.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirror_of: Vec<SourceKind>,
    /// The exact sessions a mirror copies (`source/session id`), when the harness named them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirror_refs: Vec<String>,
    /// Sources of a mirror's runs that name no original; those count as copied when the
    /// source has sessions at all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirror_unresolved: Vec<SourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitOutcome>,
}

impl SessionMeta {
    pub fn key(&self) -> String {
        format!("{}/{}", self.source.id(), self.session_id)
    }

    /// A mirror whose originals are in the archive and cover it.
    pub fn is_duplicate(&self, originals: &Originals) -> bool {
        self.mirror && self.copied_in(originals)
    }

    /// Whether `originals` hold what this mirror copies. With the originals named, each must be
    /// there; otherwise their sources must have sessions at all. A mirror of sessions that were
    /// never captured (deleted before Distill ran, say) is the only copy, so it counts as a
    /// session of its own.
    fn copied_in(&self, originals: &Originals) -> bool {
        // Runs whose original isn't named count as copied when their source has sessions.
        // Metas from before originals were recorded name none, so they check every source.
        let unresolved = if self.mirror_refs.is_empty() && self.mirror_unresolved.is_empty() {
            &self.mirror_of
        } else {
            &self.mirror_unresolved
        };
        // A named original must also reach (nearly) as far as the mirror: T3 can carry a
        // native session on after its source stopped being captured. Cursor's times mark only
        // when each turn began, so they can't say how far a session went.
        let covers = |k: &String| {
            originals.keys.get(k).is_some_and(|ended| {
                if k.starts_with("cursor/") {
                    return true;
                }
                match (
                    ended.as_deref().and_then(ts),
                    self.ended_at.as_deref().and_then(ts),
                ) {
                    (Some(orig), Some(mirror)) => orig + COVER_SLACK >= mirror,
                    _ => true,
                }
            })
        };
        self.mirror_refs.iter().all(covers)
            && unresolved.iter().all(|k| originals.sources.contains(k))
    }
}

/// How much later a mirror may end than its original and still count as a copy of it: T3
/// records a turn a little after the harness does.
const COVER_SLACK: chrono::Duration = chrono::Duration::minutes(15);

fn ts(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(s).ok()
}

/// The sessions in an index that are not mirrors, for [`SessionMeta::is_duplicate`]. Only a
/// harness's own sessions count: a mirror is never taken as proof for another mirror, since it
/// may hold less (or more) than the session it copies. When the original is gone, two copies of
/// it both stay.
#[derive(Debug, Default)]
pub struct Originals {
    pub sources: std::collections::HashSet<SourceKind>,
    /// Each original's key, with when it ended.
    pub keys: std::collections::HashMap<String, Option<String>>,
}

impl Originals {
    pub fn of(metas: &[SessionMeta]) -> Self {
        let mut o = Originals::default();
        for m in metas.iter().filter(|m| !m.mirror) {
            o.sources.insert(m.source);
            o.keys.insert(m.key(), m.ended_at.clone());
        }
        o
    }
}

/// What a parser returns before Distill adds raw references and ids.
#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub session_id: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub models: Vec<String>,
    pub harness_version: Option<String>,
    pub parent_session: Option<String>,
    pub events: Vec<Event>,
    /// For a copy of sessions captured natively elsewhere: the sources holding the originals.
    pub mirror_of: Vec<SourceKind>,
    /// And the originals themselves (`source/session id`), when known.
    pub mirror_refs: Vec<String>,
    /// Sources of runs whose original isn't known.
    pub mirror_unresolved: Vec<SourceKind>,
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
            r#"{{"source":"t3code","session_id":"t1","models":[],"counts":{{"events":0,"user":0,"assistant":0,"tool_calls":0,"tool_errors":0}},
                "raw":{{"origin":"o","sha256":"s","size":0}},"normalizer":{{"name":"t3code","version":4}},
                "mirror":true{of}}}"#
        ))
        .unwrap()
    }

    fn session(source: &str, id: &str, extra: &str) -> SessionMeta {
        serde_json::from_str(&format!(
            r#"{{"source":"{source}","session_id":"{id}","models":[],"counts":{{"events":0,"user":0,"assistant":0,"tool_calls":0,"tool_errors":0}},
                "raw":{{"origin":"o","sha256":"s","size":0}},"normalizer":{{"name":"x","version":1}}{extra}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn what_a_mirror_needs_to_count_as_copied() {
        let m = mirror(r#","mirror_of":["codex"]"#);
        let mut codex = Originals::default();
        codex.sources.insert(SourceKind::Codex);
        assert!(m.copied_in(&codex));
        // Codex never captured: the mirror is the only copy.
        assert!(!m.copied_in(&Originals::default()));
        // Metas from before `mirror_of` was recorded count as copied.
        assert!(mirror("").copied_in(&Originals::default()));

        // Named originals must each be present; another session of the source isn't enough.
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
        // The unresolved run's source must have sessions too.
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
        let store = session("cursor", "agent-1", "");
        let o = Originals::of(&[thread.clone(), transcript.clone(), store.clone()]);
        assert!(transcript.is_duplicate(&o) && thread.is_duplicate(&o));
        assert!(!store.is_duplicate(&o));
    }
}
