//! The read log (`log/reads.jsonl`): one line for every call to the read path, refusals included,
//! saying when, which reader, which session and what it got, and one line for each step of a
//! reveal request. It never holds session text or a secret's value. It cannot be rebuilt from
//! the raw archive.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::home::Home;
use crate::trust::Reader;
use crate::util::{append_jsonl, create_dir_private, read_jsonl, set_private};

/// One line of the read log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadEntry {
    pub at: String,
    #[serde(flatten)]
    pub reader: Reader,
    /// The session read: its key (`harness/id`), or what was asked for when no session matched.
    /// None for a call that lists sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(flatten)]
    pub result: ReadResult,
}

/// What the reader got.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ReadResult {
    /// Session text, this many events of it.
    Served { events: u64 },
    /// A list of sessions, this many of them, each with its facts and the opening of what the
    /// owner first wrote.
    Listed { sessions: u64 },
    /// A reveal request was filed for these secrets, by their short fingerprints (`crate::reveal`).
    RevealRequested {
        request: String,
        secrets: Vec<String>,
    },
    /// The owner approved the reader's reveal request, in the GUI.
    RevealApproved { request: String },
    /// The owner denied the reader's reveal request, in the GUI.
    RevealDenied { request: String },
    /// The reader collected the values of its approved request, this many of them. Never the
    /// values themselves.
    RevealCollected { request: String, secrets: u64 },
    /// The reader's request passed its 24 hours before it was answered or collected.
    RevealExpired { request: String },
    /// Findings by fingerprint: this many fingerprints, and the sessions listed for them (none
    /// for the rotation list, which gives counts). Never a value or session text.
    Findings {
        fingerprints: u64,
        #[serde(default, skip_serializing_if = "is_zero")]
        sessions: u64,
    },
    /// Nothing, and why.
    Refused { reason: String },
    /// Nothing: the read path failed before it could decide.
    Failed { error: String },
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl ReadEntry {
    /// Whether this line is by `who`: `owner`, `distill`, a provider's name, or `provider/model`.
    /// Names match whatever their case.
    pub fn is_by(&self, who: &str) -> bool {
        let who = who.trim().to_ascii_lowercase();
        match &self.reader {
            Reader::Owner => who == "owner",
            Reader::Distill => who == "distill",
            Reader::Model { provider, model } => {
                let provider = provider.as_deref().unwrap_or("").to_ascii_lowercase();
                match who.split_once('/') {
                    Some((p, m)) => p == provider && model.as_deref() == Some(m),
                    None => who == provider,
                }
            }
        }
    }
}

/// Append one line. The read path serves nothing it could not log.
pub(crate) fn append(home: &Home, entry: &ReadEntry) -> Result<()> {
    let dir = home.log_dir();
    create_dir_private(&dir)?;
    set_private(&dir);
    append_jsonl(&home.read_log_path(), entry)
}

/// The read log, newest first, only `who`'s lines when given (see [`ReadEntry::is_by`]), at
/// most `limit` of them.
pub fn list(home: &Home, who: Option<&str>, limit: usize) -> Result<Vec<ReadEntry>> {
    let mut lines: Vec<ReadEntry> = read_jsonl(&home.read_log_path())?;
    lines.reverse();
    Ok(lines
        .into_iter()
        .filter(|e| who.is_none_or(|w| e.is_by(w)))
        .take(limit)
        .collect())
}
