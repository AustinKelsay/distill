//! The detector: text in, findings out (issue #72, "Findings"; ADR 0002). It is pure: it reads
//! only the text it is given, and the same text always gives the same hits. Where a hit sits in a
//! session, its fingerprint and the findings file are `crate::findings`.
//!
//! It runs, in order of precedence:
//!
//! 1. pattern rules ([`rules`]), each a [`Kind`] and a regex;
//! 2. secret assignments (`DB_PASSWORD=…`, `token: |` and a YAML block), [`Kind::AssignedSecret`];
//! 3. in text that is JSON, every string decoded and detected on its own, and the value under a
//!    credential-named key whole, [`Kind::CredentialField`].
//!
//! Hits never overlap ([`resolve`]): of two with the same range the earlier rule's is kept, a hit
//! inside another is dropped (the outer value is the secret), and two that overlap partly are
//! joined into one with the first one's kind.
//!
//! Personal details (#77) are added as more [`Kind`]s of [`Class::PersonalDetail`] and more
//! [`rules`], after the secrets so that a secret wins a tie. Bump [`DETECTOR_VERSION`] whenever
//! what the detector finds changes: every session is scanned again.

mod assign;
mod json;

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The detector's version. Findings made by another version are out of date.
pub const DETECTOR_VERSION: u32 = 1;

/// How many levels of JSON inside JSON strings are read token by token; deeper text goes to the
/// text scanner rather than risk the stack.
const MAX_JSON_DEPTH: u32 = 64;

/// What a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A PEM private key, whole or cut short.
    PrivateKey,
    AnthropicKey,
    OpenaiKey,
    GithubToken,
    AwsAccessKey,
    SlackToken,
    TinfoilKey,
    GoogleApiKey,
    StripeKey,
    Jwt,
    NostrSecret,
    /// The token after `Bearer`.
    Bearer,
    /// The value of a credential-named assignment in text (`DB_PASSWORD=…`).
    AssignedSecret,
    /// A value under a credential-named key in JSON (`{"password": …}`).
    CredentialField,
}

/// Whether a finding is a secret or a personal detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// A credential. Models read a placeholder in its place.
    Secret,
    /// A detail that identifies a person or their systems, left in place for models. Its kinds
    /// arrive with #77.
    PersonalDetail,
}

impl Kind {
    pub fn class(self) -> Class {
        Class::Secret
    }

    /// The kind as written in findings and placeholders (`openai_key`).
    pub fn name(self) -> &'static str {
        match self {
            Kind::PrivateKey => "private_key",
            Kind::AnthropicKey => "anthropic_key",
            Kind::OpenaiKey => "openai_key",
            Kind::GithubToken => "github_token",
            Kind::AwsAccessKey => "aws_access_key",
            Kind::SlackToken => "slack_token",
            Kind::TinfoilKey => "tinfoil_key",
            Kind::GoogleApiKey => "google_api_key",
            Kind::StripeKey => "stripe_key",
            Kind::Jwt => "jwt",
            Kind::NostrSecret => "nostr_secret",
            Kind::Bearer => "bearer",
            Kind::AssignedSecret => "assigned_secret",
            Kind::CredentialField => "credential_field",
        }
    }
}

/// One thing the detector found in a text.
#[derive(Clone, PartialEq)]
pub struct Hit {
    pub kind: Kind,
    /// Bytes of the text given to the detector.
    pub range: Range<usize>,
    /// The value, decoded when the range covers escaped JSON, for its fingerprint. Never
    /// stored, and left out of `Debug`.
    pub value: String,
}

impl std::fmt::Debug for Hit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hit")
            .field("kind", &self.kind)
            .field("range", &self.range)
            .field("value", &format_args!("<{} bytes>", self.value.len()))
            .finish()
    }
}

/// Everything the detector finds in `text`, in order, none overlapping.
pub fn detect(text: &str) -> Vec<Hit> {
    detect_at(text, 0)
}

/// Where a hit sits in a JSON value, such as a tool call's input.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueHit {
    /// A JSON pointer (RFC 6901) to a string or number.
    pub pointer: String,
    /// The hit is in the key that `pointer` ends with, not in its value.
    pub key: bool,
    /// Bytes of that string or key, decoded, or of the number as written.
    pub hit: Hit,
}

/// Everything the detector finds in a JSON value: in every string and key, and under a
/// credential-named key a string or number whole, or each one of an array whole. An object under
/// such a key is left to its own keys: a tool schema's `"password": {"type": "string"}` holds no
/// secret.
pub fn detect_value(v: &Value) -> Vec<ValueHit> {
    let mut out = Vec::new();
    walk(v, &mut String::new(), false, &mut out);
    out
}

fn walk(v: &Value, pointer: &mut String, secret: bool, out: &mut Vec<ValueHit>) {
    let whole = |written: String| Hit {
        kind: Kind::CredentialField,
        range: 0..written.len(),
        value: written,
    };
    match v {
        Value::String(s) if secret && !s.is_empty() => out.push(ValueHit {
            pointer: pointer.clone(),
            key: false,
            hit: whole(s.clone()),
        }),
        Value::String(s) => out.extend(detect(s).into_iter().map(|hit| ValueHit {
            pointer: pointer.clone(),
            key: false,
            hit,
        })),
        Value::Number(n) if secret && json::is_secret_number(&n.to_string()) => {
            out.push(ValueHit {
                pointer: pointer.clone(),
                key: false,
                hit: whole(n.to_string()),
            })
        }
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                let len = pointer.len();
                pointer.push_str(&format!("/{i}"));
                walk(x, pointer, secret, out);
                pointer.truncate(len);
            }
        }
        Value::Object(o) => {
            for (k, x) in o {
                let len = pointer.len();
                pointer.push('/');
                pointer.push_str(&k.replace('~', "~0").replace('/', "~1"));
                out.extend(detect(k).into_iter().map(|hit| ValueHit {
                    pointer: pointer.clone(),
                    key: true,
                    hit,
                }));
                walk(x, pointer, json::is_credential_key(k), out);
                pointer.truncate(len);
            }
        }
        _ => {}
    }
}

/// The placeholder a model reads in place of a secret: `[secret: <kind> #<fingerprint>]`.
pub fn placeholder(kind: Kind, short_fingerprint: &str) -> String {
    format!("[secret: {} #{short_fingerprint}]", kind.name())
}

/// Where `text` holds placeholders. A session that read Distill's own output holds them, and
/// neither a placeholder nor what looks like an assignment inside one (`secret: openai_key`) is
/// a secret.
fn placeholders(text: &str) -> Vec<Range<usize>> {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[secret: [a-z_]+ #[0-9a-f]{8}\]").expect("valid regex"))
        .find_iter(text)
        .map(|m| m.range())
        .collect()
}

fn detect_at(text: &str, depth: u32) -> Vec<Hit> {
    if depth < MAX_JSON_DEPTH
        && let Some(hits) = json::detect_json(text, depth)
    {
        return hits;
    }
    detect_text(text)
}

/// A pattern rule.
struct Rule {
    kind: Kind,
    re: Regex,
}

/// The pattern rules, in order of precedence. A match's finding is its `v` group when it has
/// one, else the whole match after its `keep` group.
fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let r = |kind, pat: &str| Rule {
            kind,
            re: Regex::new(pat).expect("valid detector regex"),
        };
        // A key's start: a word boundary, or an escaped newline, tab, backspace or form feed
        // (`\ntk_…` in JSON inside a string, where `\b` finds no boundary between the `n` and
        // the key).
        let k = |kind, pat: &str| r(kind, &format!(r"(?:\b|(?P<keep>\\[nrtbf])){pat}"));
        vec![
            // A whole key, or a truncated one (tool output often shows only the head).
            r(
                Kind::PrivateKey,
                r"-----BEGIN [A-Z ]*PRIVATE KEY-----(?:[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----|[A-Za-z0-9+/=\s]*)",
            ),
            r(Kind::AnthropicKey, r"sk-ant-[A-Za-z0-9_\-]{20,}"),
            r(Kind::OpenaiKey, r"sk-(?:proj-)?[A-Za-z0-9_\-]{20,}"),
            k(
                Kind::GithubToken,
                r"(?:(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})\b",
            ),
            k(Kind::AwsAccessKey, r"(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
            k(Kind::SlackToken, r"xox[abprs]-[A-Za-z0-9\-]{10,}\b"),
            k(Kind::TinfoilKey, r"tk_[A-Za-z0-9]{40,}\b"),
            k(Kind::GoogleApiKey, r"AIza[0-9A-Za-z_\-]{35}\b"),
            k(Kind::StripeKey, r"(?:sk|rk)_(?:live|test)_[0-9A-Za-z]{20,}\b"),
            k(
                Kind::Jwt,
                r"eyJ[A-Za-z0-9_\-]{10,}\.eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b",
            ),
            k(Kind::NostrSecret, r"nsec1[02-9ac-hj-np-z]{58}\b"),
            k(Kind::Bearer, r"(?i:bearer)\s+(?P<v>[A-Za-z0-9_\-\.=]{20,})"),
        ]
    })
}

/// Text that is not JSON, or JSON that does not parse (a truncated snippet).
fn detect_text(text: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (rank, rule) in rules().iter().enumerate() {
        for c in rule.re.captures_iter(text) {
            let whole = c.get(0).expect("whole match");
            let range = match (c.name("v"), c.name("keep")) {
                (Some(v), _) => v.range(),
                (None, Some(keep)) => keep.end()..whole.end(),
                (None, None) => whole.range(),
            };
            hits.push((rank, rule.kind, range));
        }
    }
    let rank = rules().len();
    hits.extend(
        assign::assignments(text)
            .into_iter()
            .map(|r| (rank, Kind::AssignedSecret, r)),
    );
    resolve(hits)
        .into_iter()
        .map(|(kind, range)| Hit {
            kind,
            value: text[range.clone()].to_string(),
            range,
        })
        .collect()
}

/// Hits in order with none overlapping: of two with the same range the lower rank's is kept, one
/// inside another is dropped, and two that overlap partly are joined with the first one's kind.
fn resolve(mut hits: Vec<(usize, Kind, Range<usize>)>) -> Vec<(Kind, Range<usize>)> {
    hits.retain(|(_, _, r)| r.start < r.end);
    hits.sort_by(|a, b| {
        (a.2.start, std::cmp::Reverse(a.2.end), a.0).cmp(&(
            b.2.start,
            std::cmp::Reverse(b.2.end),
            b.0,
        ))
    });
    let mut out: Vec<(Kind, Range<usize>)> = Vec::new();
    for (_, kind, r) in hits {
        match out.last_mut() {
            Some((_, last)) if r.start < last.end => last.end = last.end.max(r.end),
            _ => out.push((kind, r)),
        }
    }
    out
}

#[cfg(test)]
mod tests;
