//! The word index (issue #72, "Organization"): every word of every session's text, as the read
//! path serves it to Distill itself, so each secret is already a placeholder and no secret's value
//! reaches the index. It lives in `index/words/` and is rebuilt from the sessions whenever it is
//! deleted.
//!
//! - **Entries.** The manifest (`manifest.json`) holds one entry per indexed session, recording
//!   what it was built from ([`MadeFrom`]: the session's capture hash and parser version, which
//!   name its events, a digest of its secret findings, and its current card). An entry whose
//!   session's events, secret findings or card have changed is out of date, and the tick indexes
//!   the session again.
//! - **Cards.** A session's current card (`crate::cards`) is indexed with its events: its title,
//!   summary and tags. Their words point at no event.
//! - **Segments.** The words themselves are in immutable segment files ([`segment`]), each holding
//!   some sessions. Indexing a session again writes it to a new segment and points its entry
//!   there; the old copy is dead and is dropped when segments are merged.
//! - **Queries.** A query looks its words up in each segment and counts only current copies, so
//!   it is answered from the index without reading any session, and its ranking depends only on
//!   what is indexed, not on how it is split into segments: deleting the index and rebuilding it
//!   gives the same results.
//!
//! Words are runs of letters, digits and `_`, matched ignoring case. A word with `_` in it is also
//! found by each of its parts. Words longer than [`MAX_WORD`] characters are not indexed.

pub mod segment;
pub mod vectors;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cards::{self, Card};
use crate::config::Config;
use crate::findings::{BuiltFrom, Findings, FingerprintKey};
use crate::home::Home;
use crate::model::{Event, SessionMeta};
use crate::normalize;
use crate::read;
use crate::util::{count, write_atomic};
use segment::{Posting, Segment, Writer};

/// The version of the index's format and of how it splits text into words. An index of another
/// version is rebuilt.
pub const INDEX_VERSION: u32 = 2;

/// Words longer than this many characters are not indexed.
pub const MAX_WORD: usize = 64;

/// How many sessions go into one new segment at most. Small in tests, so that they split the
/// index into segments and merge them.
const BATCH: usize = if cfg!(test) { 4 } else { 256 };
/// How many segments the index keeps before it merges the smallest.
const MAX_SEGMENTS: usize = 8;
/// How many events of a session each word records, for finding its passage.
const MAX_EVENTS: usize = 16;

/// What an index entry was built from. When any of it differs from the session's current state,
/// the entry is out of date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MadeFrom {
    /// The session's events and secret findings: what its placeholders were made from.
    #[serde(flatten)]
    pub session: BuiltFrom,
    /// The [`hash`](Card::hash) of the session's current card, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
}

impl MadeFrom {
    pub fn of(facts: &SessionMeta, findings: &Findings, card: Option<&Card>) -> Self {
        Self {
            session: BuiltFrom::of(facts, findings),
            card: card.map(Card::hash),
        }
    }

    /// What `facts` would be built from now: its events, its current secret findings and its
    /// current card. None when its findings are missing or out of date, because the read path
    /// would scan it first.
    pub(crate) fn now(
        home: &Home,
        config: &Config,
        key: Option<&FingerprintKey>,
        facts: &SessionMeta,
    ) -> Option<Self> {
        let key = key?;
        let f = crate::findings::load(home, facts).filter(|f| f.is_current(facts, key, config))?;
        let card = cards::current(home, facts, &f);
        Some(Self::of(facts, &f, card.as_ref()))
    }
}

/// One indexed session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    segment: u64,
    ord: u32,
    /// How many words its text has.
    words: u64,
    made_from: MadeFrom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SegmentInfo {
    /// Sessions written to it, current or not.
    sessions: u32,
    bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    next_segment: u64,
    segments: BTreeMap<u64, SegmentInfo>,
    /// By session key.
    entries: BTreeMap<String, Entry>,
}

/// The manifest, or an empty one when there is none, it can't be read, or it is another version.
fn load(home: &Home) -> Manifest {
    fs::read_to_string(home.words_manifest_path())
        .ok()
        .and_then(|t| serde_json::from_str::<Manifest>(&t).ok())
        .filter(|m| m.version == INDEX_VERSION)
        .unwrap_or(Manifest {
            version: INDEX_VERSION,
            ..Default::default()
        })
}

/// Whether `entry` was built from `facts`'s current events and secret findings. With no key or
/// no current findings, nothing is current: the read path would scan the session first.
fn is_current(
    home: &Home,
    config: &Config,
    key: Option<&FingerprintKey>,
    facts: &SessionMeta,
    entry: Option<&Entry>,
) -> bool {
    let (Some(key), Some(entry)) = (key, entry) else {
        return false;
    };
    if entry.made_from.session.capture_hash != facts.raw.sha256
        || entry.made_from.session.parser != facts.normalizer.version
    {
        return false;
    }
    crate::findings::load(home, facts)
        .filter(|f| f.is_current(facts, key, config))
        .is_some_and(|f| {
            let card = cards::current(home, facts, &f);
            MadeFrom::of(facts, &f, card.as_ref()) == entry.made_from
        })
}

/// What a build of the index did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexReport {
    /// Sessions in the session list.
    pub sessions: u64,
    /// Sessions indexed now, because their entry was missing or out of date.
    pub indexed: u64,
    /// Sessions whose entry was already current.
    pub current: u64,
    /// Sessions the read path would not serve, left out of the index.
    pub failed: u64,
    /// Entries removed because their session is gone.
    pub removed: u64,
    /// Segments in the index after the build.
    pub segments: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl IndexReport {
    pub fn summary(&self) -> String {
        let mut s = format!(
            "indexed {} ({} already current), {}",
            count(self.indexed, "session"),
            self.current,
            count(self.segments, "segment")
        );
        if self.removed > 0 {
            s += &format!(", {} removed", self.removed);
        }
        if self.failed > 0 {
            s += &format!(", {} failed", self.failed);
        }
        s
    }
}

/// Bring the index up to date: index every session whose entry is missing or out of date, through
/// the read path as Distill itself (one read-log line for the whole build), and drop entries whose
/// session is gone. One writer at a time.
pub fn build(home: &Home) -> Result<IndexReport> {
    crate::util::create_dir_private(&home.words_dir())?;
    let _lock = home.lock("index")?;
    let sessions = normalize::read_session_list(home)?;
    let mut m = load(home);
    let mut r = IndexReport {
        sessions: sessions.len() as u64,
        ..Default::default()
    };
    let keys: BTreeSet<String> = sessions.iter().map(SessionMeta::key).collect();
    let before = m.entries.len();
    m.entries.retain(|k, _| keys.contains(k));
    r.removed = (before - m.entries.len()) as u64;

    let config = home.load_config().unwrap_or_default();
    let key = FingerprintKey::existing(home).ok().flatten();
    let pending: Vec<SessionMeta> = sessions
        .into_iter()
        .filter(|f| !is_current(home, &config, key.as_ref(), f, m.entries.get(&f.key())))
        .collect();
    r.current = r.sessions - pending.len() as u64;

    let mut batch = Batch::default();
    read::for_index(home, &pending, |facts, served| {
        match served {
            Ok((events, findings, card)) => {
                let made_from = MadeFrom::of(facts, findings, card.as_ref());
                batch.add(facts, &events, card.as_ref(), made_from);
                r.indexed += 1;
                if batch.sessions.len() >= BATCH {
                    batch.flush(home, &mut m)?;
                    // Merging as it goes keeps a first build of a large archive to a few
                    // segments open at a time.
                    merge(home, &mut m)?;
                }
            }
            Err(why) => {
                m.entries.remove(&facts.key());
                r.failed += 1;
                r.errors.push(why);
            }
        }
        Ok(())
    })?;
    batch.flush(home, &mut m)?;
    merge(home, &mut m)?;
    // The commit point: the segments it names are written, and those it no longer names go.
    write_atomic(&home.words_manifest_path(), &serde_json::to_vec(&m)?)?;
    remove_unlisted(home, &m);
    r.segments = m.segments.len() as u64;
    Ok(r)
}

/// Delete every file in the index folder the manifest doesn't name: segments merged away, and
/// whatever a build that stopped part way left.
fn remove_unlisted(home: &Home, m: &Manifest) {
    let keep: BTreeSet<PathBuf> = m
        .segments
        .keys()
        .map(|&id| home.words_segment_path(id))
        .chain([home.words_manifest_path()])
        .collect();
    let Ok(rd) = fs::read_dir(home.words_dir()) else {
        return;
    };
    for e in rd.filter_map(Result::ok) {
        if !keep.contains(&e.path()) && e.path().is_file() {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// Sessions read for a new segment, not yet written.
#[derive(Default)]
struct Batch {
    sessions: Vec<(String, u64, MadeFrom)>,
    words: BTreeMap<String, Vec<Posting>>,
}

impl Batch {
    fn add(
        &mut self,
        facts: &SessionMeta,
        events: &[Event],
        card: Option<&Card>,
        made_from: MadeFrom,
    ) {
        let ord = self.sessions.len() as u32;
        let mut found: HashMap<String, (u32, Vec<u64>)> = HashMap::new();
        let mut total = 0u64;
        // Each word of `text`, found in event `seq` or, for a card's, in none.
        let mut words = |text: &str, seq: Option<u64>| {
            for_each_word(text, |w| {
                total += 1;
                let mut note = |w: String| {
                    let (n, evs) = found.entry(w).or_default();
                    *n += 1;
                    if let Some(seq) = seq
                        && evs.len() < MAX_EVENTS
                        && evs.last() != Some(&seq)
                    {
                        evs.push(seq);
                    }
                };
                if w.contains('_') {
                    for part in w.split('_').filter(|p| !p.is_empty()) {
                        note(part.to_string());
                    }
                }
                note(w);
            });
        };
        for e in events {
            for_each_text(e, |text| words(text, Some(e.seq)));
        }
        for (_, text) in card.map(Card::fields).unwrap_or_default() {
            words(text, None);
        }
        for (w, (n, mut evs)) in found {
            evs.sort_unstable();
            evs.dedup();
            self.words.entry(w).or_default().push(Posting {
                ord,
                count: n,
                events: evs,
            });
        }
        self.sessions.push((facts.key(), total, made_from));
    }

    /// Write the batch as a new segment and point its sessions' entries at it.
    fn flush(&mut self, home: &Home, m: &mut Manifest) -> Result<()> {
        if self.sessions.is_empty() {
            return Ok(());
        }
        let id = m.next_segment;
        m.next_segment += 1;
        let keys: Vec<String> = self.sessions.iter().map(|s| s.0.clone()).collect();
        let mut w = Writer::create(&home.words_segment_path(id), &keys)?;
        for (word, postings) in std::mem::take(&mut self.words) {
            w.add(&word, &segment::encode(&postings))?;
        }
        let bytes = w.finish()?;
        m.segments.insert(
            id,
            SegmentInfo {
                sessions: keys.len() as u32,
                bytes,
            },
        );
        for (ord, (key, words, made_from)) in
            std::mem::take(&mut self.sessions).into_iter().enumerate()
        {
            m.entries.insert(
                key,
                Entry {
                    segment: id,
                    ord: ord as u32,
                    words,
                    made_from,
                },
            );
        }
        Ok(())
    }
}

/// The text of an event that is indexed: its text, and every key and string inside a tool call's
/// input.
pub(crate) fn for_each_text(e: &Event, mut f: impl FnMut(&str)) {
    if let Some(t) = &e.text {
        f(t);
    }
    fn walk(v: &Value, f: &mut impl FnMut(&str)) {
        match v {
            Value::String(s) => f(s),
            Value::Array(a) => a.iter().for_each(|v| walk(v, f)),
            Value::Object(o) => o.iter().for_each(|(k, v)| {
                f(k);
                walk(v, f)
            }),
            _ => {}
        }
    }
    if let Some(input) = &e.input {
        walk(input, &mut f);
    }
}

/// Each word of `text` that is indexed, lowercased.
fn for_each_word(text: &str, mut f: impl FnMut(String)) {
    for (start, end) in word_spans(text) {
        f(text[start..end].to_lowercase());
    }
}

/// Where each indexed word of `text` is, as byte ranges.
pub(crate) fn word_spans(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        loop {
            let (start, _) = chars.find(|(_, c)| is_word_char(*c))?;
            let mut end = text.len();
            let mut n = 1;
            while let Some(&(i, c)) = chars.peek() {
                if !is_word_char(c) {
                    end = i;
                    break;
                }
                n += 1;
                chars.next();
            }
            if n <= MAX_WORD {
                return Some((start, end));
            }
        }
    })
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether the word `w` (lowercased) is found by the query word `q`: it is `q`, or `q` is one of
/// its parts between `_`.
pub(crate) fn word_matches(w: &str, q: &str) -> bool {
    w == q || (w.contains('_') && w.split('_').any(|p| p == q))
}

/// The words of a query, lowercased, each once, in order.
pub fn query_words(query: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for_each_word(query, |w| {
        if seen.insert(w.clone()) {
            out.push(w);
        }
    });
    out
}

/// A session that has every word of a query.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WordMatch {
    pub key: String,
    /// BM25 over the indexed sessions; higher is better.
    pub score: f64,
    /// Events to look in for the passage, best first: those with the most of the query's words.
    pub events: Vec<u64>,
}

/// What a query found in the index.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Lookup {
    /// In no particular order.
    pub matches: Vec<WordMatch>,
    /// How many sessions the index holds.
    pub indexed: u64,
}

/// Every indexed session that has all of `words`, scored. Reads the manifest and the words'
/// postings, never a session. A segment merged away while this ran is retried once.
pub(crate) fn lookup(home: &Home, words: &[String]) -> Result<Lookup> {
    match lookup_once(home, words) {
        Ok(l) => Ok(l),
        Err(_) => lookup_once(home, words),
    }
}

fn lookup_once(home: &Home, words: &[String]) -> Result<Lookup> {
    let m = load(home);
    let indexed = m.entries.len() as u64;
    if words.is_empty() || indexed == 0 {
        return Ok(Lookup {
            matches: Vec::new(),
            indexed,
        });
    }
    // Per query word: each session that has it, with its count and events.
    let mut per_word: Vec<HashMap<&str, (u32, Vec<u64>)>> = vec![HashMap::new(); words.len()];
    for &id in m.segments.keys() {
        let seg = Segment::open(&home.words_segment_path(id))?;
        for (i, w) in words.iter().enumerate() {
            let Some(run) = seg.get(w)? else { continue };
            for p in segment::decode(&run)? {
                let Some(key) = seg.sessions.get(p.ord as usize) else {
                    continue;
                };
                let Some((key, e)) = m.entries.get_key_value(key) else {
                    continue;
                };
                if e.segment == id && e.ord == p.ord {
                    per_word[i].insert(key.as_str(), (p.count, p.events));
                }
            }
        }
    }
    let n = indexed as f64;
    let avg = m.entries.values().map(|e| e.words).sum::<u64>() as f64 / n;
    let (k1, b) = (1.2, 0.75);
    let mut matches = Vec::new();
    for key in per_word[0].keys() {
        if !per_word.iter().all(|pw| pw.contains_key(key)) {
            continue;
        }
        let len = m.entries[*key].words as f64;
        let mut score = 0.0;
        let mut hits: BTreeMap<u64, usize> = BTreeMap::new();
        for pw in &per_word {
            let df = pw.len() as f64;
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            let (count, events) = &pw[key];
            let tf = f64::from(*count);
            score += idf * tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * len / avg.max(1.0)));
            for &e in events {
                *hits.entry(e).or_default() += 1;
            }
        }
        let mut events: Vec<(u64, usize)> = hits.into_iter().collect();
        events.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        matches.push(WordMatch {
            key: key.to_string(),
            score,
            events: events
                .into_iter()
                .map(|(e, _)| e)
                .take(MAX_EVENTS)
                .collect(),
        });
    }
    Ok(Lookup { matches, indexed })
}

/// Merge segments that are mostly dead copies, and, past [`MAX_SEGMENTS`], the smallest, into
/// one new segment holding only their current copies.
fn merge(home: &Home, m: &mut Manifest) -> Result<()> {
    let mut live: BTreeMap<u64, u32> = m.segments.keys().map(|&id| (id, 0)).collect();
    for e in m.entries.values() {
        *live.entry(e.segment).or_default() += 1;
    }
    m.segments
        .retain(|id, _| live.get(id).copied().unwrap_or(0) > 0);
    let mut chosen: BTreeSet<u64> = m
        .segments
        .iter()
        .filter(|(id, s)| live[id] * 2 < s.sessions)
        .map(|(&id, _)| id)
        .collect();
    // How many segments are left once the chosen ones are merged into one.
    let left =
        |chosen: &BTreeSet<u64>| m.segments.len() - chosen.len() + usize::from(!chosen.is_empty());
    let mut by_size: Vec<u64> = m
        .segments
        .keys()
        .filter(|id| !chosen.contains(id))
        .copied()
        .collect();
    by_size.sort_by_key(|id| (live[id], *id));
    for id in by_size {
        if left(&chosen) <= MAX_SEGMENTS {
            break;
        }
        chosen.insert(id);
    }
    if chosen.is_empty() {
        return Ok(());
    }

    // The current copies in the chosen segments, in a new order.
    let mut keys: Vec<String> = m
        .entries
        .iter()
        .filter(|(_, e)| chosen.contains(&e.segment))
        .map(|(k, _)| k.clone())
        .collect();
    keys.sort();
    let new_ord: HashMap<&str, u32> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| (k.as_str(), i as u32))
        .collect();
    let id = m.next_segment;
    m.next_segment += 1;
    let mut w = Writer::create(&home.words_segment_path(id), &keys)?;
    let mut sources = Vec::new();
    for &old in &chosen {
        let seg = Segment::open(&home.words_segment_path(old))?;
        // Old ordinal → new ordinal, for the copies that are current.
        let remap: Vec<Option<u32>> = seg
            .sessions
            .iter()
            .enumerate()
            .map(|(ord, k)| {
                m.entries
                    .get(k)
                    .filter(|e| e.segment == old && e.ord == ord as u32)
                    .and_then(|_| new_ord.get(k.as_str()).copied())
            })
            .collect();
        let mut words = seg.words()?;
        let head = words.next().transpose()?;
        sources.push(Merging { words, head, remap });
    }
    while let Some(word) = sources
        .iter()
        .filter_map(|m| m.head.as_ref().map(|(w, _)| w.clone()))
        .min()
    {
        let mut postings = Vec::new();
        for m in &mut sources {
            if m.head.as_ref().is_some_and(|(w, _)| *w == word) {
                let (_, run) = m.head.take().context("a head was just checked")?;
                for mut p in segment::decode(&run)? {
                    if let Some(Some(ord)) = m.remap.get(p.ord as usize) {
                        p.ord = *ord;
                        postings.push(p);
                    }
                }
                m.head = m.words.next().transpose()?;
            }
        }
        if !postings.is_empty() {
            postings.sort_by_key(|p| p.ord);
            w.add(&word, &segment::encode(&postings))?;
        }
    }
    let bytes = w.finish()?;
    for k in &keys {
        let e = m
            .entries
            .get_mut(k)
            .context("a merged session lost its entry")?;
        e.segment = id;
        e.ord = new_ord[k.as_str()];
    }
    m.segments.retain(|id, _| !chosen.contains(id));
    m.segments.insert(
        id,
        SegmentInfo {
            sessions: keys.len() as u32,
            bytes,
        },
    );
    Ok(())
}

/// One segment being merged: its words in order, the next one, and each old ordinal's new one
/// (None for a dead copy).
struct Merging {
    words: segment::Words,
    head: Option<(String, Vec<u8>)>,
    remap: Vec<Option<u32>>,
}

/// How far the index has got, for status.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexProgress {
    pub sessions: u64,
    /// Sessions whose entry is current.
    pub current: u64,
    /// Sessions whose entry is missing or out of date.
    pub pending: u64,
    pub segments: u64,
    /// Bytes the index takes on disk.
    pub bytes: u64,
}

/// Index progress over the session list. Changes nothing.
pub fn progress(home: &Home) -> Result<IndexProgress> {
    let sessions = normalize::read_session_list(home)?;
    let m = load(home);
    let config = home.load_config().unwrap_or_default();
    let key = FingerprintKey::existing(home).ok().flatten();
    let current = sessions
        .iter()
        .filter(|f| is_current(home, &config, key.as_ref(), f, m.entries.get(&f.key())))
        .count() as u64;
    Ok(IndexProgress {
        sessions: sessions.len() as u64,
        current,
        pending: sessions.len() as u64 - current,
        segments: m.segments.len() as u64,
        bytes: m.segments.values().map(|s| s.bytes).sum(),
    })
}

#[cfg(test)]
pub(crate) mod tests;
