//! Findings: what the detector found in a session, where, and each value's fingerprint, stored
//! beside the session in `findings.json` without changing its text (issue #72, "Findings"; ADR
//! 0002). The detector itself is `crate::detect`; this module places its hits in a session's
//! events, keeps the file current, and owns the fingerprint key.
//!
//! - A finding's place is an event, a field ([`Field`]: the event's text, or a JSON pointer into
//!   a tool call's input, to a value or the key holding it) and a byte range of that decoded
//!   string, key or number as written.
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
use sha2::Sha256;

use crate::config::Config;
use crate::detect::{self, Class, DETECTOR_VERSION, Detector, Kind, Owner};
use crate::home::{self, Home};
use crate::model::{Event, SessionMeta};
use crate::normalize;
use crate::util::{set_private, sync_dir, write_json_pretty};

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
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
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
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts
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
    /// A tool call's input: the string or number at `pointer` (RFC 6901), or, with `key`, the
    /// key that holds it.
    Input {
        pointer: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        key: bool,
    },
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
    /// The session's capture hash.
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
            capture_hash: facts.raw.sha256.clone(),
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
}

impl Findings {
    /// Whether these findings are current for `facts` with `key` and `config`.
    pub fn is_current(&self, facts: &SessionMeta, key: &FingerprintKey, config: &Config) -> bool {
        self.session == facts.key() && self.made_from == MadeFrom::current(facts, key, config)
    }
    /// The findings in one event.
    pub fn in_event(&self, seq: u64) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(move |f| f.event == seq)
    }
}

/// Everything `detector` finds in `events`, placed and fingerprinted.
pub fn find(events: &[Event], key: &FingerprintKey, detector: &Detector) -> Vec<Finding> {
    let mut out = Vec::new();
    for e in events {
        if let Some(text) = &e.text {
            out.extend(detector.detect(text).into_iter().map(|h| Finding {
                event: e.seq,
                field: Field::Text,
                start: h.range.start,
                end: h.range.end,
                kind: h.kind,
                fingerprint: key.fingerprint(&h.value),
            }));
        }
        if let Some(input) = &e.input {
            out.extend(detector.detect_value(input).into_iter().map(|v| Finding {
                event: e.seq,
                field: Field::Input {
                    pointer: v.pointer,
                    key: v.key,
                },
                start: v.hit.range.start,
                end: v.hit.range.end,
                kind: v.hit.kind,
                fingerprint: key.fingerprint(&v.hit.value),
            }));
        }
    }
    out
}

/// A session's findings file as stored, current or not. A file that can't be read counts as
/// missing.
pub fn load(home: &Home, facts: &SessionMeta) -> Option<Findings> {
    let text = fs::read_to_string(home.findings_path(facts.harness, &facts.session_id)).ok()?;
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
    let findings = Findings {
        session: facts.key(),
        made_from: MadeFrom::current(facts, key, config),
        findings: find(events, key, detector),
    };
    let _lock = home.lock_findings(facts.harness, &facts.session_id)?;
    write_json_pretty(
        &home.findings_path(facts.harness, &facts.session_id),
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
        .flat_map(|f| f.findings)
        .map(|f| f.fingerprint)
        .filter(|fp| fp.matches(prefix))
        .collect())
}

#[cfg(test)]
mod tests;
