//! The detector: text in, findings out (issue #72, "Findings"; ADR 0002). It is pure: it reads
//! only the text it is given and what a [`Detector`] was told about the [`Owner`], and the same
//! text always gives the same hits. Where a hit sits in a session, its fingerprint and the
//! findings file are `crate::findings`.
//!
//! It runs, in order of precedence:
//!
//! 1. pattern rules ([`rules`]), each a [`Kind`] and a regex: the secrets, then the personal
//!    details (email addresses and their slugs, tailnet hosts, names and addresses), then the
//!    rules made from the owner (their username inside paths, each of `personal_terms`);
//! 2. secret assignments (`DB_PASSWORD=…`, `token: |` and a YAML block), [`Kind::AssignedSecret`];
//! 3. in text that is JSON, every string decoded and detected on its own, and the value under a
//!    credential-named key whole, [`Kind::CredentialField`].
//!
//! Hits never overlap ([`resolve`]). Among secrets, of two with the same range the earlier
//! rule's is kept, a hit inside another is dropped (the outer value is the secret), and two that
//! overlap partly are joined into one with the first one's kind. Personal details resolve the
//! same way among themselves, and one that touches a secret is dropped: a secret always wins.
//!
//! Every rule reads the original text; none relies on another having rewritten it. Bump
//! [`DETECTOR_VERSION`] whenever what the detector finds changes: every session is scanned
//! again.

mod assign;
mod json;
mod weak;

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The detector's version. Findings made by another version are out of date.
pub const DETECTOR_VERSION: u32 = 5;

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
    /// An email address, except a service's (`git@…`, `noreply@…`).
    Email,
    /// An email address written as a slug (`jane-roe-gmail-com`).
    EmailSlug,
    /// A host on a tailnet (`box.tail1234.ts.net`).
    TailnetHost,
    /// A tailnet's name on its own, cut short or slugged (`tail0c0ffe`, `box-tail0c0ffe-ts-net`).
    TailnetName,
    /// An address in the tailnet range (`100.64.0.0/10`, `fd7a:115c:a1e0::/48`).
    TailnetAddress,
    /// The owner's username inside a path (`/Users/<name>/…`).
    Username,
    /// A term from the config's `personal_terms`.
    PersonalTerm,
    /// A person's name, found by the model pass (`crate::details`).
    PersonName,
    /// A postal address, found by the model pass.
    PostalAddress,
    /// A phone number, found by the model pass.
    PhoneNumber,
    /// A client's or a company's name, found by the model pass.
    Organization,
}

/// Whether a finding is a secret or a personal detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// A credential. Models read a placeholder in its place.
    Secret,
    /// A detail that identifies a person or their systems, left in place for models.
    PersonalDetail,
}

impl Kind {
    /// Every kind, secrets first, in the order above: for a face's kind filter.
    pub const ALL: [Kind; 25] = [
        Kind::PrivateKey,
        Kind::AnthropicKey,
        Kind::OpenaiKey,
        Kind::GithubToken,
        Kind::AwsAccessKey,
        Kind::SlackToken,
        Kind::TinfoilKey,
        Kind::GoogleApiKey,
        Kind::StripeKey,
        Kind::Jwt,
        Kind::NostrSecret,
        Kind::Bearer,
        Kind::AssignedSecret,
        Kind::CredentialField,
        Kind::Email,
        Kind::EmailSlug,
        Kind::TailnetHost,
        Kind::TailnetName,
        Kind::TailnetAddress,
        Kind::Username,
        Kind::PersonalTerm,
        Kind::PersonName,
        Kind::PostalAddress,
        Kind::PhoneNumber,
        Kind::Organization,
    ];

    pub fn class(self) -> Class {
        match self {
            Kind::Email
            | Kind::EmailSlug
            | Kind::TailnetHost
            | Kind::TailnetName
            | Kind::TailnetAddress
            | Kind::Username
            | Kind::PersonalTerm
            | Kind::PersonName
            | Kind::PostalAddress
            | Kind::PhoneNumber
            | Kind::Organization => Class::PersonalDetail,
            _ => Class::Secret,
        }
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
            Kind::Email => "email",
            Kind::EmailSlug => "email_slug",
            Kind::TailnetHost => "tailnet_host",
            Kind::TailnetName => "tailnet_name",
            Kind::TailnetAddress => "tailnet_address",
            Kind::Username => "username",
            Kind::PersonalTerm => "personal_term",
            Kind::PersonName => "person_name",
            Kind::PostalAddress => "postal_address",
            Kind::PhoneNumber => "phone_number",
            Kind::Organization => "organization",
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

/// What the detector is told about the owner, which no pattern can know.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Owner {
    /// The owner's username, found inside paths (`/Users/<name>/…`, `/home/<name>/…`).
    pub username: Option<String>,
    /// The config's `personal_terms`, each found as a whole word or phrase, ignoring case.
    pub terms: Vec<String>,
}

/// The detector for one owner: the pattern rules, and rules made from what it was told about
/// the owner. [`Detector::default`] knows nothing about the owner.
#[derive(Default)]
pub struct Detector {
    /// After [`rules`], in order of precedence.
    owner_rules: Vec<Rule>,
}

impl Detector {
    pub fn new(owner: &Owner) -> Self {
        let mut owner_rules = Vec::new();
        if let Some(name) = owner.username.as_deref().map(str::trim)
            && !name.is_empty()
        {
            // The name as a whole path component after a home folder, also in escaped JSON
            // (`C:\\Users\\<name>`) that the text scanner reads.
            owner_rules.push(Rule::new(
                Kind::Username,
                &format!(
                    r"(?i)[/\\](?:Users|home)[/\\]+(?P<v>{})(?:[^A-Za-z0-9_.\-]|$)",
                    regex::escape(name)
                ),
            ));
        }
        let mut terms: Vec<&str> = owner
            .terms
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();
        // Longer terms first, so `Acme Labs` is one finding rather than `Acme` and a rest.
        terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
        terms.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        owner_rules.extend(terms.into_iter().map(term_rule));
        Self { owner_rules }
    }

    /// Everything the detector finds in `text`, in order, none overlapping.
    pub fn detect(&self, text: &str) -> Vec<Hit> {
        self.detect_at(text, 0)
    }
}

/// The rule for one of `personal_terms`: the term as a whole word or phrase, ignoring case, any
/// run of spaces in it matching any other.
fn term_rule(term: &str) -> Rule {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let body = term
        .split_whitespace()
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join(r"\s+");
    // A boundary only where the term's edge is a word character; `\b` before `@handle` would
    // need a word character in front of it.
    let start = if word(term.chars().next()) {
        r"(?:\b|(?P<keep>\\[nrtbf]))"
    } else {
        ""
    };
    let end = if word(term.chars().last()) { r"\b" } else { "" };
    Rule::new(Kind::PersonalTerm, &format!("(?i){start}{body}{end}"))
}

/// Where a hit sits in a JSON value, such as a tool call's input.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueHit {
    /// The path to a string or number: at each level, the position of an array's item or of an
    /// object's member, in the order the value iterates them. Never a key's text, which can be
    /// a secret or a personal detail itself.
    pub path: Vec<usize>,
    /// The hit is in the key of the member that `path` ends with, not in its value.
    pub key: bool,
    /// Bytes of that string or key, decoded, or of the number as written.
    pub hit: Hit,
}

/// Everything the detector finds in a JSON value: in every string and key, and under a
/// credential-named key a string or any number whole, or each one of an array whole. An object under
/// such a key is left to its own keys: a tool schema's `"password": {"type": "string"}` holds no
/// secret.
impl Detector {
    pub fn detect_value(&self, v: &Value) -> Vec<ValueHit> {
        let mut out = Vec::new();
        self.walk(v, &mut Vec::new(), None, &mut out);
        out
    }

    /// `secret` is the credential-named key the value sits under, if any. A value there that
    /// cannot be a secret (too short, an id, a placeholder) is read as any other.
    fn walk(
        &self,
        v: &Value,
        path: &mut Vec<usize>,
        secret: Option<&str>,
        out: &mut Vec<ValueHit>,
    ) {
        let whole = |written: String| Hit {
            kind: Kind::CredentialField,
            range: 0..written.len(),
            value: written,
        };
        let can_be = |value: &str| secret.is_some_and(|key| weak::can_be_field_secret(key, value));
        match v {
            Value::String(s) if can_be(s) => out.push(ValueHit {
                path: path.clone(),
                key: false,
                hit: whole(s.clone()),
            }),
            Value::String(s) => out.extend(self.detect(s).into_iter().map(|hit| ValueHit {
                path: path.clone(),
                key: false,
                hit,
            })),
            Value::Number(n) if secret.is_some_and(|key| weak::can_be_secret_number(key, n)) => out
                .push(ValueHit {
                    path: path.clone(),
                    key: false,
                    hit: whole(n.to_string()),
                }),
            Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    path.push(i);
                    self.walk(x, path, secret, out);
                    path.pop();
                }
            }
            Value::Object(o) => {
                for (i, (k, x)) in o.iter().enumerate() {
                    path.push(i);
                    out.extend(self.detect(k).into_iter().map(|hit| ValueHit {
                        path: path.clone(),
                        key: true,
                        hit,
                    }));
                    let key = json::is_credential_key(k).then_some(k.as_str());
                    self.walk(x, path, key, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
}

/// The placeholder a model reads in place of a secret: `[secret: <kind> #<fingerprint>]`.
pub fn placeholder(kind: Kind, short_fingerprint: &str) -> String {
    format!("[secret: {} #{short_fingerprint}]", kind.name())
}

/// Where `text` holds placeholders. A session that read DISTILL's own output holds them, and
/// neither a placeholder nor what looks like an assignment inside one (`secret: openai_key`) is
/// a secret.
fn placeholders(text: &str) -> Vec<Range<usize>> {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[secret: [a-z_]+ #[0-9a-f]{8}\]").expect("valid regex"))
        .find_iter(text)
        .map(|m| m.range())
        .collect()
}

impl Detector {
    fn detect_at(&self, text: &str, depth: u32) -> Vec<Hit> {
        if depth < MAX_JSON_DEPTH
            && let Some(hits) = json::detect_json(self, text, depth)
        {
            return hits;
        }
        self.detect_text(text)
    }

    /// Text that is not JSON, or JSON that does not parse (a truncated snippet).
    fn detect_text(&self, text: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        let all = rules().iter().chain(&self.owner_rules);
        for (rank, rule) in all.clone().enumerate() {
            for c in rule.re.captures_iter(text) {
                if !rule.check.is_none_or(|check| check(text, &c)) {
                    continue;
                }
                let whole = c.get(0).expect("whole match");
                let range = match (c.name("v"), c.name("keep")) {
                    (Some(v), _) => v.range(),
                    (None, Some(keep)) => keep.end()..whole.end(),
                    (None, None) => whole.range(),
                };
                hits.push((rank, rule.kind, range));
            }
        }
        let rank = all.count();
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
}

/// An email address, after a boundary the rule supplies.
const EMAIL: &str = r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b";
/// One part of an IPv4 address, 0 to 255.
const OCTET: &str = r"(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])";

/// An address that names a service, not a person: `git@github.com` remotes, `noreply@…`. Only
/// exactly these: `noreply+alice@…` still names a person.
fn is_service_address(address: &str) -> bool {
    let local = address.split('@').next().unwrap_or("").to_ascii_lowercase();
    matches!(local.as_str(), "git" | "noreply" | "no-reply")
}

/// A pattern rule.
struct Rule {
    kind: Kind,
    re: Regex,
    /// Whether a match is a finding, when not every match is.
    check: Option<fn(&str, &regex::Captures) -> bool>,
}

impl Rule {
    fn new(kind: Kind, pat: &str) -> Self {
        Self {
            kind,
            re: Regex::new(pat).expect("valid detector regex"),
            check: None,
        }
    }
    fn checked(mut self, check: fn(&str, &regex::Captures) -> bool) -> Self {
        self.check = Some(check);
        self
    }
}

/// The pattern rules, in order of precedence. A match's finding is its `v` group when it has
/// one, else the whole match after its `keep` group.
fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let r = Rule::new;
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
            // Personal details, after every secret.
            k(Kind::Email, EMAIL).checked(|text, c| {
                let m = c.get(0).expect("whole match");
                // `\n@router.get` in escaped JSON is the escape's letter, not an address.
                let escape_letter = c.name("keep").is_none()
                    && text[..m.start()].ends_with('\\')
                    && m.as_str().starts_with(['n', 'r', 't', 'b', 'f']);
                !escape_letter && !is_service_address(&text[m.start()..m.end()])
            }),
            // Addresses turned into slugs (`alice-smith-gmail-com`), as contact tools and file
            // names write them. Only mail providers' domains: a slug ending in any domain
            // (`my-github-org`) is far more often a name than an address.
            r(
                Kind::EmailSlug,
                r"(?i)\b[a-z0-9]+(?:[._-][a-z0-9]+)*-(?:gmail|googlemail|protonmail|proton|pm|icloud|outlook|hotmail|live|yahoo|ymail|aol|fastmail|gmx|zoho|hey|tutanota)-(?:com|me|net|ch|de)\b",
            ),
            // A slug at any domain, when an address that is not a service's follows it on the
            // same line (a contact list: `jdoe-example-org ['jdoe@example.org']`). The previous
            // build found the address already rewritten; this reads it in the original text.
            r(
                Kind::EmailSlug,
                &format!(
                    r"(?i)\b(?P<v>[a-z0-9]+(?:[._-][a-z0-9]+)*-[a-z0-9]+-(?:com|org|net|edu|gov|io|co))\b[^\n]{{0,40}}?(?P<addr>\b{EMAIL})"
                ),
            )
            .checked(|_, c| !is_service_address(&c["addr"])),
            r(
                Kind::TailnetHost,
                r"(?i)\b[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.ts\.net\b",
            ),
            // A tailnet name on its own (`tail0c0ffe`), left after output was cut short
            // (`box.tail0c0f`), or slugged (`box-tail0c0ffe-ts-net`).
            r(
                Kind::TailnetName,
                r"(?i)\b(?:[A-Za-z0-9-]+[.-])?tail[0-9a-f]{4,8}(?:[.-]ts[.-]net)?\b",
            ),
            // 100.64.0.0/10, and Tailscale's IPv6 range fd7a:115c:a1e0::/48.
            r(
                Kind::TailnetAddress,
                &format!(r"\b100\.(?:6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.{OCTET}\.{OCTET}\b"),
            ),
            r(Kind::TailnetAddress, r"(?i)\bfd7a:115c:a1e0:[0-9a-f:]*[0-9a-f]"),
        ]
    })
}

/// Hits in order with none overlapping. Secrets are resolved first: of two with the same range
/// the lower rank's is kept, one inside another is dropped, and two that overlap partly are
/// joined with the first one's kind. Personal details are resolved the same way among
/// themselves, and one that overlaps a secret at all is dropped, so a secret is never left in
/// place inside a personal detail.
fn resolve(hits: Vec<(usize, Kind, Range<usize>)>) -> Vec<(Kind, Range<usize>)> {
    let (secrets, personal): (Vec<_>, Vec<_>) = hits
        .into_iter()
        .partition(|(_, kind, _)| kind.class() == Class::Secret);
    let secrets = resolve_class(secrets);
    let mut out: Vec<_> = resolve_class(personal)
        .into_iter()
        .filter(|(_, p)| {
            !secrets
                .iter()
                .any(|(_, s)| p.start < s.end && s.start < p.end)
        })
        .collect();
    out.extend(secrets);
    out.sort_by_key(|(_, r)| r.start);
    out
}

fn resolve_class(mut hits: Vec<(usize, Kind, Range<usize>)>) -> Vec<(Kind, Range<usize>)> {
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
