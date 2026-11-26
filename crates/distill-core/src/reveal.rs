//! Reveals: a model at a trusted provider asks for the real value of specific secrets, the
//! owner approves or denies the request in the GUI, and an approval releases those values once
//! (issue #72, "Reveals"; ADR 0004).
//!
//! - [`request`] files a request naming the reader, the session, specific fingerprints and a
//!   reason, and returns its ID. It never returns a value.
//! - [`approve`] and [`deny`] are for the GUI alone. No command and no MCP tool calls them: an
//!   agent with a shell could otherwise approve its own request.
//! - [`collect`] gives the values of an approved request to the reader that filed it, once.
//! - A request expires [`LIFETIME_HOURS`] after it was made, approved or not: an unanswered one
//!   can no longer be approved, and an approved one can no longer be collected.
//!
//! Each request is one file under `reveals/`. Every step (request, approval, denial, collection,
//! expiry, refusal) is a line in the read log. Neither ever holds a value: values come only from
//! running the detector over the stored text again, at collection.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::detect::{self, Kind};
use crate::findings::{Fingerprint, SHORT_LEN};
use crate::home::Home;
use crate::read;
use crate::readlog::{self, ReadEntry, ReadResult};
use crate::runtime::LocalRuntime;
use crate::trust::{Access, Reader, Trust};
use crate::util::{create_dir_private, write_json_pretty};

/// How long a request lives after it was made, whether or not it was approved.
pub const LIFETIME_HOURS: i64 = 24;

/// The longest reason a request may give.
pub const REASON_MAX: usize = 1000;

/// One reveal request, as its file under `reveals/` holds it. It names secrets by fingerprint
/// and never holds a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// What the reader collects with: sixteen hex characters.
    pub id: String,
    /// When it was made, and when it expires.
    pub made_at: String,
    pub expires_at: String,
    /// Who asked. Only this reader may collect.
    #[serde(flatten)]
    pub reader: Reader,
    /// The session's key (`harness/id`).
    pub session: String,
    /// The secrets asked for.
    pub secrets: Vec<Secret>,
    /// Why, in the reader's words.
    pub reason: String,
    #[serde(flatten)]
    pub state: State,
}

/// A secret a request names: its fingerprint and kind.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Secret {
    pub fingerprint: Fingerprint,
    pub kind: Kind,
}

impl Secret {
    /// What a model reads in its place: `[secret: <kind> #<fingerprint>]`.
    pub fn placeholder(&self) -> String {
        detect::placeholder(self.kind, self.fingerprint.short())
    }
}

/// Where a request stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum State {
    /// Waiting for the owner.
    Pending,
    /// The owner approved it; the reader has not collected yet.
    Approved { approved_at: String },
    /// The owner denied it.
    Denied { denied_at: String },
    /// The reader collected the values. An approval releases them once.
    Collected {
        approved_at: String,
        collected_at: String,
    },
    /// It passed its lifetime before it was answered or collected.
    Expired { expired_at: String },
}

impl State {
    fn name(&self) -> &'static str {
        match self {
            State::Pending => "pending",
            State::Approved { .. } => "approved",
            State::Denied { .. } => "denied",
            State::Collected { .. } => "collected",
            State::Expired { .. } => "expired",
        }
    }
}

impl Request {
    /// What the reader is told once its request is filed: the ID, what was asked for, and that
    /// nothing is released before the owner approves it.
    pub fn receipt(&self) -> String {
        let asked: Vec<String> = self.secrets.iter().map(Secret::placeholder).collect();
        format!(
            "Reveal request {} is filed for {} in session {}: {}. Nothing is released until the \
             owner approves it in DISTILL's window. Once it is approved, collect the values \
             once with this request ID, before {}, when it expires.",
            self.id,
            crate::util::count(self.secrets.len() as u64, "secret"),
            self.session,
            asked.join(", "),
            self.expires_at
        )
    }

    fn expires(&self) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&self.expires_at)
            .ok()
            .map(|t| t.with_timezone(&Utc))
    }

    /// Whether it has passed its lifetime at `now`. A request whose expiry can't be read has.
    fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.expires().is_none_or(|t| t <= now)
    }
}

/// What filing a request came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filed {
    Filed(Box<Request>),
    /// Nothing was filed, and why.
    Refused(String),
}

/// What collecting came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Collected {
    Released(Released),
    /// Nothing was released, and why.
    Refused(String),
}

/// The values an approved request released, once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Released {
    pub request: String,
    pub session: String,
    pub values: Vec<Revealed>,
    /// Secrets the request named that the session no longer holds.
    pub missing: Vec<Secret>,
}

/// One secret's value. Its `Debug` leaves the value out.
#[derive(Clone, PartialEq, Eq)]
pub struct Revealed {
    pub secret: Secret,
    pub value: String,
}

impl std::fmt::Debug for Revealed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Revealed")
            .field("secret", &self.secret)
            .field("value", &format_args!("<{} bytes>", self.value.len()))
            .finish()
    }
}

impl Released {
    /// The values as text for the reader: one line per secret, its placeholder and its value.
    pub fn text(&self) -> String {
        let mut t = format!(
            "Reveal {}: the owner approved releasing these values from session {} once. They \
             are not given out again.\n",
            self.request, self.session
        );
        for v in &self.values {
            t += &format!("{} = {}\n", v.secret.placeholder(), v.value);
        }
        for s in &self.missing {
            t += &format!("{} is no longer in the session.\n", s.placeholder());
        }
        t
    }
}

/// File a request as `reader` for the secrets in `session` (its key, or its id alone when only
/// one harness has it) with these `fingerprints` (each a placeholder's short form, with or
/// without `#`, or a whole placeholder), and why. Only a model at a trusted provider may.
/// Returns the request, which holds its ID and no value. Every call is a line in the read log.
pub fn request(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    session: &str,
    fingerprints: &[String],
    reason: &str,
) -> Result<Filed> {
    request_at(
        home,
        runtime,
        reader,
        session,
        fingerprints,
        reason,
        Utc::now(),
    )
}

fn request_at(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    session: &str,
    fingerprints: &[String],
    reason: &str,
    now: DateTime<Utc>,
) -> Result<Filed> {
    let mut logged = loggable(session);
    let filed = file(
        home,
        runtime,
        reader,
        session,
        fingerprints,
        reason,
        now,
        &mut logged,
    );
    let result = match &filed {
        Ok(Filed::Filed(r)) => ReadResult::RevealRequested {
            request: r.id.clone(),
            secrets: r
                .secrets
                .iter()
                .map(|s| s.fingerprint.short().to_string())
                .collect(),
        },
        Ok(Filed::Refused(why)) => ReadResult::Refused {
            reason: why.clone(),
        },
        Err(e) => ReadResult::Failed {
            error: format!("{e:#}"),
        },
    };
    if let Err(e) = log(home, Some(runtime), reader, Some(logged), result, now) {
        // A request that could not be logged is not filed.
        if let Ok(Filed::Filed(r)) = &filed {
            let _ = std::fs::remove_file(home.reveal_path(&r.id));
        }
        return Err(e);
    }
    filed
}

#[allow(clippy::too_many_arguments)]
fn file(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    session: &str,
    fingerprints: &[String],
    reason: &str,
    now: DateTime<Utc>,
    logged: &mut String,
) -> Result<Filed> {
    let refuse = |why: &str| Ok(Filed::Refused(why.to_string()));
    if !matches!(reader, Reader::Model { .. }) {
        return refuse(
            "only a model files a reveal request; the owner sees every value in DISTILL's window.",
        );
    }
    if let Access::Refused(why) = trust(home).check(reader, runtime) {
        return Ok(Filed::Refused(why));
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return refuse("give a reason: the owner reads it before approving or denying.");
    }
    if reason.chars().count() > REASON_MAX {
        return Ok(Filed::Refused(format!(
            "the reason is longer than {REASON_MAX} characters."
        )));
    }
    if holds_secret(reason) {
        return refuse("the reason looks like it holds a secret; describe the task without it.");
    }
    if fingerprints.is_empty() {
        return refuse("name at least one secret by the fingerprint its placeholder shows.");
    }
    let mut prefixes = Vec::new();
    for (i, f) in fingerprints.iter().enumerate() {
        match prefix(f) {
            Some(p) => prefixes.push(p),
            None => {
                return Ok(Filed::Refused(format!(
                    "fingerprint {} is not one: give the {SHORT_LEN} hex characters after `#` \
                     in a placeholder.",
                    i + 1
                )));
            }
        }
    }
    let (facts, secrets) = match read::secrets_in(home, session)? {
        Ok(found) => found,
        Err(why) => return Ok(Filed::Refused(why)),
    };
    *logged = facts.key();
    let mut asked: BTreeSet<Secret> = BTreeSet::new();
    for p in &prefixes {
        let matching: Vec<_> = secrets
            .iter()
            .filter(|f| f.fingerprint.matches(p))
            .collect();
        if matching.is_empty() {
            return Ok(Filed::Refused(format!(
                "no secret in {} has the fingerprint #{p}.",
                facts.key()
            )));
        }
        asked.extend(matching.into_iter().map(|f| Secret {
            fingerprint: f.fingerprint.clone(),
            kind: f.kind,
        }));
    }
    let r = Request {
        id: new_id()?,
        made_at: stamp(now),
        expires_at: stamp(now + Duration::hours(LIFETIME_HOURS)),
        reader: reader.clone(),
        session: facts.key(),
        secrets: asked.into_iter().collect(),
        reason: reason.to_string(),
        state: State::Pending,
    };
    let _lock = home.lock("reveals")?;
    save(home, &r)?;
    Ok(Filed::Filed(Box::new(r)))
}

/// Every request, newest first. Requests past their lifetime are marked expired first, each
/// with a line in the read log.
pub fn list(home: &Home) -> Result<Vec<Request>> {
    list_at(home, Utc::now())
}

fn list_at(home: &Home, now: DateTime<Utc>) -> Result<Vec<Request>> {
    let _lock = home.lock("reveals")?;
    let mut all = expire_due(home, now)?;
    all.sort_by(|a, b| b.made_at.cmp(&a.made_at).then(b.id.cmp(&a.id)));
    Ok(all)
}

/// The requests waiting for the owner, newest first: what the GUI's approvals screen shows.
pub fn pending(home: &Home) -> Result<Vec<Request>> {
    pending_at(home, Utc::now())
}

fn pending_at(home: &Home, now: DateTime<Utc>) -> Result<Vec<Request>> {
    Ok(list_at(home, now)?
        .into_iter()
        .filter(|r| r.state == State::Pending)
        .collect())
}

/// How many requests wait for the owner, for status. Writes nothing.
pub fn pending_count(home: &Home) -> u64 {
    let now = Utc::now();
    load_all(home)
        .iter()
        .filter(|r| r.state == State::Pending && !r.is_due(now))
        .count() as u64
}

/// Approve a pending request. For the GUI alone (ADR 0004): no command and no MCP tool calls
/// this. Fails when the request is not pending, including when it has expired.
pub fn approve(home: &Home, id: &str) -> Result<Request> {
    answer(home, id, true, Utc::now())
}

/// Deny a pending request. For the GUI alone, like [`approve`].
pub fn deny(home: &Home, id: &str) -> Result<Request> {
    answer(home, id, false, Utc::now())
}

fn answer(home: &Home, id: &str, approved: bool, now: DateTime<Utc>) -> Result<Request> {
    let _lock = home.lock("reveals")?;
    expire_due(home, now)?;
    let mut r = load(home, id)?.with_context(|| format!("no reveal request {id:?}"))?;
    if r.state != State::Pending {
        bail!(
            "reveal request {id} is {}, so it can no longer be {}",
            r.state.name(),
            if approved { "approved" } else { "denied" }
        );
    }
    let at = stamp(now);
    let result;
    (r.state, result) = if approved {
        (
            State::Approved {
                approved_at: at.clone(),
            },
            ReadResult::RevealApproved {
                request: r.id.clone(),
            },
        )
    } else {
        (
            State::Denied {
                denied_at: at.clone(),
            },
            ReadResult::RevealDenied {
                request: r.id.clone(),
            },
        )
    };
    save(home, &r)?;
    log(home, None, &r.reader, Some(r.session.clone()), result, now)?;
    Ok(r)
}

/// Collect the values of request `id` as `reader`: only the reader that filed it, only once the
/// owner has approved it, only once, and only before it expires. Every call is a line in the
/// read log, which records how many values were released and never the values.
pub fn collect(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    id: &str,
) -> Result<Collected> {
    collect_at(home, runtime, reader, id, Utc::now())
}

fn collect_at(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    id: &str,
    now: DateTime<Utc>,
) -> Result<Collected> {
    let _lock = home.lock("reveals")?;
    let mut session = None;
    let collected = release(home, runtime, reader, id, now, &mut session);
    let result = match &collected {
        Ok(Collected::Released(r)) => ReadResult::RevealCollected {
            request: r.request.clone(),
            secrets: r.values.len() as u64,
        },
        Ok(Collected::Refused(why)) => ReadResult::Refused {
            reason: why.clone(),
        },
        Err(e) => ReadResult::Failed {
            error: format!("{e:#}"),
        },
    };
    // Nothing is released that could not be logged.
    log(home, Some(runtime), reader, session, result, now)?;
    collected
}

fn release(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    id: &str,
    now: DateTime<Utc>,
    session: &mut Option<String>,
) -> Result<Collected> {
    let refuse = |why: String| Ok(Collected::Refused(why));
    if let Access::Refused(why) = trust(home).check(reader, runtime) {
        return refuse(why);
    }
    expire_due(home, now)?;
    let id = id.trim();
    let Some(mut r) = (if is_id(id) { load(home, id)? } else { None }) else {
        return refuse("no such reveal request.".into());
    };
    *session = Some(r.session.clone());
    if !same_reader(&r.reader, reader) {
        return refuse(format!(
            "reveal request {id} was filed by another reader; only that reader collects it."
        ));
    }
    let approved_at = match &r.state {
        State::Approved { approved_at } => approved_at.clone(),
        State::Pending => {
            return refuse(format!(
                "reveal request {id} is not approved yet: the owner approves or denies it in \
                 DISTILL's window. Try again before {}, when it expires.",
                r.expires_at
            ));
        }
        State::Denied { .. } => return refuse(format!("the owner denied reveal request {id}.")),
        State::Collected { .. } => {
            return refuse(format!(
                "reveal request {id} was already collected; an approval releases its values once."
            ));
        }
        State::Expired { .. } => {
            return refuse(format!(
                "reveal request {id} expired at {}; file a new one.",
                r.expires_at
            ));
        }
    };
    let wanted: BTreeSet<Fingerprint> = r.secrets.iter().map(|s| s.fingerprint.clone()).collect();
    let mut found = read::reveal_values(home, &r.session, &wanted)?;
    let mut released = Released {
        request: r.id.clone(),
        session: r.session.clone(),
        values: Vec::new(),
        missing: Vec::new(),
    };
    for s in &r.secrets {
        match found.remove(&s.fingerprint) {
            Some((_, value)) => released.values.push(Revealed {
                secret: s.clone(),
                value,
            }),
            None => released.missing.push(s.clone()),
        }
    }
    // Marked collected before anything is returned: a second collection is refused even if
    // this one goes no further.
    r.state = State::Collected {
        approved_at,
        collected_at: stamp(now),
    };
    save(home, &r)?;
    Ok(Collected::Released(released))
}

/// Mark every pending or approved request past its lifetime expired, each with a line in the
/// read log, and return every request. The caller holds the reveals lock.
fn expire_due(home: &Home, now: DateTime<Utc>) -> Result<Vec<Request>> {
    let mut all = load_all(home);
    for r in &mut all {
        let open = matches!(r.state, State::Pending | State::Approved { .. });
        if open && r.is_due(now) {
            r.state = State::Expired {
                expired_at: stamp(now),
            };
            save(home, r)?;
            log(
                home,
                None,
                &r.reader,
                Some(r.session.clone()),
                ReadResult::RevealExpired {
                    request: r.id.clone(),
                },
                now,
            )?;
        }
    }
    Ok(all)
}

fn trust(home: &Home) -> Trust {
    // With no config, or one that can't be read, only the built-in providers count: that trusts
    // less, never more.
    Trust::of(home).1
}

/// The same model reader: the provider whatever its case, and the same model or none.
fn same_reader(a: &Reader, b: &Reader) -> bool {
    match (a, b) {
        (
            Reader::Model {
                provider: pa,
                model: ma,
            },
            Reader::Model {
                provider: pb,
                model: mb,
            },
        ) => {
            let p = |p: &Option<String>| p.as_deref().map(|s| s.trim().to_ascii_lowercase());
            p(pa).is_some() && p(pa) == p(pb) && ma == mb
        }
        _ => false,
    }
}

/// A fingerprint as a reader names it (`a1b2c3d4`, `#a1b2c3d4`, or the whole placeholder), as
/// lowercase hex of at least the short form's length; None when it is not one.
fn prefix(named: &str) -> Option<String> {
    let s = named.trim();
    let s = s.rsplit_once('#').map_or(s, |(_, after)| after);
    let s = s.trim_end_matches(']').trim().to_ascii_lowercase();
    let hex = s.chars().all(|c| c.is_ascii_hexdigit());
    (hex && (SHORT_LEN..=64).contains(&s.len())).then_some(s)
}

/// Whether the detector finds a secret in `text`. Personal details don't count here.
fn holds_secret(text: &str) -> bool {
    detect::Detector::default()
        .detect(text)
        .iter()
        .any(|h| h.kind.class() == detect::Class::Secret)
}

/// What the read log records of a session a reader asked for: as asked, unless the detector
/// finds a secret in it.
fn loggable(asked: &str) -> String {
    if !holds_secret(asked) {
        crate::util::clip(asked.trim(), 200)
    } else {
        "(withheld: it looks like a secret)".into()
    }
}

fn log(
    home: &Home,
    runtime: Option<&LocalRuntime>,
    reader: &Reader,
    session: Option<String>,
    result: ReadResult,
    now: DateTime<Utc>,
) -> Result<()> {
    readlog::append(
        home,
        runtime,
        &ReadEntry {
            at: stamp(now),
            reader: reader.clone(),
            session,
            result,
        },
    )
}

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn new_id() -> Result<String> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("generate a request ID: {e}"))?;
    Ok(hex::encode(b))
}

fn is_id(id: &str) -> bool {
    id.len() == 16 && id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

fn save(home: &Home, r: &Request) -> Result<()> {
    let dir = home.reveals_dir();
    create_dir_private(&dir).with_context(|| format!("create {}", dir.display()))?;
    write_json_pretty(&home.reveal_path(&r.id), r)
}

fn load(home: &Home, id: &str) -> Result<Option<Request>> {
    if !is_id(id) {
        return Ok(None);
    }
    let path = home.reveal_path(id);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    let r: Request =
        serde_json::from_str(&text).with_context(|| format!("read {}", path.display()))?;
    Ok((r.id == id).then_some(r))
}

/// Every request file that reads. One that doesn't can be neither approved nor collected.
fn load_all(home: &Home) -> Vec<Request> {
    let Ok(entries) = std::fs::read_dir(home.reveals_dir()) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".json")?;
            load(home, id).ok().flatten()
        })
        .collect()
}

#[cfg(test)]
mod tests;
