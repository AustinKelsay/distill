//! The word index (issue #72, "Organization"): every word of every session's text, as the read
//! path serves it to DISTILL itself, so each secret is already a placeholder and no secret's value
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
//! found by each of its parts. Words longer than [`MAX_WORD`] characters are not indexed, nor are
//! words inside encoded data: a run of at least [`ENCODED`] bytes of base64's characters with a
//! digit in it (issue #154). Images and payloads in tool output split into tens of millions of
//! words found in one session each, nearly the whole index, and nobody searches for them.

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
pub const INDEX_VERSION: u32 = 3;

/// Words longer than this many characters are not indexed.
pub const MAX_WORD: usize = 64;

/// A run of base64's characters (letters, digits, `+`, `/`, `=`, `_`, `-`) at least this many
/// bytes long, with a digit in it, is encoded data, and none of its words are indexed. A path or a
/// slug that long has a `.` or a space in it, or no digit.
pub const ENCODED: usize = 100;

/// How many sessions go into one new segment at most. Small in tests, so that they split the
/// index into segments and merge them.
const BATCH: usize = if cfg!(test) { 4 } else { 256 };
/// How many bytes of words and postings a batch holds in memory at most before it is written, as
/// [`Batch`] counts them (a little under what they take). A session is never split, so one
/// session's words can pass it. Small in tests.
const BUDGET: usize = if cfg!(test) {
    64 * 1024
} else {
    192 * 1024 * 1024
};
/// A session with more bytes of text than this is indexed alone, in passes, so that its words
/// are never all in memory at once.
const GIANT: usize = if cfg!(test) {
    256 * 1024
} else {
    16 * 1024 * 1024
};
/// How many distinct words one pass over a giant session holds at most. A distinct word takes
/// about 300 bytes while it is counted.
const PASS: u64 = if cfg!(test) { 20_000 } else { 1_000_000 };
/// Bits in the map a giant session's first pass marks each distinct word in: 16 MiB, whatever
/// the session's size. Two words that share a bit count once, so a session with tens of millions
/// of distinct words is counted a little short.
const SEEN_BITS: usize = if cfg!(test) { 1 << 16 } else { 1 << 27 };
/// How many segments of about the same size the index keeps before it merges them into one.
/// Segments are in tiers by size, each [`TIER`] times the one below, starting at [`FLOOR`] bytes;
/// a tier that fills is merged into one segment of the tier above. So each copy of a session is
/// rewritten about once per tier, and the index holds at most `TIER - 1` segments per tier.
const TIER: usize = 8;
/// The size below which every segment is in the lowest tier.
const FLOOR: u64 = if cfg!(test) { 1024 } else { 4 * 1024 * 1024 };

/// The tier of a segment of `bytes` (see [`TIER`]).
fn tier(bytes: u64) -> u32 {
    let (mut t, mut top) = (0, FLOOR * TIER as u64);
    while bytes >= top {
        t += 1;
        top = top.saturating_mul(TIER as u64);
    }
    t
}
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
    if entry.made_from.session.capture_hash != facts.raw.source()
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
/// the read path as DISTILL itself (one read-log line for the whole build), and drop entries whose
/// session is gone. One writer at a time.
pub fn build(home: &Home) -> Result<IndexReport> {
    build_within(home, Limits::DEFAULT)
}

/// What bounds the memory of a build.
#[derive(Debug, Clone, Copy)]
struct Limits {
    /// A batch is written once it holds this many bytes (see [`BUDGET`]).
    budget: usize,
    /// A session whose text is longer than this many bytes is written as a segment of its own,
    /// in passes over word ranges (see [`write_alone`]).
    giant: usize,
    /// How many distinct words one such pass holds at most.
    pass: u64,
}

impl Limits {
    const DEFAULT: Limits = Limits {
        budget: BUDGET,
        giant: GIANT,
        pass: PASS,
    };
}

/// [`build`] within `limits`.
fn build_within(home: &Home, limits: Limits) -> Result<IndexReport> {
    crate::util::create_dir_private(&home.words_dir())?;
    let _lock = home.lock("index")?;
    let sessions = normalize::read_session_list(home)?;
    let mut m = load(home);
    // The segments the saved manifest names, which a query may be reading. Any other segment
    // this build writes and then merges away is deleted at once.
    let saved: BTreeSet<u64> = m.segments.keys().copied().collect();
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
                if text_bytes(&events, card.as_ref()) > limits.giant {
                    batch.flush(home, &mut m)?;
                    write_alone(
                        home,
                        &mut m,
                        facts,
                        &events,
                        card.as_ref(),
                        made_from,
                        limits.pass,
                    )?;
                    r.indexed += 1;
                    merge(home, &mut m, &saved)?;
                    return Ok(());
                }
                batch.add(facts, &events, card.as_ref(), made_from);
                r.indexed += 1;
                if batch.is_full(limits.budget) {
                    batch.flush(home, &mut m)?;
                    // Merging as it goes keeps a first build of a large archive to a few
                    // segments open at a time.
                    merge(home, &mut m, &saved)?;
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
    merge(home, &mut m, &saved)?;
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
    /// What `words` takes, counted as [`WORD_COST`], [`POSTING_COST`] and the events' bytes.
    bytes: usize,
}

/// What a word new to a batch takes besides its bytes: its map node and its list.
const WORD_COST: usize = 64;
/// What a posting takes besides its events: the struct and its list's slot.
const POSTING_COST: usize = 48;

impl Batch {
    /// Whether the batch should be written now: it holds [`BATCH`] sessions or `budget` bytes.
    fn is_full(&self, budget: usize) -> bool {
        self.sessions.len() >= BATCH || self.bytes >= budget
    }

    fn add(
        &mut self,
        facts: &SessionMeta,
        events: &[Event],
        card: Option<&Card>,
        made_from: MadeFrom,
    ) {
        let ord = self.sessions.len() as u32;
        let mut found = Found::new();
        let total = notes(events, card, |w, seq| note(&mut found, w, seq));
        for (w, (n, mut evs)) in found {
            let w_len = w.len();
            evs.sort_unstable();
            evs.dedup();
            self.bytes += POSTING_COST + evs.len() * 8;
            let list = self.words.entry(w).or_default();
            if list.is_empty() {
                self.bytes += WORD_COST + w_len;
            }
            list.push(Posting {
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
        self.bytes = 0;
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

/// A session's words, each with its count and the first events it is in.
type Found = HashMap<String, (u32, Vec<u64>)>;

/// Record one occurrence of `w`, found in event `seq` or, for a card's, in none.
fn note(found: &mut Found, w: String, seq: Option<u64>) {
    let (n, evs) = found.entry(w).or_default();
    *n += 1;
    if let Some(seq) = seq
        && evs.len() < MAX_EVENTS
        && evs.last() != Some(&seq)
    {
        evs.push(seq);
    }
}

/// Call `f` with each occurrence of an indexed word in a session's events and card, in event
/// order, with the event it is in (none for the card): a word with `_` in it once for each of its
/// parts and once whole. Returns how many words the text has.
fn notes(events: &[Event], card: Option<&Card>, mut f: impl FnMut(String, Option<u64>)) -> u64 {
    let mut total = 0u64;
    let mut words = |text: &str, seq: Option<u64>| {
        for_each_word(text, |w| {
            total += 1;
            if w.contains('_') {
                for part in w.split('_').filter(|p| !p.is_empty()) {
                    f(part.to_string(), seq);
                }
            }
            f(w, seq);
        });
    };
    for e in events {
        for_each_text(e, |text| words(text, Some(e.seq)));
    }
    for (_, text) in card.map(Card::fields).unwrap_or_default() {
        words(text, None);
    }
    total
}

/// How many bytes of text a session has to index.
fn text_bytes(events: &[Event], card: Option<&Card>) -> usize {
    let mut n = 0;
    for e in events {
        for_each_text(e, |t| n += t.len());
    }
    n + card
        .map(Card::fields)
        .unwrap_or_default()
        .iter()
        .map(|(_, t)| t.len())
        .sum::<usize>()
}

/// Where a word sorts, by its first two bytes: never lower for a later word.
fn bucket(w: &str) -> usize {
    let b = w.as_bytes();
    (usize::from(b[0]) << 8) | usize::from(b.get(1).copied().unwrap_or(0))
}

/// Index one session as a segment of its own, in passes over ranges of words, so that its words
/// are never all in memory at once: a first pass counts the distinct words in each [`bucket`]
/// (see [`SEEN_BITS`]), the buckets are cut into ranges of about `pass` distinct words, and each
/// range is counted and written in word order before the next. Each pass reads the whole session,
/// so they are cut by distinct words, which take memory, not by occurrences, which repeat. The
/// segment is the one a batch of this session alone would write.
fn write_alone(
    home: &Home,
    m: &mut Manifest,
    facts: &SessionMeta,
    events: &[Event],
    card: Option<&Card>,
    made_from: MadeFrom,
    pass: u64,
) -> Result<()> {
    let mut sizes = vec![0u64; 1 << 16];
    let mut seen = vec![0u64; SEEN_BITS / 64];
    let total = notes(events, card, |w, _| {
        let mut h = std::hash::DefaultHasher::new();
        std::hash::Hash::hash(&w, &mut h);
        let bit = std::hash::Hasher::finish(&h) as usize % SEEN_BITS;
        let (word, mask) = (bit / 64, 1u64 << (bit % 64));
        if seen[word] & mask == 0 {
            seen[word] |= mask;
            sizes[bucket(&w)] += 1;
        }
    });
    drop(seen);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let (mut start, mut held) = (0, 0u64);
    for (b, &n) in sizes.iter().enumerate() {
        if held > 0 && held + n > pass {
            ranges.push((start, b));
            (start, held) = (b, 0);
        }
        held += n;
    }
    ranges.push((start, sizes.len()));
    drop(sizes);

    let id = m.next_segment;
    m.next_segment += 1;
    let key = facts.key();
    let mut w = Writer::create(&home.words_segment_path(id), std::slice::from_ref(&key))?;
    for (from, to) in ranges {
        let mut found = Found::new();
        notes(events, card, |word, seq| {
            if (from..to).contains(&bucket(&word)) {
                note(&mut found, word, seq);
            }
        });
        let mut sorted: Vec<_> = found.into_iter().collect();
        sorted.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        for (word, (count, mut evs)) in sorted {
            evs.sort_unstable();
            evs.dedup();
            w.add(
                &word,
                &segment::encode(&[Posting {
                    ord: 0,
                    count,
                    events: evs,
                }]),
            )?;
        }
    }
    let bytes = w.finish()?;
    m.segments.insert(id, SegmentInfo { sessions: 1, bytes });
    m.entries.insert(
        key,
        Entry {
            segment: id,
            ord: 0,
            words: total,
            made_from,
        },
    );
    Ok(())
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
    let encoded = encoded_runs(text);
    let mut skip = encoded.into_iter().peekable();
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
            while skip.next_if(|&(_, e)| e < end).is_some() {}
            let inside = skip.peek().is_some_and(|&(s, e)| s <= start && end <= e);
            if n <= MAX_WORD && !inside {
                return Some((start, end));
            }
        }
    })
}

/// The runs of encoded data in `text` (see [`ENCODED`]), as byte ranges, in order.
fn encoded_runs(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut start, mut digit) = (None, false);
    for (i, b) in text.bytes().enumerate().chain([(text.len(), b' ')]) {
        if b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'_' | b'-') {
            start.get_or_insert(i);
            digit |= b.is_ascii_digit();
        } else if let Some(s) = start.take() {
            if digit && i - s >= ENCODED {
                out.push((s, i));
            }
            digit = false;
        }
    }
    out
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

/// Merge until nothing is left to merge: segments that are mostly dead copies, and any tier that
/// holds [`TIER`] segments, each into one new segment holding only their current copies. A
/// segment it drops is deleted unless it is one of `saved`, which a query may be reading until
/// the manifest that drops it is written; so a first build of a large archive holds about one
/// index's worth of segments at a time, not every segment it wrote.
fn merge(home: &Home, m: &mut Manifest, saved: &BTreeSet<u64>) -> Result<()> {
    let discard = |id: u64| {
        if !saved.contains(&id) {
            let _ = fs::remove_file(home.words_segment_path(id));
        }
    };
    loop {
        let mut live: BTreeMap<u64, u32> = m.segments.keys().map(|&id| (id, 0)).collect();
        for e in m.entries.values() {
            *live.entry(e.segment).or_default() += 1;
        }
        m.segments.retain(|&id, _| {
            let keep = live.get(&id).copied().unwrap_or(0) > 0;
            if !keep {
                discard(id);
            }
            keep
        });
        let mut chosen: BTreeSet<u64> = m
            .segments
            .iter()
            .filter(|(id, s)| live[id] * 2 < s.sessions)
            .map(|(&id, _)| id)
            .collect();
        if chosen.is_empty() {
            let mut tiers: BTreeMap<u32, BTreeSet<u64>> = BTreeMap::new();
            for (&id, s) in &m.segments {
                tiers.entry(tier(s.bytes)).or_default().insert(id);
            }
            if let Some(full) = tiers.into_values().find(|ids| ids.len() >= TIER) {
                chosen = full;
            }
        }
        if chosen.is_empty() {
            return Ok(());
        }
        merge_into_one(home, m, chosen, &discard)?;
    }
}

/// Merge the `chosen` segments into one new segment holding only their current copies, and
/// `discard` each of them.
fn merge_into_one(
    home: &Home,
    m: &mut Manifest,
    chosen: BTreeSet<u64>,
    discard: &impl Fn(u64),
) -> Result<()> {
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
    drop(sources);
    chosen.into_iter().for_each(discard);
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
