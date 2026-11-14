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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitOutcome>,
}

impl SessionMeta {
    pub fn key(&self) -> String {
        format!("{}/{}", self.source.id(), self.session_id)
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
    pub mirror: bool,
}

impl Parsed {
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
