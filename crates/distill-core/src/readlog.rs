//! The read log (`log/reads.jsonl`): one line for every call to the read path, refusals included,
//! saying when, which reader, which session and what it got, and one line for each step of a
//! reveal request. It never holds session text or a secret's value. It cannot be rebuilt from
//! the raw archive.

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::home::Home;
use crate::trust::Reader;
use crate::util::{append_jsonl, count, create_dir_private, read_jsonl, set_private};

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
    /// Distill itself read this many sessions, and this many of their events, to build the word
    /// index: one line per tick.
    Indexed { sessions: u64, events: u64 },
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
    find(
        home,
        &LogFilter {
            reader: who.map(str::to_string),
            ..LogFilter::default()
        },
        limit,
    )
}

/// Which lines of the read log. Every filter that is set must match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFilter {
    /// Only this reader's lines (see [`ReadEntry::is_by`]).
    pub reader: Option<String>,
    /// Lines written on or after this day (UTC).
    pub since: Option<NaiveDate>,
    /// Lines written on or before this day (UTC).
    pub until: Option<NaiveDate>,
}

impl LogFilter {
    fn matches(&self, e: &ReadEntry) -> bool {
        let day = || {
            DateTime::parse_from_rfc3339(&e.at)
                .ok()
                .map(|t| t.with_timezone(&Utc).date_naive())
        };
        self.reader.as_deref().is_none_or(|w| e.is_by(w))
            && self.since.is_none_or(|s| day().is_some_and(|d| d >= s))
            && self.until.is_none_or(|u| day().is_some_and(|d| d <= u))
    }
}

/// The lines of the read log that match `filter`, newest first, at most `limit` of them.
pub fn find(home: &Home, filter: &LogFilter, limit: usize) -> Result<Vec<ReadEntry>> {
    let mut lines: Vec<ReadEntry> = read_jsonl(&home.read_log_path())?;
    lines.reverse();
    Ok(lines
        .into_iter()
        .filter(|e| filter.matches(e))
        .take(limit)
        .collect())
}

impl ReadResult {
    /// What the reader got, as one line: `served 3 events`, `reveal <id> approved by the owner`.
    pub fn text(&self) -> String {
        match self {
            ReadResult::Served { events } => format!("served {}", count(*events, "event")),
            ReadResult::Listed { sessions } => format!("listed {}", count(*sessions, "session")),
            ReadResult::Indexed { sessions, events } => format!(
                "indexed {}, {}",
                count(*sessions, "session"),
                count(*events, "event")
            ),
            ReadResult::RevealRequested { request, secrets } => {
                let fps: Vec<String> = secrets.iter().map(|s| format!("#{s}")).collect();
                format!("reveal {request} requested: {}", fps.join(" "))
            }
            ReadResult::RevealApproved { request } => {
                format!("reveal {request} approved by the owner")
            }
            ReadResult::RevealDenied { request } => format!("reveal {request} denied by the owner"),
            ReadResult::RevealCollected { request, secrets } => {
                format!("reveal {request} collected: {}", count(*secrets, "value"))
            }
            ReadResult::RevealExpired { request } => format!("reveal {request} expired"),
            ReadResult::Findings {
                fingerprints,
                sessions,
            } => {
                let mut s = format!("listed {}", count(*fingerprints, "fingerprint"));
                if *sessions > 0 {
                    s += &format!(" in {}", count(*sessions, "session"));
                }
                s
            }
            ReadResult::Refused { reason } => format!("refused: {reason}"),
            ReadResult::Failed { error } => format!("failed: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(home: &Home, at: &str, reader: Reader, result: ReadResult) {
        append(
            home,
            &ReadEntry {
                at: at.into(),
                reader,
                session: Some("claude-code/s1".into()),
                result,
            },
        )
        .unwrap();
    }

    fn day(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_log_is_filtered_by_reader_and_by_the_day_each_line_was_written() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        let cloud = || Reader::model(Some("example-cloud"), Some("m"));
        line(
            &home,
            "2026-11-10T23:59:00Z",
            Reader::Owner,
            ReadResult::Served { events: 3 },
        );
        line(
            &home,
            "2026-11-11T08:00:00Z",
            cloud(),
            ReadResult::Served { events: 1 },
        );
        line(
            &home,
            "2026-11-12T08:00:00Z",
            cloud(),
            ReadResult::RevealApproved {
                request: "ab".into(),
            },
        );
        line(
            &home,
            "2026-11-13T00:00:00Z",
            Reader::Owner,
            ReadResult::Listed { sessions: 2 },
        );

        let all = find(&home, &LogFilter::default(), 100).unwrap();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].at, "2026-11-13T00:00:00Z", "newest first");

        let f = LogFilter {
            reader: Some("example-cloud".into()),
            ..LogFilter::default()
        };
        assert_eq!(find(&home, &f, 100).unwrap().len(), 2);

        let f = LogFilter {
            since: Some(day("2026-11-11")),
            until: Some(day("2026-11-12")),
            ..LogFilter::default()
        };
        let got: Vec<String> = find(&home, &f, 100)
            .unwrap()
            .into_iter()
            .map(|e| e.at)
            .collect();
        assert_eq!(got, ["2026-11-12T08:00:00Z", "2026-11-11T08:00:00Z"]);

        let f = LogFilter {
            reader: Some("owner".into()),
            since: Some(day("2026-11-11")),
            until: None,
        };
        assert_eq!(find(&home, &f, 100).unwrap().len(), 1);
        assert_eq!(find(&home, &LogFilter::default(), 1).unwrap().len(), 1);
    }

    #[test]
    fn each_result_reads_as_one_line() {
        let cases = [
            (ReadResult::Served { events: 1 }, "served 1 event"),
            (ReadResult::Listed { sessions: 2 }, "listed 2 sessions"),
            (
                ReadResult::Indexed {
                    sessions: 2,
                    events: 9,
                },
                "indexed 2 sessions, 9 events",
            ),
            (
                ReadResult::RevealRequested {
                    request: "ab".into(),
                    secrets: vec!["a1b2c3d4".into()],
                },
                "reveal ab requested: #a1b2c3d4",
            ),
            (
                ReadResult::RevealApproved {
                    request: "ab".into(),
                },
                "reveal ab approved by the owner",
            ),
            (
                ReadResult::RevealDenied {
                    request: "ab".into(),
                },
                "reveal ab denied by the owner",
            ),
            (
                ReadResult::RevealCollected {
                    request: "ab".into(),
                    secrets: 1,
                },
                "reveal ab collected: 1 value",
            ),
            (
                ReadResult::RevealExpired {
                    request: "ab".into(),
                },
                "reveal ab expired",
            ),
            (
                ReadResult::Findings {
                    fingerprints: 3,
                    sessions: 0,
                },
                "listed 3 fingerprints",
            ),
            (
                ReadResult::Findings {
                    fingerprints: 1,
                    sessions: 2,
                },
                "listed 1 fingerprint in 2 sessions",
            ),
            (
                ReadResult::Refused {
                    reason: "no".into(),
                },
                "refused: no",
            ),
            (
                ReadResult::Failed {
                    error: "boom".into(),
                },
                "failed: boom",
            ),
        ];
        for (r, want) in cases {
            assert_eq!(r.text(), want);
        }
    }
}
