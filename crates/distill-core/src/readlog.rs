//! The read log (`log/reads.jsonl`): one line for every call to the read path, refusals included,
//! saying when, which reader, which session and what it got, and one line for each step of a
//! reveal request. It never holds session text or a secret's value. It cannot be rebuilt from
//! the raw archive.
//!
//! Each line also carries `prev`, the SHA-256 of the chained line before it (of nothing, for the
//! first), so a chained line changed or taken out shows as a break ([`verify`]; issue #161). A
//! line without `prev`, from an older build (one still running appends without the lock), is read
//! as before and is not in the chain. A process running as the owner can still rewrite the whole log (ADR 0005): a
//! backup run notices when the log no longer starts with what it stored before
//! ([`crate::backup::BackupReport::read_log_rewritten`]).

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::home::Home;
use crate::runtime::LocalRuntime;
use crate::trust::{Reader, Trust};
use crate::util::{count, create_dir_private, private_file, read_jsonl, set_private};

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
    /// DISTILL itself read this many sessions, and this many of their events, to build the word
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
    /// The tags in use: this many tags, counted on this many sessions' cards.
    Tags { tags: u64, sessions: u64 },
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

/// A line of the read log as written: the entry, and for a reader that reads as Ollama, where
/// Ollama served it from when it read ([`Trust::ollama_via`]). Lines from before #162 have none.
#[derive(Serialize, Deserialize)]
struct Logged {
    #[serde(flatten)]
    entry: ReadEntry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    via: Option<String>,
}

/// Append one line, chained to the line before it, with where Ollama served the reader from
/// when `runtime` is given. The read path serves nothing it could not log. Every process that
/// reads appends here, so the last line is read and the new one written under the `reads` lock.
pub(crate) fn append(home: &Home, runtime: Option<&LocalRuntime>, entry: &ReadEntry) -> Result<()> {
    let via = runtime
        .and_then(|rt| Trust::ollama_via(&entry.reader, rt))
        .map(str::to_string);
    let dir = home.log_dir();
    create_dir_private(&dir)?;
    set_private(&dir);
    let path = home.read_log_path();
    let _lock = home.lock("reads")?;
    let entry = serde_json::to_string(&Logged {
        entry: entry.clone(),
        via,
    })?;
    let Some(fields) = entry.strip_prefix('{') else {
        anyhow::bail!("a read log line is a JSON object");
    };
    let line = format!(
        "{{\"prev\":\"{}\",{fields}\n",
        sha256(&last_chained(&path)?)
    );
    let mut f = private_file().create(true).append(true).open(&path)?;
    f.write_all(line.as_bytes())?;
    f.sync_data()?;
    set_private(&path);
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

/// Whether a line of the read log carries `prev`.
fn chained(line: &[u8]) -> Option<bool> {
    let v: serde_json::Value = serde_json::from_slice(line).ok()?;
    Some(v.get("prev").is_some())
}

/// The last line of the file at `path` that carries `prev`, without its newline; empty when there
/// is none.
fn last_chained(path: &Path) -> Result<Vec<u8>> {
    let mut f = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let len = f.metadata()?.len();
    let mut want = 4096;
    loop {
        let from = len.saturating_sub(want);
        f.seek(SeekFrom::Start(from))?;
        let mut tail = Vec::new();
        f.read_to_end(&mut tail)?;
        let body = tail.strip_suffix(b"\n").unwrap_or(&tail);
        let mut lines: Vec<&[u8]> = body.split(|b| *b == b'\n').collect();
        // The first piece is a whole line only when the tail starts the file.
        if from > 0 {
            lines.remove(0);
        }
        if let Some(line) = lines.into_iter().rev().find(|l| chained(l) == Some(true)) {
            return Ok(line.to_vec());
        }
        if from == 0 {
            return Ok(Vec::new());
        }
        want *= 4;
    }
}

/// What [`verify`] found in the read log's chain.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Chain {
    pub lines: u64,
    /// The lines that carry `prev`.
    pub chained: u64,
    /// The first line whose `prev` is not the line before it, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broken: Option<Break>,
}

/// A line of the read log that doesn't follow the line before it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Break {
    /// Its place in the log, from 1.
    pub line: u64,
    /// When it says it was written, if it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// Walk the read log's chain from its first line, to the first break.
pub fn verify(home: &Home) -> Result<Chain> {
    let bytes = match fs::read(home.read_log_path()) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    // Only whole lines: a line being appended now is not yet one.
    let whole = bytes.iter().rposition(|b| *b == b'\n').unwrap_or(0);
    let body = &bytes[..whole];
    let mut chain = Chain::default();
    let mut before: &[u8] = &[];
    for line in body.split(|b| *b == b'\n').filter(|_| !body.is_empty()) {
        chain.lines += 1;
        let parsed: Option<serde_json::Value> = serde_json::from_slice(line).ok();
        let prev = parsed.as_ref().and_then(|v| v.get("prev"));
        if prev.is_some() {
            chain.chained += 1;
        }
        let follows = match (&parsed, prev) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(_), Some(p)) => p.as_str() == Some(sha256(before).as_str()),
        };
        if !follows {
            let at = parsed
                .as_ref()
                .and_then(|v| v["at"].as_str().map(str::to_string));
            chain.broken = Some(Break {
                line: chain.lines,
                at,
            });
            break;
        }
        if prev.is_some() {
            before = line;
        }
    }
    Ok(chain)
}

/// What the owner should know about edits to the read log, in a sentence: a break in its chain,
/// or a backup run that found it rewritten since the run before. None when neither is seen.
pub fn edited(home: &Home) -> Result<Option<String>> {
    said(home, &verify(home)?)
}

/// [`edited`], from a [`verify`] already made.
pub fn said(home: &Home, chain: &Chain) -> Result<Option<String>> {
    if let Some(b) = chain.broken.clone() {
        let when =
            b.at.map(|at| format!(", written {at},"))
                .unwrap_or_default();
        return Ok(Some(format!(
            "The read log has been edited: line {}{when} doesn't follow the chained line before it.",
            b.line
        )));
    }
    let backups: Vec<crate::backup::BackupReport> = read_jsonl(&home.backup_log_path())?;
    Ok(backups.iter().rev().find_map(|r| {
        let seen = r.read_log_rewritten.as_ref()?;
        Some(format!(
            "The read log has been rewritten: the backup to {} at {seen} found it no longer \
             started with what the run before had stored.",
            r.target
        ))
    }))
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

/// What left this machine over the last day and the last 7 days, by where it went (issue #162):
/// from the read log, each model at a provider other than Ollama on this machine that was served
/// something (a model Ollama serves from a cloud reads as `ollama-cloud`, as the trust check
/// places it), and from the backup log, each backup target. Counts only, never what was read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Left {
    /// Where text went, most in the last 7 days first.
    pub places: Vec<Place>,
    /// What stayed on this machine: reads by the owner, DISTILL itself and models in Ollama.
    pub here: Spans,
    /// The read log has been edited ([`edited`]), so these counts may be short.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub maybe_short: bool,
}

/// One place text went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    /// `provider/model` for a reader, the target's name for a backup.
    pub name: String,
    pub kind: PlaceKind,
    #[serde(flatten)]
    pub spans: Spans,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceKind {
    /// A model at a provider, reading through the read path.
    Reader,
    /// A backup target, which holds the backup set encrypted (ADR 0009).
    Backup,
}

/// The last day's counts and the last 7 days'.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Spans {
    pub day: Sent,
    pub week: Sent,
}

/// How much went somewhere in a span of time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sent {
    /// Reads that served something (text, a list, tags, fingerprints, revealed values, an index
    /// build), or whole backup runs.
    pub times: u64,
    /// The sessions whose text, or revealed values, were served.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub sessions: u64,
    /// Rows of lists and searches served, each a session's facts and its opening; a session
    /// listed twice counts twice.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub listed: u64,
    /// For a backup: the bytes stored at the target, encrypted, by the runs in the backup log,
    /// whole or not (a scheduled run stopped at its deadline is not logged).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub bytes: u64,
}

/// [`Left`] as of `now`. A read by a model in Ollama counts where its line says Ollama served it
/// from; a line from before that was logged is judged by `stored_here`, whether Ollama has the
/// model on this machine now (one it hasn't, it serves from a cloud).
pub fn left(home: &Home, now: DateTime<Utc>, stored_here: &dyn Fn(&str) -> bool) -> Result<Left> {
    use std::collections::{BTreeMap, BTreeSet};
    let (day, week) = (
        now - chrono::Duration::days(1),
        now - chrono::Duration::days(7),
    );
    let when = |at: &str| {
        DateTime::parse_from_rfc3339(at)
            .ok()
            .map(|t| t.with_timezone(&Utc))
            .filter(|t| *t >= week && *t <= now)
    };
    // Per place: the counts, and the sessions seen in each span.
    type Tally = (Spans, BTreeSet<String>, BTreeSet<String>);
    let mut places: BTreeMap<(String, bool), Tally> = BTreeMap::new();
    let mut here: Tally = Default::default();
    for Logged { entry: e, via } in read_jsonl::<Logged>(&home.read_log_path())? {
        let Some(t) = when(&e.at) else { continue };
        let mut listed = 0;
        let session = match &e.result {
            ReadResult::Served { .. } | ReadResult::RevealCollected { .. } => e.session.clone(),
            ReadResult::Listed { sessions } => {
                listed = *sessions;
                None
            }
            ReadResult::Tags { .. } | ReadResult::Findings { .. } | ReadResult::Indexed { .. } => {
                None
            }
            _ => continue,
        };
        let tally = match &e.reader {
            Reader::Model { provider, model } => {
                let mut p = provider
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                let model = model.as_deref().map(str::trim).filter(|m| !m.is_empty());
                if p == crate::trust::OLLAMA {
                    // Where Ollama served it from as logged; for an older line, a guess from
                    // what Ollama has on this machine now.
                    let cloud = match (via.as_deref(), model) {
                        (Some(v), _) => v == crate::trust::OLLAMA_CLOUD,
                        (None, Some(m)) => !stored_here(m),
                        (None, None) => false,
                    };
                    if cloud {
                        p = crate::trust::OLLAMA_CLOUD.into();
                    } else {
                        p.clear();
                    }
                }
                if p.is_empty() {
                    &mut here
                } else {
                    let name = model.map_or(p.clone(), |m| format!("{p}/{m}"));
                    places.entry((name, false)).or_default()
                }
            }
            _ => &mut here,
        };
        let recent = t >= day;
        tally.0.week.times += 1;
        tally.0.week.listed += listed;
        if recent {
            tally.0.day.times += 1;
            tally.0.day.listed += listed;
        }
        if let Some(s) = session {
            if recent {
                tally.2.insert(s.clone());
            }
            tally.1.insert(s);
        }
    }
    let backups: Vec<crate::backup::BackupReport> = read_jsonl(&home.backup_log_path())?;
    // A run that failed may still have stored objects: its bytes count, but not as a run.
    for r in backups.iter().filter(|r| r.ok || r.bytes_sent > 0) {
        let Some(t) = when(&r.at) else { continue };
        let tally = places.entry((r.target.clone(), true)).or_default();
        for (sent, inside) in [(&mut tally.0.week, true), (&mut tally.0.day, t >= day)] {
            if inside {
                sent.times += u64::from(r.ok);
                sent.bytes += r.bytes_sent;
            }
        }
    }
    let spans = |(mut s, week, day): Tally| {
        s.week.sessions = week.len() as u64;
        s.day.sessions = day.len() as u64;
        s
    };
    let mut places: Vec<Place> = places
        .into_iter()
        .map(|((name, backup), tally)| Place {
            name,
            kind: if backup {
                PlaceKind::Backup
            } else {
                PlaceKind::Reader
            },
            spans: spans(tally),
        })
        .collect();
    places.sort_by(|a, b| {
        b.spans
            .week
            .times
            .cmp(&a.spans.week.times)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(Left {
        places,
        here: spans(here),
        maybe_short: false,
    })
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
            ReadResult::Tags { tags, sessions } => format!(
                "listed {} on {}",
                count(*tags, "tag"),
                count(*sessions, "session")
            ),
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
            None,
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
    fn each_line_is_chained_to_the_one_before_and_an_edit_shows_as_a_break() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        let path = home.read_log_path();
        let served = |at: &str| {
            line(&home, at, Reader::Owner, ReadResult::Served { events: 1 });
        };
        let lines = || -> Vec<String> {
            fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect()
        };
        let put = |lines: &[String]| fs::write(&path, lines.join("\n") + "\n").unwrap();
        let broken_at = || verify(&home).unwrap().broken.map(|b| b.line);

        // Lines an older build wrote, without `prev`, start the chain.
        fs::create_dir_all(home.log_dir()).unwrap();
        let old =
            r#"{"at":"2026-11-01T00:00:00Z","reader":"owner","result":"listed","sessions":1}"#;
        fs::write(&path, format!("{old}\n{old}\n")).unwrap();
        for at in [
            "2026-11-02T00:00:00Z",
            "2026-11-03T00:00:00Z",
            "2026-11-04T00:00:00Z",
        ] {
            served(at);
        }
        let chain = verify(&home).unwrap();
        assert_eq!((chain.lines, chain.chained, chain.broken), (5, 3, None));
        assert!(lines()[2].starts_with(r#"{"prev":""#), "{}", lines()[2]);
        assert_eq!(find(&home, &LogFilter::default(), 10).unwrap().len(), 5);
        assert_eq!(edited(&home).unwrap(), None);
        let whole = lines();

        // A line changed, taken out or copied in shows at the line after it.
        let mut changed = whole.clone();
        changed[2] = changed[2].replace("\"events\":1", "\"events\":9");
        put(&changed);
        assert_eq!(broken_at(), Some(4));
        let mut gone = whole.clone();
        gone.remove(3);
        put(&gone);
        assert_eq!(broken_at(), Some(4));
        let mut more = whole.clone();
        more.insert(3, whole[2].clone());
        put(&more);
        assert_eq!(broken_at(), Some(4));
        let message = edited(&home).unwrap().unwrap();
        assert!(
            message.contains("line 4, written 2026-11-02T00:00:00Z,"),
            "{message}"
        );
        // So does a first line taken out of a chain that started on a new log.
        fs::remove_file(&path).unwrap();
        served("2026-11-05T00:00:00Z");
        served("2026-11-06T00:00:00Z");
        put(&lines()[1..]);
        assert_eq!(broken_at(), Some(1));

        // A line an older build still running appends (an MCP server started before an install)
        // is not in the chain: the next chained line names the chained line before it, so the
        // older line can't hide an edit to that one.
        put(&whole);
        fs::write(&path, format!("{}{old}\n", whole.join("\n") + "\n")).unwrap();
        served("2026-11-07T00:00:00Z");
        let chain = verify(&home).unwrap();
        assert_eq!((chain.lines, chain.chained, chain.broken), (7, 4, None));
        let mut behind = lines();
        behind[4] = behind[4].replace("\"events\":1", "\"events\":9");
        put(&behind);
        assert_eq!(broken_at(), Some(7));

        // A line still being appended is not a whole line yet, so not a break.
        put(&lines());
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(br#"{"prev":"ab"#).unwrap();
        assert_eq!(broken_at(), Some(7), "only the edit above");
        put(&whole);
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(br#"{"prev":"ab"#).unwrap();
        assert_eq!(
            verify(&home).unwrap(),
            Chain {
                lines: 5,
                chained: 3,
                broken: None
            }
        );
    }

    #[test]
    fn what_left_this_machine_is_counted_by_place_for_a_day_and_a_week() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        let now: DateTime<Utc> = "2026-11-20T12:00:00Z".parse().unwrap();
        let ago = |hours: i64| {
            (now - chrono::Duration::hours(hours))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        };
        // Ollama has `small:8b` on this machine and serves `big:cloud` from a cloud.
        let fake = crate::runtime::FakeRuntime::running()
            .with_model("small:8b", crate::runtime::Stored::Here)
            .with_model("big:cloud", crate::runtime::Stored::Cloud);
        let rt = fake.runtime();
        let put_by = |rt: Option<&LocalRuntime>,
                      hours: i64,
                      reader: Reader,
                      session: Option<&str>,
                      result: ReadResult| {
            append(
                &home,
                rt,
                &ReadEntry {
                    at: ago(hours),
                    reader,
                    session: session.map(str::to_string),
                    result,
                },
            )
            .unwrap();
        };
        let put =
            |hours, reader, session, result| put_by(Some(&rt), hours, reader, session, result);
        let cloud = || Reader::model(Some("Example-Cloud"), Some("m"));
        let served = || ReadResult::Served { events: 3 };
        put(2, cloud(), Some("claude-code/a"), served());
        put(3, cloud(), Some("claude-code/a"), served());
        put(50, cloud(), Some("codex/b"), served());
        put(1, cloud(), None, ReadResult::Listed { sessions: 5 });
        put(
            4,
            cloud(),
            Some("claude-code/a"),
            ReadResult::Refused {
                reason: "no".into(),
            },
        );
        put(200, cloud(), Some("codex/c"), served());
        put(
            5,
            Reader::model(Some("ollama"), Some("small:8b")),
            Some("codex/b"),
            served(),
        );
        // A model Ollama doesn't have here, it serves from a cloud.
        put(
            8,
            Reader::model(Some("ollama"), Some("big:cloud")),
            Some("codex/b"),
            served(),
        );
        // A line from before where Ollama served it from was logged.
        put_by(
            None,
            30,
            Reader::model(Some("ollama"), Some("old:7b")),
            Some("codex/b"),
            served(),
        );
        put(6, Reader::Owner, Some("codex/b"), served());
        put(
            7,
            Reader::Distill,
            None,
            ReadResult::Indexed {
                sessions: 9,
                events: 90,
            },
        );
        put(
            30,
            Reader::model(Some("example-agent"), None),
            Some("codex/b"),
            ReadResult::RevealCollected {
                request: "ab".into(),
                secrets: 1,
            },
        );
        for (hours, ok, bytes) in [(1, true, 100), (2, false, 7), (80, true, 50)] {
            crate::util::append_jsonl(
                &home.backup_log_path(),
                &crate::backup::BackupReport {
                    at: ago(hours),
                    target: "drive".into(),
                    ok,
                    bytes_sent: bytes,
                    ..Default::default()
                },
            )
            .unwrap();
        }

        let sent = |times, sessions, listed, bytes| Sent {
            times,
            sessions,
            listed,
            bytes,
        };
        // What Ollama has now is the other way round from when the reads were logged: the lines
        // that say where Ollama served them from go by that, an older line by what it has now.
        let got = left(&home, now, &|m| m == "big:cloud").unwrap();
        let places: Vec<(&str, PlaceKind, Sent, Sent)> = got
            .places
            .iter()
            .map(|p| {
                (
                    p.name.as_str(),
                    p.kind,
                    p.spans.day.clone(),
                    p.spans.week.clone(),
                )
            })
            .collect();
        let reader = PlaceKind::Reader;
        assert_eq!(
            places,
            [
                (
                    "example-cloud/m",
                    reader,
                    sent(3, 1, 5, 0),
                    sent(4, 2, 5, 0)
                ),
                // The failed run's bytes count, but not as a run.
                (
                    "drive",
                    PlaceKind::Backup,
                    sent(1, 0, 0, 107),
                    sent(2, 0, 0, 157)
                ),
                ("example-agent", reader, sent(0, 0, 0, 0), sent(1, 1, 0, 0)),
                (
                    "ollama-cloud/big:cloud",
                    reader,
                    sent(1, 1, 0, 0),
                    sent(1, 1, 0, 0)
                ),
                (
                    "ollama-cloud/old:7b",
                    reader,
                    sent(0, 0, 0, 0),
                    sent(1, 1, 0, 0)
                ),
            ]
        );
        assert_eq!(
            got.here,
            Spans {
                day: sent(3, 1, 0, 0),
                week: sent(3, 1, 0, 0)
            },
            "Ollama on this machine, the owner and DISTILL itself"
        );
        assert!(!got.maybe_short);
    }

    #[test]
    fn readers_appending_at_once_keep_the_chain_whole() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..25 {
                        line(
                            &home,
                            "2026-11-01T00:00:00Z",
                            Reader::Owner,
                            ReadResult::Served { events: 1 },
                        );
                    }
                });
            }
        });
        let chain = verify(&home).unwrap();
        assert_eq!((chain.lines, chain.chained, chain.broken), (200, 200, None));
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
