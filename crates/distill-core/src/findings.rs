//! Findings: what the detector found in a session, where, and each value's fingerprint, stored
//! beside the session in `findings.json` without changing its text (issue #72, "Findings"; ADR
//! 0002). The detector itself is `crate::detect`; this module places its hits in a session's
//! events, keeps the file current, and owns the fingerprint key.
//!
//! - A finding's place is an event, a field ([`Field`]: the event's text, or a path of positions
//!   into a tool call's input, to a value or the key holding it) and a byte range of that decoded
//!   string, key or number as written. A place never holds any of the session's text: a key can
//!   be a secret itself.
//! - A finding never holds the value: only its kind and its [`Fingerprint`], a keyed hash.
//! - The file records what it was made from ([`MadeFrom`]). When any of it differs from the
//!   current one, the findings are out of date and the session is scanned again.
//!
//! Like the parsers, the scan reads stored text directly; nothing else outside the read path
//! does.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;

use crate::config::Config;
use crate::detect::{self, Class, DETECTOR_VERSION, Detector, Kind, Owner};
use crate::home::{self, Home};
use crate::model::{Event, SessionMeta};
use crate::normalize;
use crate::util::{create_dir_private, private_file, set_private, sync_dir, write_json_pretty};

/// A keyed hash of a finding's value: the same wherever that value appears, and the value cannot
/// be recovered from it. Stored whole (64 hex characters); readers are shown [`short`](Self::short).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Fingerprint(String);

/// How many hex characters of a fingerprint readers are shown.
pub const SHORT_LEN: usize = 8;

impl Fingerprint {
    /// The eight hex characters a placeholder shows.
    pub fn short(&self) -> &str {
        &self.0[..SHORT_LEN.min(self.0.len())]
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Whether this fingerprint starts with `prefix` (a short form, or any part of it), ignoring
    /// case.
    pub fn matches(&self, prefix: &str) -> bool {
        !prefix.is_empty() && self.0.starts_with(&prefix.to_ascii_lowercase())
    }
}

/// The key fingerprints are made with. Generated once, stored at `keys/fingerprint.key`, readable
/// by the owner only. Losing it changes every fingerprint at the next scan.
pub struct FingerprintKey([u8; 32]);

impl FingerprintKey {
    /// The home's key, created on first use.
    pub fn open(home: &Home) -> Result<Self> {
        if let Some(k) = Self::existing(home)? {
            return Ok(k);
        }
        create_key(home)?;
        Self::existing(home)?.context("the fingerprint key was not created")
    }

    /// The home's key, if it has one yet.
    pub fn existing(home: &Home) -> Result<Option<Self>> {
        let path = home.fingerprint_key_path();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        let bytes = hex::decode(text.trim())
            .ok()
            .and_then(|b| b.try_into().ok());
        match bytes {
            Some(b) => Ok(Some(Self(b))),
            None => bail!("{} is not a fingerprint key", path.display()),
        }
    }

    /// The fingerprint of `value`.
    pub fn fingerprint(&self, value: &str) -> Fingerprint {
        Fingerprint(hex::encode(self.mac(value.as_bytes())))
    }

    /// Names this key without revealing it, so findings made with another key are out of date.
    pub fn id(&self) -> String {
        hex::encode(self.mac(b"\0distill fingerprint key id"))[..16].to_string()
    }

    fn mac(&self, bytes: &[u8]) -> [u8; 32] {
        let mut m = Hmac::<Sha256>::new_from_slice(&self.0).expect("HMAC takes any key length");
        m.update(bytes);
        m.finalize().into_bytes().into()
    }
}

/// Write a new key, unless another writer got there first. The key is written whole to a
/// temporary file and linked into place, so no reader ever sees half a key and no key is ever
/// replaced.
fn create_key(home: &Home) -> Result<()> {
    let dir = home.keys_dir();
    create_dir_private(&dir).with_context(|| format!("create {}", dir.display()))?;
    set_private(&home.root);
    set_private(&dir);
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| anyhow::anyhow!("generate a fingerprint key: {e}"))?;
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).map_err(|e| anyhow::anyhow!("generate a file name: {e}"))?;
    let tmp = dir.join(format!(".fingerprint.key.{}", hex::encode(nonce)));
    let written =
        write_private(&tmp, hex::encode(key).as_bytes()).and_then(|()| {
            match fs::hard_link(&tmp, home.fingerprint_key_path()) {
                Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => {
                    Err(e).context("put the fingerprint key in place")
                }
                _ => Ok(()),
            }
        });
    let _ = fs::remove_file(&tmp);
    written?;
    sync_dir(&dir)
}

/// Create `path`, readable by the owner only from the start, holding `bytes`.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = private_file()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

/// Which part of an event a finding is in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum Field {
    /// The event's text.
    Text,
    /// A tool call's input: the string or number at `path`, or, with `key`, the key that holds
    /// it. Each step of the path is the position of an array's item or an object's member, in
    /// the order the input iterates them ([`at`]).
    Input {
        path: Vec<usize>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        key: bool,
    },
}

/// What is at `path` in `input`: the value, and the key it is held under when the path ends at
/// an object's member.
pub fn at<'a>(input: &'a Value, path: &[usize]) -> Option<(Option<&'a str>, &'a Value)> {
    let mut here = (None, input);
    for &i in path {
        here = match here.1 {
            Value::Array(a) => (None, a.get(i)?),
            Value::Object(o) => o.iter().nth(i).map(|(k, v)| (Some(k.as_str()), v))?,
            _ => return None,
        };
    }
    Some(here)
}

/// A detected piece of sensitive text in a session: where it is, what kind it is, and its
/// fingerprint. Never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// The event, by sequence number.
    pub event: u64,
    #[serde(flatten)]
    pub field: Field,
    /// Bytes of the field's decoded string, key, or number as written: `start..end`.
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
    pub fingerprint: Fingerprint,
}

impl Finding {
    pub fn is_secret(&self) -> bool {
        self.kind.class() == Class::Secret
    }
    /// What a model reads in its place: `[secret: <kind> #<fingerprint>]`.
    pub fn placeholder(&self) -> String {
        detect::placeholder(self.kind, self.fingerprint.short())
    }
}

/// What a session's findings were made from. Findings made from anything else are out of date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MadeFrom {
    /// [`DETECTOR_VERSION`].
    pub detector: u32,
    /// The session's capture hash, with any attached captures' ([`RawRef::source`]).
    pub capture_hash: String,
    /// The version of the parser that wrote the session's events.
    pub parser: u32,
    /// The fingerprint key's [`id`](FingerprintKey::id).
    pub key: String,
    /// A hash of the config's `personal_terms`, with the owner's username, which the detector
    /// is told beside them ([`owner`]).
    pub personal_terms: String,
}

impl MadeFrom {
    /// What findings for `facts` would be made from now, with `config`.
    pub fn current(facts: &SessionMeta, key: &FingerprintKey, config: &Config) -> Self {
        Self {
            detector: DETECTOR_VERSION,
            capture_hash: facts.raw.source(),
            parser: facts.normalizer.version,
            key: key.id(),
            personal_terms: personal_terms_hash(&owner(config)),
        }
    }
}

/// What the detector is told about the owner: the config's `personal_terms`, and the owner's
/// username, the name of their home folder.
pub fn owner(config: &Config) -> Owner {
    let username = home::user_home()
        .ok()
        .and_then(|h| h.file_name()?.to_str().map(str::to_string));
    Owner {
        username,
        terms: config.privacy.personal_terms.clone(),
    }
}

/// Names what the detector was told about the owner without holding it: a findings file is not
/// a list of the owner's terms.
fn personal_terms_hash(owner: &Owner) -> String {
    use sha2::Digest;
    let mut h = Sha256::new();
    for part in owner.username.iter().chain(&owner.terms) {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    h.update([owner.username.is_some() as u8]);
    hex::encode(h.finalize())[..16].to_string()
}

/// A session's findings file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Findings {
    /// The session's key (`harness/id`).
    pub session: String,
    pub made_from: MadeFrom,
    /// In event order.
    pub findings: Vec<Finding>,
    /// What the detector found in the session's card, when it has one (`crate::cards`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<CardFindings>,
    /// What the model found in what the owner and the agent wrote (`crate::details`),
    /// once its pass has run. Kept under its own version, so that it and the pattern scan can
    /// each be rerun alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelFindings>,
}

/// The personal details a model found in a session, and what they were made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFindings {
    pub made_from: ModelMadeFrom,
    /// In event order. Each is in an event's text, and none overlaps a pattern finding.
    pub findings: Vec<Finding>,
}

/// What a session's model findings were made from. Their places fit the session's events only
/// while its capture hash and parser version are the same, and their fingerprints only while the
/// key is; the pattern scan keeps them only then. The pass runs again when any of it, the
/// [`MODEL_DETECTOR_VERSION`](crate::details::MODEL_DETECTOR_VERSION) or the model differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelMadeFrom {
    /// [`MODEL_DETECTOR_VERSION`](crate::details::MODEL_DETECTOR_VERSION).
    pub detector: u32,
    /// The model that found them, at the runtime's provider.
    pub model: String,
    pub capture_hash: String,
    pub parser: u32,
    /// The fingerprint key's [`id`](FingerprintKey::id).
    pub key: String,
}

impl ModelMadeFrom {
    /// Whether findings made from this fit the events and key that `pattern` was made from.
    pub(crate) fn fits(&self, pattern: &MadeFrom) -> bool {
        self.capture_hash == pattern.capture_hash
            && self.parser == pattern.parser
            && self.key == pattern.key
    }
}

/// The findings in a session's card, and which card they were made from. A card is scanned like
/// session text; one found to hold a secret is discarded and written again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardFindings {
    /// The [`hash`](crate::cards::Card::hash) of the card these were found in.
    pub hash: String,
    pub findings: Vec<CardFinding>,
}

/// A finding in a card. Its place is one of the card's fields (`title`, `summary`, or
/// `tags/<n>`) and a byte range of it. Never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardFinding {
    pub field: String,
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
    pub fingerprint: Fingerprint,
}

impl CardFinding {
    pub fn is_secret(&self) -> bool {
        self.kind.class() == Class::Secret
    }
}

/// What a card, an index entry or a set of vectors was built from: the session's events, named by
/// its capture hash and parser version, and its secret findings, which its placeholders were made
/// from. When either changes, what was built from them is out of date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltFrom {
    pub capture_hash: String,
    pub parser: u32,
    /// A digest of the session's secret findings.
    pub secrets: String,
}

impl BuiltFrom {
    /// What something built now from `facts`'s events, served with `findings`, is built from.
    pub fn of(facts: &SessionMeta, findings: &Findings) -> Self {
        let secrets: Vec<_> = findings.findings.iter().filter(|f| f.is_secret()).collect();
        Self {
            capture_hash: facts.raw.source(),
            parser: facts.normalizer.version,
            secrets: crate::util::sha256_hex(&serde_json::to_vec(&secrets).unwrap_or_default()),
        }
    }
}

/// Everything `detector` finds in the fields of a card, by field name, fingerprinted.
pub fn find_in_fields<'a>(
    fields: impl IntoIterator<Item = (String, &'a str)>,
    key: &FingerprintKey,
    detector: &Detector,
) -> Vec<CardFinding> {
    fields
        .into_iter()
        .flat_map(|(field, text)| {
            detector
                .detect(text)
                .into_iter()
                .map(move |h| CardFinding {
                    field: field.clone(),
                    start: h.range.start,
                    end: h.range.end,
                    kind: h.kind,
                    fingerprint: key.fingerprint(&h.value),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Record `card` as the findings of the session's card, keeping its other findings. The
/// session's findings must be stored already.
pub(crate) fn set_card(home: &Home, facts: &SessionMeta, card: CardFindings) -> Result<()> {
    let _lock = home.lock_findings(facts.harness, &facts.session_id)?;
    let mut f = load(home, facts).with_context(|| format!("{} has no findings", facts.key()))?;
    f.card = Some(card);
    write_json_pretty(
        &home.findings_path(facts.machine.as_deref(), facts.harness, &facts.session_id),
        &f,
    )
}

impl Findings {
    /// Whether these findings are current for `facts` with `key` and `config`. Only the pattern
    /// scan's count: a missing or out-of-date model section never holds up a read, because
    /// personal details are never masked.
    pub fn is_current(&self, facts: &SessionMeta, key: &FingerprintKey, config: &Config) -> bool {
        self.session == facts.key() && self.made_from == MadeFrom::current(facts, key, config)
    }
    /// The model's findings, when they fit the events the pattern findings were made from.
    pub fn model_findings(&self) -> &[Finding] {
        match &self.model {
            Some(m) if m.made_from.fits(&self.made_from) => &m.findings,
            _ => &[],
        }
    }
    /// Every finding: the pattern scan's, then the model's.
    pub fn all(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().chain(self.model_findings())
    }
    /// The findings in one event, the pattern scan's and the model's.
    pub fn in_event(&self, seq: u64) -> impl Iterator<Item = &Finding> {
        self.all().filter(move |f| f.event == seq)
    }
}

/// What the model's answers came to once placed in a session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Placed {
    /// Findings stored.
    pub findings: u64,
    /// Answers found nowhere in what the owner and the agent wrote, and dropped.
    pub unplaced: u64,
}

/// Record what `model` answered for a session as its model findings, keeping its other findings.
/// Each answer is a kind and the exact text the model said it found; it is placed wherever that
/// text is written in an event the owner or the agent wrote, as a whole word, and dropped when
/// it is nowhere. A place that touches a pattern finding is dropped: a secret's place, or a
/// personal detail the patterns found already. The session's pattern findings must be stored and
/// current with the events the model read.
pub(crate) fn set_model(
    home: &Home,
    config: &Config,
    facts: &SessionMeta,
    model: &str,
    answers: &[(Kind, String)],
) -> Result<Placed> {
    let key = FingerprintKey::open(home)?;
    let events = normalize::read_events(home, facts)?;
    let _lock = home.lock_findings(facts.harness, &facts.session_id)?;
    let mut f = load(home, facts)
        .filter(|f| f.is_current(facts, &key, config))
        .with_context(|| format!("{} changed while the model read it", facts.key()))?;
    let (findings, unplaced) = place_answers(&events, answers, &key, &f.findings);
    let placed = Placed {
        findings: findings.len() as u64,
        unplaced,
    };
    f.model = Some(ModelFindings {
        made_from: ModelMadeFrom {
            detector: crate::details::MODEL_DETECTOR_VERSION,
            model: model.to_string(),
            capture_hash: f.made_from.capture_hash.clone(),
            parser: f.made_from.parser,
            key: f.made_from.key.clone(),
        },
        findings,
    });
    write_json_pretty(
        &home.findings_path(facts.machine.as_deref(), facts.harness, &facts.session_id),
        &f,
    )?;
    Ok(placed)
}

/// Every place each answer is written in what the owner and the agent wrote, with no place
/// touching `taken` or another, and how many answers were placed nowhere. Of two that overlap,
/// the one that starts first is kept, and of two that start together the longer.
fn place_answers(
    events: &[Event],
    answers: &[(Kind, String)],
    key: &FingerprintKey,
    taken: &[Finding],
) -> (Vec<Finding>, u64) {
    use crate::model::EventKind;
    let mut found: Vec<Finding> = Vec::new();
    let mut unplaced = 0;
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for (kind, text) in answers {
        let text = text.trim();
        if text.is_empty() || !seen.insert(text) {
            continue;
        }
        let mut placed = false;
        let written = events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::User | EventKind::Assistant));
        for e in written {
            let Some(body) = e.text.as_deref() else {
                continue;
            };
            for (start, _) in body.match_indices(text) {
                let end = start + text.len();
                if !whole_word(body, start, end) {
                    continue;
                }
                placed = true;
                found.push(Finding {
                    event: e.seq,
                    field: Field::Text,
                    start,
                    end,
                    kind: *kind,
                    fingerprint: key.fingerprint(text),
                });
            }
        }
        unplaced += u64::from(!placed);
    }
    found.sort_by_key(|f| (f.event, f.start, std::cmp::Reverse(f.end)));
    let mut kept: Vec<Finding> = Vec::new();
    for f in found {
        let touches = |o: &Finding| {
            o.event == f.event && o.field == Field::Text && o.start < f.end && f.start < o.end
        };
        if !taken.iter().any(touches) && !kept.iter().any(touches) {
            kept.push(f);
        }
    }
    (kept, unplaced)
}

/// Whether `body[start..end]` is not part of a longer word: a letter or digit at either edge of
/// it has none beside it.
fn whole_word(body: &str, start: usize, end: usize) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let inner = &body[start..end];
    let before = body[..start].chars().next_back();
    let after = body[end..].chars().next();
    !(inner.chars().next().is_some_and(word) && before.is_some_and(word))
        && !(inner.chars().next_back().is_some_and(word) && after.is_some_and(word))
}

/// Everything `detector` finds in `events`, placed and fingerprinted.
pub fn find(events: &[Event], key: &FingerprintKey, detector: &Detector) -> Vec<Finding> {
    placed(events, key, detector).map(|(f, _)| f).collect()
}

/// The value of each secret in `events` whose fingerprint is in `wanted`, by fingerprint, with
/// its kind. Values come only from running the detector over the stored text again; nothing
/// stores them. Only a reveal the owner approved asks for them (`crate::reveal`).
pub(crate) fn values(
    events: &[Event],
    key: &FingerprintKey,
    detector: &Detector,
    wanted: &BTreeSet<Fingerprint>,
) -> std::collections::BTreeMap<Fingerprint, (Kind, String)> {
    placed(events, key, detector)
        .filter(|(f, _)| f.is_secret() && wanted.contains(&f.fingerprint))
        .map(|(f, value)| (f.fingerprint, (f.kind, value)))
        .collect()
}

/// Each finding in `events` with the value it was made from.
fn placed<'a>(
    events: &'a [Event],
    key: &'a FingerprintKey,
    detector: &'a Detector,
) -> impl Iterator<Item = (Finding, String)> + 'a {
    events.iter().flat_map(move |e| {
        let in_text = e.text.iter().flat_map(move |text| {
            detector.detect(text).into_iter().map(move |h| {
                let f = Finding {
                    event: e.seq,
                    field: Field::Text,
                    start: h.range.start,
                    end: h.range.end,
                    kind: h.kind,
                    fingerprint: key.fingerprint(&h.value),
                };
                (f, h.value)
            })
        });
        let in_input = e.input.iter().flat_map(move |input| {
            detector.detect_value(input).into_iter().map(move |v| {
                let f = Finding {
                    event: e.seq,
                    field: Field::Input {
                        path: v.path,
                        key: v.key,
                    },
                    start: v.hit.range.start,
                    end: v.hit.range.end,
                    kind: v.hit.kind,
                    fingerprint: key.fingerprint(&v.hit.value),
                };
                (f, v.hit.value)
            })
        });
        in_text.chain(in_input)
    })
}

/// A session's findings file as stored, current or not. A file that can't be read counts as
/// missing.
pub fn load(home: &Home, facts: &SessionMeta) -> Option<Findings> {
    let text = fs::read_to_string(home.findings_path(
        facts.machine.as_deref(),
        facts.harness,
        &facts.session_id,
    ))
    .ok()?;
    serde_json::from_str(&text).ok()
}

/// The session's findings, current with the detector: the stored ones when they are, otherwise
/// a new scan of `events`, stored before it is returned.
pub(crate) fn current_or_scan(
    home: &Home,
    config: &Config,
    facts: &SessionMeta,
    events: &[Event],
) -> Result<Findings> {
    let key = FingerprintKey::open(home)?;
    if let Some(f) = load(home, facts).filter(|f| f.is_current(facts, &key, config)) {
        return Ok(f);
    }
    let detector = Detector::new(&owner(config));
    scan(home, config, facts, events, &key, &detector)
}

/// Scan `events` and replace the session's findings file whole. The per-session lock is held
/// only for the write, so a read never waits on a tick's scan of other sessions.
fn scan(
    home: &Home,
    config: &Config,
    facts: &SessionMeta,
    events: &[Event],
    key: &FingerprintKey,
    detector: &Detector,
) -> Result<Findings> {
    let mut findings = Findings {
        session: facts.key(),
        made_from: MadeFrom::current(facts, key, config),
        findings: find(events, key, detector),
        // The card is scanned with the session; one that holds a secret is discarded.
        card: crate::cards::rescan(home, facts, key, detector),
        model: None,
    };
    let _lock = home.lock_findings(facts.harness, &facts.session_id)?;
    // The model's findings are kept while they still fit the session's events and key, so that
    // rescanning with new patterns or terms never makes the model pass run again. Any that now
    // touch a pattern finding are dropped: a model finding never overrides a secret's place.
    findings.model = load(home, facts)
        .and_then(|old| old.model)
        .filter(|m| m.made_from.fits(&findings.made_from))
        .map(|mut m| {
            let taken = &findings.findings;
            m.findings.retain(|f| {
                !taken.iter().any(|o| {
                    o.event == f.event && o.field == f.field && o.start < f.end && f.start < o.end
                })
            });
            m
        });
    write_json_pretty(
        &home.findings_path(facts.machine.as_deref(), facts.harness, &facts.session_id),
        &findings,
    )?;
    Ok(findings)
}

/// What a scan of the archive did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanReport {
    /// Sessions in the session list.
    pub sessions: u64,
    /// Sessions scanned now, because their findings were missing or out of date.
    pub scanned: u64,
    /// Sessions whose findings were already current.
    pub current: u64,
    pub failed: u64,
    /// Findings in the sessions scanned now, and of those, secrets.
    pub findings: u64,
    pub secrets: u64,
    /// Each failure: the session and what went wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl ScanReport {
    pub fn summary(&self) -> String {
        let mut s = format!(
            "scanned {} ({} already current), {} found, {} of them secrets",
            crate::util::count(self.scanned, "session"),
            self.current,
            crate::util::count(self.findings, "finding"),
            self.secrets
        );
        if self.failed > 0 {
            s += &format!(", {} failed", self.failed);
        }
        s
    }
}

/// Scan every session whose findings are missing or out of date with `config`, with no limit.
pub fn scan_all(home: &Home, config: &Config) -> Result<ScanReport> {
    let sessions = normalize::read_session_list(home)?;
    let mut r = ScanReport {
        sessions: sessions.len() as u64,
        ..Default::default()
    };
    if sessions.is_empty() {
        return Ok(r);
    }
    let key = FingerprintKey::open(home)?;
    let detector = Detector::new(&owner(config));
    for facts in &sessions {
        if load(home, facts).is_some_and(|f| f.is_current(facts, &key, config)) {
            r.current += 1;
            continue;
        }
        let scanned = normalize::read_events(home, facts)
            .and_then(|events| scan(home, config, facts, &events, &key, &detector));
        match scanned {
            Ok(f) => {
                r.scanned += 1;
                r.findings += f.findings.len() as u64;
                r.secrets += f.findings.iter().filter(|f| f.is_secret()).count() as u64;
            }
            Err(e) => {
                r.failed += 1;
                r.errors.push(format!("{}: {e:#}", facts.key()));
            }
        }
    }
    Ok(r)
}

/// How far the scan has got, for status.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanProgress {
    pub sessions: u64,
    /// Sessions whose findings are current.
    pub current: u64,
    /// Sessions whose findings are missing or out of date.
    pub pending: u64,
    /// Secrets in the current findings.
    pub secrets: u64,
    /// Personal details in the current findings: the patterns' and the model's.
    #[serde(default)]
    pub personal_details: u64,
}

/// Scan progress over the session list. Creates nothing: with no key yet, nothing is current.
/// With no config, or one that can't be read, the owner has no `personal_terms`.
pub fn progress(home: &Home) -> Result<ScanProgress> {
    let config = home.load_config().unwrap_or_default();
    let sessions = normalize::read_session_list(home)?;
    let key = FingerprintKey::existing(home).ok().flatten();
    let mut p = ScanProgress {
        sessions: sessions.len() as u64,
        ..Default::default()
    };
    for facts in &sessions {
        match (&key, load(home, facts)) {
            (Some(k), Some(f)) if f.is_current(facts, k, &config) => {
                p.current += 1;
                p.secrets += f.findings.iter().filter(|f| f.is_secret()).count() as u64;
                p.personal_details += f.all().filter(|f| !f.is_secret()).count() as u64;
            }
            _ => p.pending += 1,
        }
    }
    Ok(p)
}

/// Every stored fingerprint that starts with `prefix`, such as a placeholder's short form. Two
/// values can share a short form, so a lookup can return more than one.
pub fn lookup(home: &Home, prefix: &str) -> Result<BTreeSet<Fingerprint>> {
    Ok(normalize::read_session_list(home)?
        .iter()
        .filter_map(|facts| load(home, facts))
        .flat_map(|f| f.all().map(|f| f.fingerprint.clone()).collect::<Vec<_>>())
        .filter(|fp| fp.matches(prefix))
        .collect())
}

#[cfg(test)]
mod tests;
