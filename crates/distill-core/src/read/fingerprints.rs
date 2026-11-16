//! Findings by fingerprint through the read path: what sensitive values the archive holds,
//! without any of them (issue #78). [`fingerprints`] groups the stored findings by fingerprint,
//! secrets by default, which is the rotation list; [`where_found`] lists every session a
//! fingerprint, or its short form, appears in.
//!
//! Both answer from the stored findings and the session list alone: no session text is read and
//! nothing is scanned. Mirrors of sessions in the archive are left out, as the original counts. Sessions whose findings are missing or out of date are counted as pending,
//! for the next scan. Each call appends one line to the read log.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use super::{PAGE_MAX, Page};
use crate::detect::{Class, Kind};
use crate::findings::{self, Findings, Fingerprint, FingerprintKey};
use crate::harness::Harness;
use crate::home::Home;
use crate::model::{Originals, SessionMeta};
use crate::normalize;
use crate::readlog::{self, ReadEntry, ReadResult};
use crate::runtime::LocalRuntime;
use crate::trust::{Access, Reader, Trust};
use crate::util::{count, now_rfc3339};

/// Where one fingerprint appears.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Appearances {
    pub fingerprint: Fingerprint,
    /// What it was found as, in kind order.
    pub kinds: Vec<Kind>,
    /// Every session it appears in, newest first.
    pub sessions: Vec<Appearance>,
}

/// One session a fingerprint appears in: its facts, and the events it is in. No text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Appearance {
    /// The session's key, `harness/id`.
    pub session: String,
    pub harness: Harness,
    pub project: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    /// The events it is in, by sequence number.
    pub events: Vec<u64>,
}

/// What a lookup found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lookup {
    /// What was asked for.
    pub asked: String,
    /// Every stored fingerprint that starts with it, in fingerprint order.
    pub found: Vec<Appearances>,
    /// Sessions whose findings are missing or out of date, so not searched until they are scanned.
    pub pending: u64,
}

/// Which findings to group. Every filter that is set must match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindingFilter {
    pub kinds: Kinds,
    pub harness: Option<Harness>,
    /// Sessions that started on or after this day (UTC).
    pub since: Option<NaiveDate>,
    /// Sessions that started on or before this day (UTC).
    pub until: Option<NaiveDate>,
}

/// Which kinds of finding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Kinds {
    /// Every kind of secret: the rotation list.
    #[default]
    Secrets,
    PersonalDetails,
    All,
    Only(Kind),
}

/// How to sort the fingerprints.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Order {
    /// Most sessions first.
    #[default]
    Spread,
    /// Most recently seen first.
    Recent,
}

/// What a reader gets.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer<T> {
    Served(T),
    /// Nothing, and why.
    Refused(String),
}

/// One fingerprint and how widely it appears.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Spread {
    pub fingerprint: Fingerprint,
    /// What it was found as, in kind order. Usually one.
    pub kinds: Vec<Kind>,
    /// How many sessions it appears in.
    pub sessions: u64,
    pub harnesses: Vec<Harness>,
    /// When the earliest of those sessions started, and the latest ended.
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
}

/// A page of fingerprints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fingerprints {
    pub rows: Vec<Spread>,
    /// How many fingerprints match in all.
    pub total: u64,
    /// Where the next page starts, when there is one.
    pub next: Option<usize>,
    /// Sessions whose findings are missing or out of date, so not counted until they are scanned.
    pub pending: u64,
}

/// The fingerprints in the archive that match `filter`, sorted by `order`, the rows in `page`.
/// A reader that may not read is refused. Every call appends one line to the read log.
pub fn fingerprints(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    filter: &FindingFilter,
    order: Order,
    page: Page,
) -> Result<Answer<Fingerprints>> {
    let answer = gate(home, runtime, reader, || {
        let stored = stored(home)?;
        let mut groups: BTreeMap<Fingerprint, Group> = BTreeMap::new();
        for (facts, findings) in stored.sessions.iter().filter(|(m, _)| filter.matches(m)) {
            for f in findings
                .findings
                .iter()
                .filter(|f| filter.kinds.admit(f.kind))
            {
                let g = groups.entry(f.fingerprint.clone()).or_default();
                g.kinds.insert(f.kind);
                g.sessions.insert(facts.key());
                g.harnesses.insert(facts.harness);
                let (start, end) = times(facts);
                g.first_seen = earliest(g.first_seen, start);
                g.last_seen = latest(g.last_seen, end);
            }
        }
        let mut rows: Vec<Spread> = groups
            .into_iter()
            .map(|(fingerprint, g)| Spread {
                fingerprint,
                kinds: g.kinds.into_iter().collect(),
                sessions: g.sessions.len() as u64,
                harnesses: g.harnesses.into_iter().collect(),
                first_seen: g.first_seen,
                last_seen: g.last_seen,
            })
            .collect();
        let recency = |s: &Spread| std::cmp::Reverse(s.last_seen);
        match order {
            Order::Spread => rows.sort_by(|a, b| {
                b.sessions
                    .cmp(&a.sessions)
                    .then(recency(a).cmp(&recency(b)))
                    .then(a.fingerprint.cmp(&b.fingerprint))
            }),
            Order::Recent => rows.sort_by(|a, b| {
                recency(a)
                    .cmp(&recency(b))
                    .then(b.sessions.cmp(&a.sessions))
                    .then(a.fingerprint.cmp(&b.fingerprint))
            }),
        }
        let total = rows.len() as u64;
        let rows: Vec<Spread> = rows
            .into_iter()
            .skip(page.offset)
            .take(page.limit.min(PAGE_MAX))
            .collect();
        let end = page.offset + rows.len();
        Ok(Fingerprints {
            next: ((end as u64) < total && !rows.is_empty()).then_some(end),
            rows,
            total,
            pending: stored.pending,
        })
    });
    log(home, reader, &answer, |f| (f.rows.len() as u64, 0))?;
    answer
}

impl Fingerprints {
    /// The page as text, for a model or the command line: a line about the page, then a line per
    /// fingerprint with its short form.
    pub fn text(&self) -> String {
        let mut t = if self.rows.is_empty() {
            if self.total == 0 {
                "No fingerprints match.\n".to_string()
            } else {
                format!(
                    "No fingerprints on this page: {} match in all, the first at offset 0.\n",
                    self.total
                )
            }
        } else {
            let first = self.next.unwrap_or(self.total as usize) - self.rows.len() + 1;
            let mut head = format!(
                "{} match{}; {}–{} here.",
                count(self.total, "fingerprint"),
                if self.total == 1 { "es" } else { "" },
                first,
                first + self.rows.len() - 1
            );
            match self.next {
                Some(next) => head += &format!(" More: the next page starts at offset {next}."),
                None => head += " This is the last page.",
            }
            head + "\n"
        };
        for r in &self.rows {
            t += &format!(
                "\n#{}  {}  {}  {}",
                r.fingerprint.short(),
                kind_names(&r.kinds),
                count(r.sessions, "session"),
                r.harnesses
                    .iter()
                    .map(|h| h.id())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if let Some(at) = r.first_seen {
                t += &format!("  first seen {}", at.format("%Y-%m-%d"));
            }
            if let Some(at) = r.last_seen {
                t += &format!("  last seen {}", at.format("%Y-%m-%d"));
            }
        }
        if !self.rows.is_empty() {
            t.push('\n');
        }
        t + &pending_note(self.pending)
    }
}

impl Lookup {
    /// What was found as text, for a model or the command line: each fingerprint by its short
    /// form, then a line per session it appears in.
    pub fn text(&self) -> String {
        let mut t = String::new();
        if self.found.is_empty() {
            t += &format!("No fingerprint starts with {:?}.\n", self.asked);
        } else if self.found.len() > 1 {
            t += &format!(
                "{} start with {:?}.\n",
                count(self.found.len() as u64, "fingerprint"),
                self.asked
            );
        }
        for f in &self.found {
            t += &format!(
                "\n#{} ({}) is in {}:\n",
                f.fingerprint.short(),
                kind_names(&f.kinds),
                count(f.sessions.len() as u64, "session")
            );
            for s in &f.sessions {
                t += &format!("  {} · {}", s.session, s.harness.label());
                if let Some(p) = &s.project {
                    t += &format!(" · project {p}");
                }
                if let Some(at) = s.started_at {
                    t += &format!(" · started {}", at.format("%Y-%m-%d %H:%M UTC"));
                }
                let events: Vec<String> = s.events.iter().map(u64::to_string).collect();
                t += &format!(" · events {}\n", events.join(", "));
            }
        }
        t + &pending_note(self.pending)
    }
}

fn kind_names(kinds: &[Kind]) -> String {
    kinds
        .iter()
        .map(|k| k.name())
        .collect::<Vec<_>>()
        .join(", ")
}

/// What is said about sessions not yet counted, if any.
fn pending_note(pending: u64) -> String {
    if pending == 0 {
        return String::new();
    }
    format!(
        "\n{} not scanned, or scanned by an older detector, so not counted here; `distill scan` \
         or the next tick scans {}.\n",
        count(pending, "session"),
        if pending == 1 { "it" } else { "them" }
    )
}

/// Every session each fingerprint that starts with `asked` (a fingerprint, or its short form, with
/// or without a placeholder's `#`, in either case) appears in. Two values can share a short
/// form, so a lookup can find several. A reader that may not read is refused. Every call appends
/// one line to the read log.
pub fn where_found(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    asked: &str,
) -> Result<Answer<Lookup>> {
    let answer = gate(home, runtime, reader, || {
        let stored = stored(home)?;
        let prefix = asked.trim().trim_start_matches('#');
        let mut found: BTreeMap<Fingerprint, (BTreeSet<Kind>, Vec<Appearance>)> = BTreeMap::new();
        for (facts, findings) in &stored.sessions {
            let mut here: BTreeMap<&Fingerprint, BTreeSet<u64>> = BTreeMap::new();
            for f in findings
                .findings
                .iter()
                .filter(|f| f.fingerprint.matches(prefix))
            {
                here.entry(&f.fingerprint).or_default().insert(f.event);
                found
                    .entry(f.fingerprint.clone())
                    .or_default()
                    .0
                    .insert(f.kind);
            }
            let (started_at, ended_at) = times(facts);
            for (fp, events) in here {
                found
                    .get_mut(fp)
                    .expect("entered above")
                    .1
                    .push(Appearance {
                        session: facts.key(),
                        harness: facts.harness,
                        project: facts.cwd.clone(),
                        started_at,
                        ended_at,
                        events: events.into_iter().collect(),
                    });
            }
        }
        Ok(Lookup {
            asked: asked.trim().to_string(),
            found: found
                .into_iter()
                .map(|(fingerprint, (kinds, mut sessions))| {
                    sessions.sort_by(|a, b| {
                        b.started_at
                            .cmp(&a.started_at)
                            .then(a.session.cmp(&b.session))
                    });
                    Appearances {
                        fingerprint,
                        kinds: kinds.into_iter().collect(),
                        sessions,
                    }
                })
                .collect(),
            pending: stored.pending,
        })
    });
    log(home, reader, &answer, |l| {
        (
            l.found.len() as u64,
            l.found.iter().map(|f| f.sessions.len() as u64).sum(),
        )
    })?;
    answer
}

/// One fingerprint's findings as they are gathered.
#[derive(Default)]
struct Group {
    kinds: BTreeSet<Kind>,
    sessions: BTreeSet<String>,
    harnesses: BTreeSet<Harness>,
    first_seen: Option<DateTime<Utc>>,
    last_seen: Option<DateTime<Utc>>,
}

/// The current findings of every session that has them, and how many sessions do not. Mirrors
/// of sessions in the archive are left out: their original already counts.
struct Stored {
    sessions: Vec<(SessionMeta, Findings)>,
    pending: u64,
}

/// Each session's stored findings, when they are current. Reads the session list and the findings
/// files, never events, and scans nothing. With no fingerprint key yet, nothing is current.
fn stored(home: &Home) -> Result<Stored> {
    let list = normalize::read_session_list(home)?;
    let originals = Originals::of(&list);
    let key = FingerprintKey::existing(home).ok().flatten();
    let mut out = Stored {
        sessions: Vec::new(),
        pending: 0,
    };
    for facts in list.into_iter().filter(|m| !m.is_duplicate(&originals)) {
        match (&key, findings::load(home, &facts)) {
            (Some(k), Some(f)) if f.is_current(&facts, k) => out.sessions.push((facts, f)),
            _ => out.pending += 1,
        }
    }
    Ok(out)
}

/// Run `serve` if `reader` may read; otherwise refuse. With no config, or one that can't be read,
/// only the built-in providers count: that trusts less, never more.
fn gate<T>(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    serve: impl FnOnce() -> Result<T>,
) -> Result<Answer<T>> {
    let trust = Trust::new(&home.load_config().unwrap_or_default());
    if let Access::Refused(why) = trust.check(reader, runtime) {
        return Ok(Answer::Refused(why));
    }
    serve().map(Answer::Served)
}

/// Append the call's line to the read log: `counts` gives the fingerprints and sessions served.
fn log<T>(
    home: &Home,
    reader: &Reader,
    answer: &Result<Answer<T>>,
    counts: impl Fn(&T) -> (u64, u64),
) -> Result<()> {
    let result = match answer {
        Ok(Answer::Served(t)) => {
            let (fingerprints, sessions) = counts(t);
            ReadResult::Findings {
                fingerprints,
                sessions,
            }
        }
        Ok(Answer::Refused(reason)) => ReadResult::Refused {
            reason: reason.clone(),
        },
        Err(e) => ReadResult::Failed {
            error: format!("{e:#}"),
        },
    };
    readlog::append(
        home,
        &ReadEntry {
            at: now_rfc3339(),
            reader: reader.clone(),
            session: None,
            result,
        },
    )
}

/// When a session started and ended, each falling back to the other.
fn times(m: &SessionMeta) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    let parse = |t: &Option<String>| {
        t.as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
    };
    let (start, end) = (parse(&m.started_at), parse(&m.ended_at));
    (start.or(end), end.or(start))
}

fn earliest(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn latest(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    a.max(b)
}

impl FindingFilter {
    fn matches(&self, m: &SessionMeta) -> bool {
        let day = times(m).0.map(|t| t.date_naive());
        self.harness.is_none_or(|h| m.harness == h)
            && self.since.is_none_or(|s| day.is_some_and(|d| d >= s))
            && self.until.is_none_or(|u| day.is_some_and(|d| d <= u))
    }
}

impl Kinds {
    /// `secrets`, `personal`, `all`, or one kind's name as findings and placeholders write it
    /// (`github_token`).
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim().to_ascii_lowercase().replace('-', "_");
        Ok(match s.as_str() {
            "secrets" | "secret" => Kinds::Secrets,
            "personal" | "personal_details" | "personal_detail" => Kinds::PersonalDetails,
            "all" => Kinds::All,
            _ => match serde_json::from_value(serde_json::Value::String(s.clone())) {
                Ok(kind) => Kinds::Only(kind),
                Err(_) => anyhow::bail!(
                    "unknown kind {s:?}: secrets, personal, all, or one kind's name as a \
                     placeholder writes it, such as anthropic_key"
                ),
            },
        })
    }

    fn admit(self, kind: Kind) -> bool {
        match self {
            Kinds::Secrets => kind.class() == Class::Secret,
            Kinds::PersonalDetails => kind.class() == Class::PersonalDetail,
            Kinds::All => true,
            Kinds::Only(k) => k == kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Counts, Event, EventKind, Producer, RawRef};
    use crate::runtime::{FakeRuntime, Stored};

    /// Fake secrets, built at run time so no secret scanner mistakes this file for a leak.
    fn fake(n: u32) -> String {
        ["sk-", "ant-", "api03-", &format!("Fq{n}").repeat(9)].concat()
    }

    /// A session as the parse step would leave it: `said` by the owner, at `started`.
    struct Session {
        harness: Harness,
        id: &'static str,
        started: &'static str,
        ended: &'static str,
        said: Vec<String>,
        /// The key of the session this one mirrors.
        mirror_of: Option<&'static str>,
    }

    fn session(
        harness: Harness,
        id: &'static str,
        started: &'static str,
        said: &[&str],
    ) -> Session {
        Session {
            harness,
            id,
            started,
            ended: started,
            said: said.iter().map(|s| s.to_string()).collect(),
            mirror_of: None,
        }
    }

    /// A home with `sessions` written in and scanned.
    fn home_with(sessions: &[Session]) -> (tempfile::TempDir, Home) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        for s in sessions {
            let mut events: Vec<Event> = s
                .said
                .iter()
                .map(|t| Event::text(EventKind::User, t.clone()))
                .collect();
            for (i, e) in events.iter_mut().enumerate() {
                e.seq = i as u64;
            }
            let facts = SessionMeta {
                harness: s.harness,
                session_id: s.id.into(),
                cwd: Some("/work/app".into()),
                git_branch: None,
                models: vec!["m".into()],
                harness_version: None,
                started_at: Some(s.started.into()),
                ended_at: Some(s.ended.into()),
                parent_session: None,
                counts: Counts::of(&events),
                raw: RawRef {
                    origin: format!("/s/{}.jsonl", s.id),
                    sha256: "0".repeat(64),
                    size: 1,
                },
                normalizer: Producer {
                    name: s.harness.id().into(),
                    version: 1,
                },
                steward: false,
                mirror: s.mirror_of.is_some(),
                mirror_of: s
                    .mirror_of
                    .map(|k| Harness::parse(k.split('/').next().unwrap()).unwrap())
                    .into_iter()
                    .collect(),
                mirror_refs: s.mirror_of.map(str::to_string).into_iter().collect(),
                mirror_unresolved: Vec::new(),
            };
            std::fs::create_dir_all(home.session_dir(s.harness, s.id)).unwrap();
            std::fs::write(
                home.meta_path(s.harness, s.id),
                serde_json::to_string(&facts).unwrap(),
            )
            .unwrap();
            let lines: Vec<String> = events
                .iter()
                .map(|e| serde_json::to_string(e).unwrap())
                .collect();
            std::fs::write(home.events_path(s.harness, s.id), lines.join("\n") + "\n").unwrap();
        }
        crate::normalize::rebuild_session_list(&home).unwrap();
        crate::findings::scan_all(&home).unwrap();
        (dir, home)
    }

    fn local() -> LocalRuntime {
        FakeRuntime::stopped()
            .with_model("small:8b", Stored::Here)
            .runtime()
    }

    fn owner_list(home: &Home, filter: &FindingFilter, order: Order) -> Fingerprints {
        match fingerprints(
            home,
            &local(),
            &Reader::Owner,
            filter,
            order,
            Page::default(),
        )
        .unwrap()
        {
            Answer::Served(f) => f,
            Answer::Refused(why) => panic!("refused: {why}"),
        }
    }

    fn at(t: &str) -> Option<DateTime<Utc>> {
        Some(t.parse().unwrap())
    }

    #[test]
    fn secrets_are_grouped_by_fingerprint_with_their_spread() {
        let (a, b) = (fake(1), fake(2));
        let (_d, home) = home_with(&[
            session(
                Harness::ClaudeCode,
                "s1",
                "2026-11-01T09:00:00Z",
                &[&format!("use {a}"), &format!("and {a} again")],
            ),
            Session {
                ended: "2026-11-05T10:00:00Z",
                ..session(
                    Harness::Codex,
                    "s2",
                    "2026-11-05T09:00:00Z",
                    &[&format!("try {a}")],
                )
            },
            session(
                Harness::Codex,
                "s3",
                "2026-11-03T09:00:00Z",
                &[&format!("then {b}")],
            ),
            session(
                Harness::Codex,
                "s4",
                "2026-11-04T09:00:00Z",
                &["nothing here"],
            ),
        ]);
        let list = owner_list(&home, &FindingFilter::default(), Order::Spread);
        assert_eq!((list.total, list.next, list.pending), (2, None, 0));
        let [first, second] = &list.rows[..] else {
            panic!("{list:?}")
        };
        assert_eq!(first.kinds, [Kind::AnthropicKey]);
        assert_eq!(first.sessions, 2);
        assert_eq!(first.harnesses, [Harness::ClaudeCode, Harness::Codex]);
        assert_eq!(first.first_seen, at("2026-11-01T09:00:00Z"));
        assert_eq!(first.last_seen, at("2026-11-05T10:00:00Z"));
        assert_eq!(second.sessions, 1);
        assert_eq!(second.harnesses, [Harness::Codex]);
        assert_ne!(first.fingerprint, second.fingerprint);
    }

    fn github(n: u32) -> String {
        ["gh", "p_", &format!("Tk{n}").repeat(12)].concat()
    }

    fn kinds(list: &Fingerprints) -> Vec<Vec<Kind>> {
        list.rows.iter().map(|r| r.kinds.clone()).collect()
    }

    #[test]
    fn a_kind_filter_selects_one_kind_personal_details_or_everything() {
        let (_d, home) = home_with(&[session(
            Harness::ClaudeCode,
            "s1",
            "2026-11-01T09:00:00Z",
            &[&format!("{} and {}", fake(1), github(1))],
        )]);
        let with = |kinds| {
            owner_list(
                &home,
                &FindingFilter {
                    kinds,
                    ..Default::default()
                },
                Order::Spread,
            )
        };
        assert_eq!(
            kinds(&with(Kinds::Only(Kind::GithubToken))),
            [[Kind::GithubToken]]
        );
        assert_eq!(with(Kinds::Secrets).total, 2);
        assert_eq!(with(Kinds::All).total, 2);
        assert_eq!(with(Kinds::PersonalDetails).total, 0);
    }

    #[test]
    fn a_kind_filter_is_parsed_from_a_class_or_a_kind_name() {
        assert_eq!(Kinds::parse("secrets").unwrap(), Kinds::Secrets);
        assert_eq!(Kinds::parse("personal").unwrap(), Kinds::PersonalDetails);
        assert_eq!(Kinds::parse("all").unwrap(), Kinds::All);
        assert_eq!(
            Kinds::parse("github_token").unwrap(),
            Kinds::Only(Kind::GithubToken)
        );
        let err = Kinds::parse("tea_leaves").unwrap_err().to_string();
        assert!(
            err.contains("secrets") && err.contains("tea_leaves"),
            "{err}"
        );
    }

    #[test]
    fn it_filters_by_harness_and_start_day_and_sorts_by_spread_or_recency() {
        let (a, b, c) = (fake(1), fake(2), fake(3));
        let (_d, home) = home_with(&[
            session(Harness::ClaudeCode, "s1", "2026-11-01T09:00:00Z", &[&a]),
            session(Harness::ClaudeCode, "s2", "2026-11-02T09:00:00Z", &[&a]),
            session(Harness::ClaudeCode, "s3", "2026-11-09T09:00:00Z", &[&b]),
            session(Harness::Codex, "s4", "2026-11-05T09:00:00Z", &[&c]),
        ]);
        let seen = |list: &Fingerprints| -> Vec<(u64, Option<DateTime<Utc>>)> {
            list.rows
                .iter()
                .map(|r| (r.sessions, r.last_seen))
                .collect()
        };
        let all = FindingFilter::default();
        assert_eq!(
            seen(&owner_list(&home, &all, Order::Spread)),
            [
                (2, at("2026-11-02T09:00:00Z")),
                (1, at("2026-11-09T09:00:00Z")),
                (1, at("2026-11-05T09:00:00Z")),
            ]
        );
        assert_eq!(
            seen(&owner_list(&home, &all, Order::Recent)),
            [
                (1, at("2026-11-09T09:00:00Z")),
                (1, at("2026-11-05T09:00:00Z")),
                (2, at("2026-11-02T09:00:00Z")),
            ]
        );
        let codex = FindingFilter {
            harness: Some(Harness::Codex),
            ..Default::default()
        };
        assert_eq!(
            seen(&owner_list(&home, &codex, Order::Spread)),
            [(1, at("2026-11-05T09:00:00Z"))]
        );
        let early = FindingFilter {
            since: Some(NaiveDate::from_ymd_opt(2026, 11, 2).unwrap()),
            until: Some(NaiveDate::from_ymd_opt(2026, 11, 5).unwrap()),
            ..Default::default()
        };
        assert_eq!(
            seen(&owner_list(&home, &early, Order::Spread)),
            [
                (1, at("2026-11-05T09:00:00Z")),
                (1, at("2026-11-02T09:00:00Z")),
            ]
        );
    }

    fn owner_lookup(home: &Home, asked: &str) -> Lookup {
        match where_found(home, &local(), &Reader::Owner, asked).unwrap() {
            Answer::Served(l) => l,
            Answer::Refused(why) => panic!("refused: {why}"),
        }
    }

    #[test]
    fn a_fingerprint_or_its_short_form_lists_every_session_it_appears_in() {
        let (a, b) = (fake(1), fake(2));
        let (_d, home) = home_with(&[
            session(
                Harness::ClaudeCode,
                "s1",
                "2026-11-01T09:00:00Z",
                &[&format!("use {a}"), "nothing", &format!("again {a}")],
            ),
            session(
                Harness::Codex,
                "s2",
                "2026-11-05T09:00:00Z",
                &[&format!("try {a}")],
            ),
            session(
                Harness::Codex,
                "s3",
                "2026-11-03T09:00:00Z",
                &[&format!("then {b}")],
            ),
        ]);
        let list = owner_list(&home, &FindingFilter::default(), Order::Spread);
        let widest = list.rows[0].fingerprint.clone();
        for asked in [
            widest.as_str().to_string(),
            widest.short().to_string(),
            format!("#{}", widest.short().to_ascii_uppercase()),
        ] {
            let l = owner_lookup(&home, &asked);
            let [found] = &l.found[..] else {
                panic!("{asked}: {l:?}")
            };
            assert_eq!(found.fingerprint, widest);
            assert_eq!(found.kinds, [Kind::AnthropicKey]);
            let where_: Vec<(&str, Harness, &[u64])> = found
                .sessions
                .iter()
                .map(|s| (s.session.as_str(), s.harness, &s.events[..]))
                .collect();
            assert_eq!(
                where_,
                [
                    ("codex/s2", Harness::Codex, &[0][..]),
                    ("claude-code/s1", Harness::ClaudeCode, &[0, 2][..]),
                ]
            );
            assert_eq!(found.sessions[0].project.as_deref(), Some("/work/app"));
            assert_eq!(found.sessions[0].started_at, at("2026-11-05T09:00:00Z"));
        }
        assert!(owner_lookup(&home, "zz").found.is_empty());
        assert!(owner_lookup(&home, "").found.is_empty());
    }

    #[test]
    fn a_short_form_that_several_fingerprints_start_lists_them_all() {
        // Seventeen values among sixteen hex digits: two fingerprints share a first digit.
        let values: Vec<String> = (1..=17).map(fake).collect();
        let ids: Vec<String> = (1..=17).map(|n| format!("s{n}")).collect();
        let sessions: Vec<Session> = values
            .iter()
            .zip(&ids)
            .map(|(v, id)| Session {
                id: Box::leak(id.clone().into_boxed_str()),
                ..session(
                    Harness::ClaudeCode,
                    "x",
                    "2026-11-01T09:00:00Z",
                    &[v.as_str()],
                )
            })
            .collect();
        let (_d, home) = home_with(&sessions);
        let list = owner_list(&home, &FindingFilter::default(), Order::Spread);
        let mut firsts = std::collections::BTreeMap::<char, Vec<Fingerprint>>::new();
        for r in list.rows {
            let c = r.fingerprint.as_str().chars().next().unwrap();
            firsts.entry(c).or_default().push(r.fingerprint);
        }
        let (digit, shared) = firsts.into_iter().find(|(_, f)| f.len() > 1).unwrap();
        let l = owner_lookup(&home, &digit.to_string());
        let found: Vec<Fingerprint> = l.found.iter().map(|f| f.fingerprint.clone()).collect();
        let mut shared = shared;
        shared.sort();
        assert_eq!(found, shared);
        assert!(l.found.iter().all(|f| f.sessions.len() == 1));
    }

    #[test]
    fn a_reader_that_may_not_read_is_refused_and_every_call_is_in_the_read_log() {
        let (_d, home) = home_with(&[session(
            Harness::ClaudeCode,
            "s1",
            "2026-11-01T09:00:00Z",
            &[&fake(1)],
        )]);
        let (filter, page) = (FindingFilter::default(), Page::default());
        let unknown = Reader::model(Some("unheard-of"), None);
        let refused =
            fingerprints(&home, &local(), &unknown, &filter, Order::Spread, page).unwrap();
        assert!(
            matches!(&refused, Answer::Refused(why) if why.contains("unheard-of")),
            "{refused:?}"
        );
        let unnamed = Reader::model(None, None);
        assert!(matches!(
            where_found(&home, &local(), &unnamed, "a").unwrap(),
            Answer::Refused(_)
        ));
        let ollama = Reader::model(Some("ollama"), Some("small:8b"));
        let Answer::Served(list) =
            fingerprints(&home, &local(), &ollama, &filter, Order::Spread, page).unwrap()
        else {
            panic!("a local model is refused")
        };
        let short = list.rows[0].fingerprint.short().to_string();
        assert!(matches!(
            where_found(&home, &local(), &ollama, &short).unwrap(),
            Answer::Served(_)
        ));

        let log = readlog::list(&home, None, 10).unwrap();
        let results: Vec<_> = log.iter().rev().map(|e| e.result.clone()).collect();
        assert!(matches!(results[0], ReadResult::Refused { .. }));
        assert!(matches!(results[1], ReadResult::Refused { .. }));
        assert_eq!(
            results[2..],
            [
                ReadResult::Findings {
                    fingerprints: 1,
                    sessions: 0
                },
                ReadResult::Findings {
                    fingerprints: 1,
                    sessions: 1
                },
            ]
        );
        assert!(log.iter().all(|e| e.session.is_none()));
        assert_eq!(log[0].reader, ollama);
    }

    #[test]
    fn it_answers_from_stored_findings_without_reading_session_text_or_scanning() {
        let (_d, home) = home_with(&[
            session(
                Harness::ClaudeCode,
                "s1",
                "2026-11-01T09:00:00Z",
                &[&fake(1)],
            ),
            session(
                Harness::ClaudeCode,
                "s2",
                "2026-11-02T09:00:00Z",
                &[&fake(2)],
            ),
        ]);
        // s1's events are gone: only its stored findings remain. s2's findings are gone: it is
        // pending, and is not scanned to answer.
        std::fs::remove_file(home.events_path(Harness::ClaudeCode, "s1")).unwrap();
        let s2 = home.findings_path(Harness::ClaudeCode, "s2");
        std::fs::remove_file(&s2).unwrap();
        let list = owner_list(&home, &FindingFilter::default(), Order::Spread);
        assert_eq!((list.total, list.pending), (1, 1));
        let short = list.rows[0].fingerprint.short().to_string();
        let l = owner_lookup(&home, &short);
        assert_eq!((l.found.len(), l.pending), (1, 1));
        assert_eq!(l.found[0].sessions[0].session, "claude-code/s1");
        assert!(!s2.exists());
    }

    #[test]
    fn a_page_holds_at_most_its_limit_and_says_where_the_next_starts() {
        let sessions: Vec<Session> = (1..=3)
            .map(|n| Session {
                id: ["s1", "s2", "s3"][n as usize - 1],
                ..session(
                    Harness::ClaudeCode,
                    "x",
                    "2026-11-01T09:00:00Z",
                    &[&fake(n)],
                )
            })
            .collect();
        let (_d, home) = home_with(&sessions);
        let page = |offset, limit| match fingerprints(
            &home,
            &local(),
            &Reader::Owner,
            &FindingFilter::default(),
            Order::Spread,
            Page { offset, limit },
        )
        .unwrap()
        {
            Answer::Served(f) => (f.rows.len(), f.total, f.next),
            Answer::Refused(why) => panic!("{why}"),
        };
        assert_eq!(page(0, 2), (2, 3, Some(2)));
        assert_eq!(page(2, 2), (1, 3, None));
    }

    #[test]
    fn a_mirror_of_a_session_in_the_archive_is_not_counted_again() {
        let a = fake(1);
        let (_d, home) = home_with(&[
            session(Harness::ClaudeCode, "s1", "2026-11-01T09:00:00Z", &[&a]),
            Session {
                mirror_of: Some("claude-code/s1"),
                ..session(Harness::T3Code, "t1", "2026-11-01T09:00:00Z", &[&a])
            },
        ]);
        let list = owner_list(&home, &FindingFilter::default(), Order::Spread);
        assert_eq!(list.rows[0].sessions, 1);
        assert_eq!(list.rows[0].harnesses, [Harness::ClaudeCode]);
        let l = owner_lookup(&home, list.rows[0].fingerprint.short());
        assert_eq!(l.found[0].sessions.len(), 1);
    }

    #[test]
    fn as_text_each_fingerprint_shows_its_short_form_and_spread_and_never_the_value() {
        let (a, b) = (fake(1), fake(2));
        let (_d, home) = home_with(&[
            session(Harness::ClaudeCode, "s1", "2026-11-01T09:00:00Z", &[&a]),
            session(Harness::Codex, "s2", "2026-11-05T09:00:00Z", &[&a]),
            session(Harness::Codex, "s3", "2026-11-03T09:00:00Z", &[&b]),
        ]);
        std::fs::remove_file(home.findings_path(Harness::Codex, "s3")).unwrap();
        let list = owner_list(&home, &FindingFilter::default(), Order::Spread);
        let t = list.text();
        let short = list.rows[0].fingerprint.short().to_string();
        assert!(t.starts_with("1 fingerprint"), "{t}");
        let row = t.lines().find(|l| l.contains(&short)).unwrap();
        for part in [
            "anthropic_key",
            "2 sessions",
            "claude-code, codex",
            "first seen 2026-11-01",
            "last seen 2026-11-05",
        ] {
            assert!(row.contains(part), "{part:?} missing: {row}");
        }
        assert!(!row.contains(list.rows[0].fingerprint.as_str()), "{row}");
        assert!(t.contains("1 session") && t.contains("distill scan"), "{t}");

        let l = owner_lookup(&home, &short);
        let t = l.text();
        assert!(
            t.contains(&format!("#{short}")) && t.contains("2 sessions"),
            "{t}"
        );
        let s2 = t.find("codex/s2").unwrap();
        let s1 = t.find("claude-code/s1").unwrap();
        assert!(s2 < s1, "{t}");
        assert!(t.contains("/work/app") && t.contains("events 0"), "{t}");
        let none = owner_lookup(&home, "zz").text();
        assert!(none.contains("No fingerprint starts with \"zz\""), "{none}");
        for t in [list.text(), l.text(), none] {
            assert!(!t.contains(&a) && !t.contains(&b), "{t}");
        }
    }
}
