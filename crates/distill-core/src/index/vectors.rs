//! Vectors for search by meaning (issue #72, "Organization" and "The steward"): what the owner
//! wrote in each session, and its card's summary, embedded by the config's `embedding_model`
//! through the local client, kept in one flat file (`index/vectors.bin`) and searched directly.
//!
//! - **What is embedded.** The session's messages from the owner as the read path serves them to
//!   the embedding model, so each secret is a placeholder; long sessions are cut into pieces of at
//!   most [`CHUNK`] bytes, at most [`MAX_CHUNKS`] of them spread over the session. The current
//!   card's summary is one more text. Nothing else of the session is embedded.
//! - **Currency.** Each session's vectors record what they were built from
//!   ([`MadeFrom`](super::MadeFrom): its events, its secret findings and its current card), and
//!   are rebuilt when any of it changes. The file names the model that made it; vectors from
//!   another model are not comparable, so a file of another model or version is rebuilt whole.
//! - **Which sessions.** The sessions that get a card ([`cards::wanted`]), newest first, up to
//!   `sessions_per_run` each tick. Deleting the file and running the tick rebuilds it.
//! - **Search.** A query is embedded by the same model and compared with every vector in the
//!   file; a session scores its best vector. A query that holds a secret is never embedded.
//!
//! The file: [`MAGIC`], a little-endian `u32` length and a JSON header of that length (the
//! version, the model, the vectors' length, and per session its key, what it was built from and
//! each vector's event), then every vector as little-endian `f32`s, in the header's order, each of
//! unit length.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::MadeFrom;
use crate::cards::{self, Card};
use crate::config::Config;
use crate::detect::{Class, Detector};
use crate::findings::{self, FingerprintKey};
use crate::home::Home;
use crate::model::{Event, EventKind, Originals, SessionMeta};
use crate::normalize;
use crate::read;
use crate::runtime::{LocalRuntime, RuntimeError, RuntimeState};
use crate::tick::Deadline;
use crate::util::{count, write_atomic};

/// The version of the file's format and of what is embedded. A file of another version is
/// rebuilt.
pub const VECTORS_VERSION: u32 = 1;

const MAGIC: &[u8] = b"distill-vectors\n";

/// The most bytes of the owner's text one vector is made from.
pub const CHUNK: usize = 2_000;
/// The most vectors of the owner's text one session has; a longer session's pieces are taken
/// evenly from start to end.
pub const MAX_CHUNKS: usize = 8;
/// How many sessions search by meaning adds to a search's results at most.
pub const MEANING_TOP: usize = 20;

/// One session's entry in the file's header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    key: String,
    made_from: MadeFrom,
    /// Per vector, the event its text starts at: an owner's message, or None for the card's
    /// summary.
    events: Vec<Option<u64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Header {
    version: u32,
    model: String,
    /// The length of every vector; 0 while there are none.
    dims: usize,
    sessions: Vec<Entry>,
}

/// One session's vectors.
#[derive(Debug, Clone, PartialEq)]
struct Session {
    made_from: MadeFrom,
    events: Vec<Option<u64>>,
    /// `events.len()` vectors of `dims` each, one after another.
    vectors: Vec<f32>,
}

/// The file, in memory.
#[derive(Debug, Clone, PartialEq)]
struct Store {
    model: String,
    dims: usize,
    sessions: BTreeMap<String, Session>,
}

impl Store {
    fn empty(model: &str) -> Self {
        Self {
            model: model.to_string(),
            dims: 0,
            sessions: BTreeMap::new(),
        }
    }

    fn vectors(&self) -> u64 {
        self.sessions.values().map(|s| s.events.len() as u64).sum()
    }
}

/// The file, or None when there is none or it can't be read.
fn load(home: &Home) -> Option<Store> {
    let bytes = std::fs::read(home.vectors_path()).ok()?;
    decode(&bytes).ok()
}

fn decode(bytes: &[u8]) -> Result<Store> {
    let rest = bytes.strip_prefix(MAGIC).context("not a vectors file")?;
    let (len, rest) = rest.split_at_checked(4).context("no header")?;
    let len = u32::from_le_bytes(len.try_into()?) as usize;
    let (header, mut data) = rest.split_at_checked(len).context("a short header")?;
    let header: Header = serde_json::from_slice(header)?;
    if header.version != VECTORS_VERSION {
        bail!("version {}", header.version);
    }
    let mut sessions = BTreeMap::new();
    for e in header.sessions {
        let n = e.events.len() * header.dims * 4;
        let (vs, rest) = data.split_at_checked(n).context("short vectors")?;
        data = rest;
        let vectors = vs
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        sessions.insert(
            e.key,
            Session {
                made_from: e.made_from,
                events: e.events,
                vectors,
            },
        );
    }
    if !data.is_empty() {
        bail!("bytes after the last vector");
    }
    Ok(Store {
        model: header.model,
        dims: header.dims,
        sessions,
    })
}

fn encode(store: &Store) -> Result<Vec<u8>> {
    let header = Header {
        version: VECTORS_VERSION,
        model: store.model.clone(),
        dims: store.dims,
        sessions: store
            .sessions
            .iter()
            .map(|(k, s)| Entry {
                key: k.clone(),
                made_from: s.made_from.clone(),
                events: s.events.clone(),
            })
            .collect(),
    };
    let header = serde_json::to_vec(&header)?;
    let floats: usize = store.sessions.values().map(|s| s.vectors.len()).sum();
    let mut out = Vec::with_capacity(MAGIC.len() + 4 + header.len() + floats * 4);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&u32::try_from(header.len())?.to_le_bytes());
    out.extend_from_slice(&header);
    for s in store.sessions.values() {
        for f in &s.vectors {
            out.extend_from_slice(&f.to_le_bytes());
        }
    }
    Ok(out)
}

/// `v` scaled to unit length; a vector of zeros stays zeros.
fn unit(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

/// The texts embedded for a session, each with the event it starts at (None for the card's
/// summary): what the owner wrote, in pieces of at most [`CHUNK`] bytes, at most [`MAX_CHUNKS`]
/// of them, then the card's summary.
fn texts_of(events: &[Event], card: Option<&Card>) -> Vec<(Option<u64>, String)> {
    let mut chunks: Vec<(Option<u64>, String)> = Vec::new();
    let mut open: Option<(u64, String)> = None;
    for e in events.iter().filter(|e| e.kind == EventKind::User) {
        let Some(text) = e.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) else {
            continue;
        };
        for piece in pieces(text) {
            match &mut open {
                Some((_, c)) if c.len() + 2 + piece.len() <= CHUNK => {
                    c.push_str("\n\n");
                    c.push_str(piece);
                }
                _ => {
                    if let Some((seq, c)) = open.take() {
                        chunks.push((Some(seq), c));
                    }
                    open = Some((e.seq, piece.to_string()));
                }
            }
        }
    }
    if let Some((seq, c)) = open {
        chunks.push((Some(seq), c));
    }
    if chunks.len() > MAX_CHUNKS {
        let n = chunks.len();
        chunks = (0..MAX_CHUNKS)
            .map(|i| chunks[i * n / MAX_CHUNKS].clone())
            .collect();
    }
    if let Some(c) = card {
        chunks.push((None, c.summary.clone()));
    }
    chunks
}

/// `text` in pieces of at most [`CHUNK`] bytes, cut at whitespace where there is some.
fn pieces(mut text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    while text.len() > CHUNK {
        let mut at = CHUNK;
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        if let Some(i) = text[..at].rfind(char::is_whitespace).filter(|&i| i > 0) {
            at = i;
        }
        out.push(text[..at].trim_end());
        text = text[at..].trim_start();
    }
    if !text.is_empty() {
        out.push(text);
    }
    out
}

/// What a run of the vectors step did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VectorReport {
    /// The embedding model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Why the step did not run, when it didn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    /// Sessions that get vectors.
    pub sessions: u64,
    /// Of those, sessions whose vectors were already current.
    pub current: u64,
    /// Sessions waiting for vectors before this run.
    pub pending: u64,
    /// Sessions embedded this run.
    pub embedded: u64,
    /// Sessions that got no vectors this run: the read was refused or the model failed.
    pub failed: u64,
    /// Sessions whose vectors were dropped because they are gone or get none.
    pub removed: u64,
    /// Vectors in the file after the run.
    pub vectors: u64,
    /// The tick's deadline passed with sessions left, which wait for the next tick.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopped_at_deadline: bool,
    /// Each failure: the session and what went wrong. Never session text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl VectorReport {
    pub fn summary(&self) -> String {
        if let Some(why) = &self.skipped {
            return format!("vectors skipped: {why}");
        }
        let mut s = format!(
            "embedded {} ({} already current, {} waiting), {}",
            count(self.embedded, "session"),
            self.current,
            self.pending.saturating_sub(self.embedded),
            count(self.vectors, "vector")
        );
        if self.removed > 0 {
            s += &format!(", {} removed", self.removed);
        }
        if self.failed > 0 {
            s += &format!(", {} failed", self.failed);
        }
        if self.stopped_at_deadline {
            s += ", stopped at the tick's time limit";
        }
        s
    }
}

/// The embedding model, or why it can't be used now: no `embedding_model`, or no local runtime
/// that can run it.
fn usable_model(config: &Config, runtime: &LocalRuntime) -> std::result::Result<String, String> {
    let Some(model) = config.steward.embedding_model.clone() else {
        return Err("no embedding_model is set".into());
    };
    match runtime.state() {
        RuntimeState::NotInstalled => Err("no local runtime is installed".into()),
        RuntimeState::NotRunning => Err("the local runtime is not running".into()),
        RuntimeState::Running(_) if !runtime.stored_here(&model) => Err(format!(
            "embedding_model \"{model}\" is not stored on this machine"
        )),
        RuntimeState::Running(_) => Ok(model),
    }
}

/// Embed the sessions whose vectors are missing or out of date, newest first, up to
/// `sessions_per_run`, reading each through the read path for the embedding model. Skipped, with
/// the reason in the report, when `embedding_model` is not set or no local runtime can run it.
/// Takes no session once `deadline` has passed. One writer at a time.
pub fn build(
    home: &Home,
    config: &Config,
    runtime: &LocalRuntime,
    deadline: Deadline,
) -> Result<VectorReport> {
    let model = match usable_model(config, runtime) {
        Ok(m) => m,
        Err(why) => {
            return Ok(VectorReport {
                model: config.steward.embedding_model.clone(),
                skipped: Some(why),
                ..Default::default()
            });
        }
    };
    let _lock = home.lock("vectors")?;
    let all = normalize::read_session_list(home)?;
    let originals = Originals::of(&all);
    let key = FingerprintKey::existing(home).ok().flatten();
    let mut store = load(home)
        .filter(|s| s.model == model)
        .unwrap_or_else(|| Store::empty(&model));
    let mut changed = !home.vectors_path().exists();
    let mut r = VectorReport {
        model: Some(model.clone()),
        ..Default::default()
    };

    let wanted: Vec<&SessionMeta> = all
        .iter()
        .filter(|f| cards::wanted(f, &originals))
        .collect();
    let keys: BTreeSet<String> = wanted.iter().map(|f| f.key()).collect();
    let before = store.sessions.len();
    store.sessions.retain(|k, _| keys.contains(k));
    r.removed = (before - store.sessions.len()) as u64;
    changed |= r.removed > 0;

    let mut pending: Vec<&SessionMeta> = Vec::new();
    for facts in wanted {
        r.sessions += 1;
        let now = super::MadeFrom::now(home, config, key.as_ref(), facts);
        let stored = store.sessions.get(&facts.key()).map(|s| &s.made_from);
        if now.is_some() && now.as_ref() == stored {
            r.current += 1;
        } else {
            pending.push(facts);
        }
    }
    r.pending = pending.len() as u64;
    pending.sort_by(|a, b| {
        started(b)
            .cmp(&started(a))
            .then_with(|| a.key().cmp(&b.key()))
    });
    pending.truncate(config.steward.sessions_per_run);
    let pending: Vec<SessionMeta> = pending.into_iter().cloned().collect();

    r.stopped_at_deadline =
        read::for_embedding(home, &model, &pending, deadline, |facts, served| {
            let (events, found, card) = match served {
                Ok(s) => s,
                Err(why) => {
                    changed |= store.sessions.remove(&facts.key()).is_some();
                    r.failed += 1;
                    r.errors.push(why);
                    return Ok(ControlFlow::Continue(()));
                }
            };
            let texts = texts_of(&events, card.as_ref());
            let made_from = MadeFrom::of(facts, found, card.as_ref());
            match embed(runtime, &model, &texts, store.dims) {
                Ok(vectors) => {
                    if store.dims == 0 && !vectors.is_empty() {
                        store.dims = vectors.len() / texts.len();
                    }
                    store.sessions.insert(
                        facts.key(),
                        Session {
                            made_from,
                            events: texts.into_iter().map(|(e, _)| e).collect(),
                            vectors,
                        },
                    );
                    changed = true;
                    r.embedded += 1;
                    Ok(ControlFlow::Continue(()))
                }
                // With the runtime gone, every other session would fail the same way, and none was
                // sent.
                Err(e @ (RuntimeError::NotRunning | RuntimeError::NotStoredHere(_))) => {
                    r.failed += 1;
                    r.errors
                        .push(format!("{}: the model failed: {e}", facts.key()));
                    Ok(ControlFlow::Break(()))
                }
                Err(e) => {
                    r.failed += 1;
                    r.errors
                        .push(format!("{}: the model failed: {e}", facts.key()));
                    Ok(ControlFlow::Continue(()))
                }
            }
        })?;
    if changed {
        write_atomic(&home.vectors_path(), &encode(&store)?)?;
    }
    r.vectors = store.vectors();
    Ok(r)
}

/// Each text's vector, of unit length, one after another; every one `dims` long when `dims` is
/// set.
fn embed(
    runtime: &LocalRuntime,
    model: &str,
    texts: &[(Option<u64>, String)],
    dims: usize,
) -> std::result::Result<Vec<f32>, RuntimeError> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let refs: Vec<&str> = texts.iter().map(|(_, t)| t.as_str()).collect();
    let vectors = runtime.embed(model, &refs)?;
    if vectors.len() != texts.len() {
        return Err(RuntimeError::Failed(format!(
            "{} vectors for {} texts",
            vectors.len(),
            texts.len()
        )));
    }
    let want = if dims == 0 { vectors[0].len() } else { dims };
    if want == 0 || vectors.iter().any(|v| v.len() != want) {
        return Err(RuntimeError::Failed(format!(
            "vectors of another length than the file's {want}"
        )));
    }
    Ok(vectors.into_iter().flat_map(unit).collect())
}

fn started(f: &SessionMeta) -> Option<&str> {
    f.started_at.as_deref().or(f.ended_at.as_deref())
}

/// A session close in meaning to a query.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeaningMatch {
    pub key: String,
    /// The cosine of its closest vector; higher is better.
    pub score: f32,
    /// The owner's message that closest vector starts at, when it is not the card's.
    pub event: Option<u64>,
}

/// What search by meaning found, or why it was not used.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Nearest {
    Found {
        /// Best first, at most [`MEANING_TOP`], each closer than unrelated.
        matches: Vec<MeaningMatch>,
        /// Of `among`, how many sessions have no vectors yet.
        unembedded: u64,
    },
    Skipped(String),
}

/// The sessions among `among` (keys) closest in meaning to `query`, best first. Embeds the query
/// with `embedding_model` through the local client, unless it holds a secret; reads the vectors
/// file, never a session.
pub(crate) fn nearest(
    home: &Home,
    config: &Config,
    runtime: &LocalRuntime,
    query: &str,
    among: &BTreeMap<String, Option<&str>>,
) -> Nearest {
    let model = match usable_model(config, runtime) {
        Ok(m) => m,
        Err(why) => return Nearest::Skipped(why),
    };
    let Some(store) = load(home).filter(|s| s.model == model) else {
        return Nearest::Skipped(format!("no vectors have been made with \"{model}\" yet"));
    };
    if holds_secret(home, config, query) {
        return Nearest::Skipped("the query holds a secret, which is never embedded".into());
    }
    let q = match runtime.embed(&model, &[query]) {
        Ok(mut v) if v.len() == 1 => unit(v.remove(0)),
        Ok(_) => return Nearest::Skipped("the embedding model gave no vector".into()),
        Err(e) => return Nearest::Skipped(format!("the embedding model failed: {e}")),
    };
    if q.len() != store.dims {
        return Nearest::Skipped(format!(
            "the query's vector is not the length of the stored ones ({} and {})",
            q.len(),
            store.dims
        ));
    }
    let mut matches: Vec<(MeaningMatch, Option<&str>)> = Vec::new();
    let mut unembedded = 0;
    for (key, started) in among {
        let Some(s) = store.sessions.get(key) else {
            unembedded += 1;
            continue;
        };
        let best = s
            .vectors
            .chunks_exact(store.dims.max(1))
            .zip(&s.events)
            .map(|(v, e)| (v.iter().zip(&q).map(|(a, b)| a * b).sum::<f32>(), *e))
            .max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((score, event)) = best.filter(|b| b.0 > 0.0) {
            matches.push((
                MeaningMatch {
                    key: key.clone(),
                    score,
                    event,
                },
                *started,
            ));
        }
    }
    // Closest first; among equals, newest first.
    matches.sort_by(|a, b| {
        b.0.score
            .total_cmp(&a.0.score)
            .then(b.1.cmp(&a.1))
            .then(a.0.key.cmp(&b.0.key))
    });
    matches.truncate(MEANING_TOP);
    Nearest::Found {
        matches: matches.into_iter().map(|(m, _)| m).collect(),
        unembedded,
    }
}

/// Whether `query` holds a secret: one the detector finds, or a word whose fingerprint is that of
/// a secret in the index.
fn holds_secret(home: &Home, config: &Config, query: &str) -> bool {
    let detector = Detector::new(&findings::owner(config));
    if detector
        .detect(query)
        .iter()
        .any(|h| h.kind.class() == Class::Secret)
    {
        return true;
    }
    let Some(key) = FingerprintKey::existing(home).ok().flatten() else {
        return false;
    };
    let mut words: BTreeSet<&str> = BTreeSet::new();
    for w in query.split_whitespace() {
        words.insert(w);
        words.extend(w.split(['=', ':', '"', '\'', '`', ',', ';', '(', ')']));
    }
    words.into_iter().filter(|w| w.len() >= 8).any(|w| {
        let fp = key.fingerprint(w).short().to_string();
        super::lookup(home, &[fp]).is_ok_and(|l| !l.matches.is_empty())
    })
}

/// How far the vectors have got, for status.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VectorProgress {
    /// The config's `embedding_model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Why the vectors step is skipped: no `embedding_model`, or why the last tick skipped it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    /// Sessions that get vectors.
    pub sessions: u64,
    /// Sessions whose vectors are current.
    pub current: u64,
    /// Sessions whose vectors are missing or out of date.
    pub pending: u64,
    /// Vectors in the file.
    pub vectors: u64,
    /// Bytes the file takes on disk.
    pub bytes: u64,
    /// How many sessions the last run did, when it stopped at the tick's deadline with sessions
    /// left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_after: Option<u64>,
}

/// Vector progress over the session list. Changes nothing and contacts no runtime: whether the
/// step could run is taken from the last tick that ran it.
pub fn progress(home: &Home) -> Result<VectorProgress> {
    let config = home.load_config().unwrap_or_default();
    let all = normalize::read_session_list(home)?;
    let originals = Originals::of(&all);
    let key = FingerprintKey::existing(home).ok().flatten();
    let store = config
        .steward
        .embedding_model
        .as_deref()
        .and_then(|m| load(home).filter(|s| s.model == m));
    let mut p = VectorProgress {
        model: config.steward.embedding_model.clone(),
        vectors: store.as_ref().map_or(0, Store::vectors),
        bytes: std::fs::metadata(home.vectors_path()).map_or(0, |m| m.len()),
        ..Default::default()
    };
    for facts in all.iter().filter(|f| cards::wanted(f, &originals)) {
        p.sessions += 1;
        let stored = store
            .as_ref()
            .and_then(|s| s.sessions.get(&facts.key()))
            .map(|s| &s.made_from);
        let current = stored.is_some()
            && super::MadeFrom::now(home, &config, key.as_ref(), facts).as_ref() == stored;
        if current {
            p.current += 1;
        } else {
            p.pending += 1;
        }
    }
    let last = crate::runlog::recent(home, 500)?
        .into_iter()
        .find_map(|e| e.outcome("vectors"))
        .and_then(|o| serde_json::from_value::<VectorReport>(o.report).ok());
    p.skipped = match &config.steward.embedding_model {
        None => Some("no embedding_model is set".into()),
        Some(_) => last.as_ref().and_then(|r| r.skipped.clone()),
    };
    p.stopped_after = last
        .filter(|r| r.stopped_at_deadline)
        .map(|r| r.embedded + r.failed);
    Ok(p)
}

#[cfg(test)]
mod tests;
