//! Search through the read path (issue #72, "Organization" and "The read path"): sessions that
//! match a query, best first, each with its facts, the opening of what the owner first wrote and
//! the matching passage, prepared for the reader like any other session text, and logged as one
//! line with a count. A session's current card is part of the word index and of its hit; a model
//! reads the card's title and summary in place of the opening.
//!
//! The candidates come from the index, never from reading every session: only the sessions on the
//! page asked for are read, to prepare their openings and passages. There are two sources, each a
//! ranked list: the word index ([`Source::Words`]) and the vectors ([`Source::Meaning`]), which
//! [`merge`] fuses into one. Without an `embedding_model` or a local runtime that can run it,
//! search finds by words alone and says so ([`Meaning::Skipped`]).

use std::collections::{BTreeMap, HashMap};

use anyhow::{Result, bail};
use regex::Regex;
use serde_json::Value;

use super::list::{LEFT_OUT, card_or_opening, facts_line, opening_of};
use super::{Filter, NOTICE_END, NOTICE_START, Opening, PAGE_MAX, Page, defuse, mask, scanned};
use crate::cards::Shown;
use crate::config::Config;
use crate::findings::{Field, Finding, Findings};
use crate::home::Home;
use crate::index;
use crate::index::vectors::{self, Nearest};
use crate::model::{Event, EventKind, Originals, Roots, SessionMeta};
use crate::normalize;
use crate::readlog::{self, ReadEntry, ReadResult};
use crate::runtime::LocalRuntime;
use crate::trust::{Access, Reader, Trust};
use crate::util::{count, now_rfc3339};

/// How many hits a face asks for when the caller doesn't say.
pub const SEARCH_DEFAULT: usize = 10;

/// What a searcher gets.
#[derive(Debug, Clone, PartialEq)]
pub enum Searched {
    /// For a model: the hits as text. Openings and passages sit between [`NOTICE_START`] and
    /// [`NOTICE_END`].
    Text(String),
    /// For the owner and DISTILL itself.
    Hits(Box<Hits>),
    /// No hits, no facts and no counts, and why.
    Refused(String),
}

/// One page of the sessions that match, best first.
#[derive(Debug, Clone, PartialEq)]
pub struct Hits {
    pub hits: Vec<Hit>,
    /// How many sessions match in all.
    pub total: u64,
    /// Where the next page starts, when there is one.
    pub next: Option<usize>,
    /// Sessions in the session list the index doesn't hold yet, which a search can't find.
    pub unindexed: u64,
    /// Whether search by meaning was part of this search.
    pub meaning: Meaning,
}

/// Whether a search was also by meaning.
#[derive(Debug, Clone, PartialEq)]
pub enum Meaning {
    /// It was. Some of the sessions searched may have no vectors yet.
    Used {
        /// Sessions searched that have no vectors yet, which only words can find.
        unembedded: u64,
    },
    /// It was not, and why: the results are word matches only.
    Skipped(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub facts: SessionMeta,
    /// The session's current card. A model reads its title and summary in place of the opening.
    pub card: Option<Shown>,
    pub opening: Opening,
    /// For the owner, the findings inside the opening's text, their ranges counted from its
    /// start. Empty for everyone else.
    pub opening_findings: Vec<Finding>,
    /// Where the session matches; None when it was withheld from the reader or holds no text.
    pub passage: Option<Passage>,
    /// What found it.
    pub found_by: Vec<Source>,
    /// When an agent started the session (a sub-agent, a delegated task): the session the owner
    /// started it from.
    pub started_from: Option<SessionMeta>,
}

/// A source of search results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// The word index: every word of the query is in the session.
    Words,
    /// The vectors: the session is close in meaning to the query.
    Meaning,
}

/// The matching part of one string in one event, as the reader is served it.
#[derive(Debug, Clone, PartialEq)]
pub struct Passage {
    /// The event, by sequence number.
    pub event: u64,
    pub kind: EventKind,
    /// Which string of the event: its text, or a key or string inside a tool call's input.
    pub field: Field,
    /// Part of that string.
    pub text: String,
    /// Whether the string goes on before and after `text`.
    pub cut_before: bool,
    pub cut_after: bool,
    /// For the owner, the findings inside `text`, their ranges counted from its start. Empty for
    /// everyone else, whose text holds placeholders instead.
    pub findings: Vec<Finding>,
}

/// Search for sessions that have every word of `query`, among those that match `filter`, best
/// first, the hits in `page`, prepared for `reader`. A reader that may not read is refused before
/// anything is looked up. Every call appends one line to the read log with how many sessions it
/// served; the query itself is not logged.
pub fn search(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    query: &str,
    filter: &Filter,
    page: Page,
) -> Result<Searched> {
    let mut served = 0;
    let searched = serve(home, runtime, reader, query, filter, page, &mut served);
    let result = match &searched {
        Ok(Searched::Text(_) | Searched::Hits(_)) => ReadResult::Listed { sessions: served },
        Ok(Searched::Refused(reason)) => ReadResult::Refused {
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
    )?;
    searched
}

/// One source's match: a session and where in it to look for the passage, best first.
struct Found {
    key: String,
    events: Vec<u64>,
}

/// One source's ranked list.
type Ranked = (Source, Vec<Found>);

/// Each source's matches among `candidates`, each list best first, how many sessions the word
/// index holds, and whether search by meaning took part.
fn found(
    home: &Home,
    config: &Config,
    runtime: &LocalRuntime,
    query: &str,
    words: &[String],
    candidates: &HashMap<String, &SessionMeta>,
) -> Result<(Vec<Ranked>, u64, Meaning)> {
    let lookup = index::lookup(home, words)?;
    let mut by_words: Vec<(f64, &SessionMeta, index::WordMatch)> = lookup
        .matches
        .into_iter()
        .filter_map(|m| candidates.get(&m.key).map(|f| (m.score, *f, m)))
        .collect();
    // Best score first; among equals, newest first.
    by_words.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(b.1.started_at.cmp(&a.1.started_at))
            .then(a.2.key.cmp(&b.2.key))
    });
    let words_list = by_words
        .into_iter()
        .map(|(_, _, m)| Found {
            key: m.key,
            events: m.events,
        })
        .collect();
    let among: BTreeMap<String, Option<&str>> = candidates
        .iter()
        .map(|(k, f)| (k.clone(), f.started_at.as_deref()))
        .collect();
    let (meaning_list, meaning) = match vectors::nearest(home, config, runtime, query, &among) {
        Nearest::Found {
            matches,
            unembedded,
        } => (
            matches
                .into_iter()
                .map(|m| Found {
                    key: m.key,
                    events: m.event.into_iter().collect(),
                })
                .collect(),
            Meaning::Used { unembedded },
        ),
        Nearest::Skipped(why) => (Vec::new(), Meaning::Skipped(why)),
    };
    Ok((
        vec![(Source::Words, words_list), (Source::Meaning, meaning_list)],
        lookup.indexed,
        meaning,
    ))
}

/// A session found by one or more sources.
struct Merged {
    key: String,
    events: Vec<u64>,
    found_by: Vec<Source>,
}

/// Fuse each source's ranked list into one by reciprocal rank: a session high in any list, or in
/// several, comes first. With one list its order is kept.
fn merge(lists: Vec<Ranked>) -> Vec<Merged> {
    const K: f64 = 60.0;
    let mut by_key: BTreeMap<String, (f64, Merged)> = BTreeMap::new();
    for (source, list) in lists {
        for (rank, f) in list.into_iter().enumerate() {
            let entry = by_key.entry(f.key.clone()).or_insert_with(|| {
                (
                    0.0,
                    Merged {
                        key: f.key.clone(),
                        events: Vec::new(),
                        found_by: Vec::new(),
                    },
                )
            });
            entry.0 += 1.0 / (K + rank as f64 + 1.0);
            entry.1.found_by.push(source);
            for e in f.events {
                if !entry.1.events.contains(&e) {
                    entry.1.events.push(e);
                }
            }
        }
    }
    let mut merged: Vec<(f64, Merged)> = by_key.into_values().collect();
    merged.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.key.cmp(&b.1.key)));
    merged.into_iter().map(|(_, m)| m).collect()
}

/// What `reader` gets.
fn serve(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    query: &str,
    filter: &Filter,
    page: Page,
    served: &mut u64,
) -> Result<Searched> {
    let config = home.load_config().unwrap_or_default();
    let trust = Trust::new(&config);
    if let Access::Refused(why) = trust.check(reader, runtime) {
        return Ok(Searched::Refused(why));
    }
    let words = index::query_words(query);
    if words.is_empty() {
        bail!("a search needs at least one word");
    }
    let all = normalize::read_session_list(home)?;
    let originals = Originals::of(&all);
    let roots = Roots::of(&all);
    let key = crate::findings::FingerprintKey::existing(home)
        .ok()
        .flatten();
    let finds = crate::cards::finder(&config);
    let candidates: HashMap<String, &SessionMeta> = all
        .iter()
        .filter(|m| filter.matches(m, &originals))
        .filter(|m| filter.card_matches(home, &config, key.as_ref(), m, &finds))
        .map(|m| (m.key(), m))
        .collect();
    let (lists, indexed, meaning) = found(home, &config, runtime, query, &words, &candidates)?;
    let merged = merge(lists);
    let total = merged.len() as u64;
    let limit = page.limit.min(PAGE_MAX);
    let hits: Vec<Hit> = merged
        .into_iter()
        .skip(page.offset)
        .take(limit)
        .map(|m| {
            let mut h = hit(home, &config, reader, candidates[&m.key], m, &words);
            h.started_from = roots
                .of_session(&h.facts.key())
                .and_then(|r| all.iter().find(|m| m.key() == r))
                // The previous steward's sessions stay out of search unless asked for.
                .filter(|r| !r.steward || filter.steward)
                .cloned();
            h
        })
        .collect();
    *served = hits.len() as u64;
    let end = page.offset + hits.len();
    let hits = Hits {
        next: ((end as u64) < total && !hits.is_empty()).then_some(end),
        hits,
        total,
        unindexed: (all.len() as u64).saturating_sub(indexed),
        meaning,
    };
    Ok(match reader {
        Reader::Model { .. } => Searched::Text(render(&hits, page.offset)),
        Reader::Owner | Reader::Distill => Searched::Hits(Box::new(hits)),
    })
}

/// One hit, read through the read path for `reader`.
fn hit(
    home: &Home,
    config: &Config,
    reader: &Reader,
    facts: &SessionMeta,
    m: Merged,
    words: &[String],
) -> Hit {
    let withheld = |why: String| Hit {
        facts: facts.clone(),
        card: None,
        opening: Opening::Withheld(why),
        opening_findings: Vec::new(),
        passage: None,
        found_by: m.found_by.clone(),
        started_from: None,
    };
    let (events, scan) = match scanned(home, config, facts) {
        Ok(Ok(scanned)) => scanned,
        Ok(Err(why)) => return withheld(why),
        Err(e) => return withheld(format!("{} could not be read: {e:#}", facts.key())),
    };
    let card = Shown::of(home, facts, &scan, matches!(reader, Reader::Owner), config);
    let (events, owner_findings) = match reader {
        Reader::Owner => (events, Some(&scan)),
        Reader::Model { .. } | Reader::Distill => match mask(events, &scan) {
            Ok(masked) => (masked, None),
            Err(e) => {
                return withheld(format!(
                    "{}'s findings do not fit its events ({e:#})",
                    facts.key()
                ));
            }
        },
    };
    let (opening, opening_findings) = opening_of(&events, owner_findings);
    Hit {
        facts: facts.clone(),
        card,
        opening,
        opening_findings,
        passage: passage(&events, owner_findings, words, &m.events),
        found_by: m.found_by,
        started_from: None,
    }
}

/// How much of a string a passage shows around the first match, in bytes.
const BEFORE: usize = 80;
const AFTER: usize = 240;

/// The passage of `events` that best matches `words`: the string, in the events the index pointed
/// at, holding the most of them, and around the first. When none holds any (a word the index
/// found in a placeholder, which the owner's stored text doesn't have), the start of the first
/// string of the first event pointed at; and None when it pointed at none, because the words are
/// only in the session's card.
fn passage(
    events: &[Event],
    findings: Option<&Findings>,
    words: &[String],
    pointed: &[u64],
) -> Option<Passage> {
    let by_seq: HashMap<u64, &Event> = events.iter().map(|e| (e.seq, e)).collect();
    let pointed: Vec<&Event> = pointed
        .iter()
        .filter_map(|s| by_seq.get(s).copied())
        .collect();
    let mut best: Option<(usize, &Event, Field, &str, usize)> = None;
    for e in pointed.iter().copied().chain(events) {
        for (field, text) in strings(e) {
            let mut have = vec![false; words.len()];
            let mut first = None;
            for (s, end) in index::word_spans(text) {
                let w = text[s..end].to_lowercase();
                for (i, q) in words.iter().enumerate() {
                    if index::word_matches(&w, q) {
                        have[i] = true;
                        first.get_or_insert(s);
                    }
                }
            }
            let n = have.iter().filter(|h| **h).count();
            if n > 0 && best.as_ref().is_none_or(|b| n > b.0) {
                best = Some((n, e, field, text, first.unwrap_or(0)));
            }
        }
        if best.as_ref().is_some_and(|b| b.0 == words.len()) {
            break;
        }
    }
    let (e, field, text, at) = match best {
        Some((_, e, field, text, at)) => (e, field, text, at),
        None => {
            let e = pointed.first().copied()?;
            let (field, text) = strings(e).into_iter().next()?;
            (e, field, text, 0)
        }
    };
    let mut start = floor(text, at.saturating_sub(BEFORE));
    let mut end = ceil(text, at.saturating_add(AFTER).min(text.len()));
    // Cut between words where there is a space to cut at.
    if start > 0
        && let Some(i) = text[start..at].find(char::is_whitespace)
    {
        start = ceil(text, start + i + 1);
    }
    if end < text.len()
        && let Some(i) = text[at..end].rfind(char::is_whitespace)
        && at + i > at
    {
        end = at + i;
    }
    // Never cut a finding or a placeholder in two.
    let kept: Vec<&Finding> = findings
        .map(|f| f.in_event(e.seq).filter(|f| f.field == field).collect())
        .unwrap_or_default();
    let spans: Vec<(usize, usize)> = match findings {
        Some(_) => kept.iter().map(|f| (f.start, f.end)).collect(),
        None => placeholders(text),
    };
    for &(s, t) in &spans {
        if s < start && t > start {
            start = floor(text, s);
        }
        if s < end && t > end {
            end = ceil(text, t.min(text.len()));
        }
    }
    let findings = kept
        .into_iter()
        .filter(|f| f.start >= start && f.end <= end)
        .map(|f| Finding {
            start: f.start - start,
            end: f.end - start,
            ..f.clone()
        })
        .collect();
    Some(Passage {
        event: e.seq,
        kind: e.kind,
        field,
        text: text[start..end].to_string(),
        cut_before: start > 0,
        cut_after: end < text.len(),
        findings,
    })
}

/// Every string of an event a passage can come from, with where it is: its text, then each key
/// and string inside a tool call's input.
fn strings(e: &Event) -> Vec<(Field, &str)> {
    let mut out = Vec::new();
    if let Some(t) = e.text.as_deref().filter(|t| !t.trim().is_empty()) {
        out.push((Field::Text, t));
    }
    fn walk<'a>(v: &'a Value, at: &mut Vec<usize>, out: &mut Vec<(Field, &'a str)>) {
        let field = |at: &[usize], key| Field::Input {
            path: at.to_vec(),
            key,
        };
        match v {
            Value::String(s) => out.push((field(at, false), s)),
            Value::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    at.push(i);
                    walk(v, at, out);
                    at.pop();
                }
            }
            Value::Object(o) => {
                for (i, (k, v)) in o.iter().enumerate() {
                    at.push(i);
                    out.push((field(at, true), k));
                    walk(v, at, out);
                    at.pop();
                }
            }
            _ => {}
        }
    }
    if let Some(input) = &e.input {
        walk(input, &mut Vec::new(), &mut out);
    }
    out
}

/// Where each placeholder in `text` is.
fn placeholders(text: &str) -> Vec<(usize, usize)> {
    static PLACEHOLDER: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PLACEHOLDER
        .get_or_init(|| Regex::new(r"\[secret: [a-z0-9_]+ #[0-9a-f]+\]").expect("a valid regex"))
        .find_iter(text)
        .map(|m| (m.start(), m.end()))
        .collect()
}

fn floor(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// A page of hits as text for a model: a line about the page, then the hits between the two
/// notices.
fn render(hits: &Hits, offset: usize) -> String {
    let mut head = if hits.hits.is_empty() {
        if hits.total == 0 {
            format!("No sessions match. {LEFT_OUT}")
        } else {
            format!(
                "No sessions on this page: {} match in all, the first at offset 0. {LEFT_OUT}",
                hits.total
            )
        }
    } else {
        let mut h = format!(
            "Sessions {}–{} of {} that match, best first. {LEFT_OUT}",
            offset + 1,
            offset + hits.hits.len(),
            hits.total
        );
        match hits.next {
            Some(next) => h += &format!(" More: the next page starts at offset {next}."),
            None => h += " This is the last page.",
        }
        h
    };
    if hits.unindexed > 0 {
        let n = hits.unindexed;
        head += &format!(
            " {} {} not in the word index yet, so no search finds {}.",
            count(n, "session"),
            if n == 1 { "is" } else { "are" },
            if n == 1 { "it" } else { "them" }
        );
    }
    match &hits.meaning {
        Meaning::Skipped(why) => {
            head += &format!(" Search by meaning is off ({why}), so these are word matches only.");
        }
        Meaning::Used { unembedded } if *unembedded > 0 => {
            let n = *unembedded;
            head += &format!(
                " {} {} no vectors yet, so only {} words find {}.",
                count(n, "session"),
                if n == 1 { "has" } else { "have" },
                if n == 1 { "its" } else { "their" },
                if n == 1 { "it" } else { "them" }
            );
        }
        Meaning::Used { .. } => {}
    }
    head.push('\n');
    if hits.hits.is_empty() {
        return head;
    }
    let mut body = String::new();
    for h in &hits.hits {
        body += &format!(
            "\n{}{}\n{}",
            facts_line(&h.facts),
            h.started_from
                .as_ref()
                .map(|r| format!(" · started from {}", r.key()))
                .unwrap_or_default(),
            card_or_opening(h.card.as_ref(), &h.opening)
        );
        if let Some(p) = &h.passage {
            let words: Vec<&str> = p.text.split_whitespace().collect();
            body += &format!(
                "  Matched in event {} ({}{}): {}{}{}\n",
                p.event,
                kind(p.kind),
                match &p.field {
                    Field::Text => String::new(),
                    Field::Input { key: true, .. } => ", a key of its input".to_string(),
                    Field::Input { .. } => ", its input".to_string(),
                },
                if p.cut_before { "…" } else { "" },
                words.join(" "),
                if p.cut_after { "…" } else { "" },
            );
        }
    }
    format!("{head}{NOTICE_START}\n{}\n{NOTICE_END}\n", defuse(&body))
}

fn kind(k: EventKind) -> String {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
