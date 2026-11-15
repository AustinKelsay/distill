//! Secret scanning for exports. Raw captures are never edited; this runs on the way out.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;

struct Rule {
    name: &'static str,
    re: Regex,
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let r = |name, pat: &str| Rule { name, re: Regex::new(pat).expect("valid redaction regex") };
        // A key's start: a word boundary, or an escaped newline or tab (`\ntk_…` in JSON inside
        // a string, where `\b` finds no boundary between the `n` and the key).
        let k = |name, pat: &str| r(name, &format!(r"(?:\b|(?P<keep>\\[nrt])){pat}"));
        vec![
            // A whole key, or a truncated one (tool output often shows only the head).
            r(
                "private_key",
                r"-----BEGIN [A-Z ]*PRIVATE KEY-----(?:[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----|[A-Za-z0-9+/=\s]*)",
            ),
            r("anthropic_key", r"sk-ant-[A-Za-z0-9_\-]{20,}"),
            r("openai_key", r"sk-(?:proj-)?[A-Za-z0-9_\-]{20,}"),
            k("github_token", r"(?:(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})\b"),
            k("aws_access_key", r"(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
            k("slack_token", r"xox[abprs]-[A-Za-z0-9\-]{10,}\b"),
            k("tinfoil_key", r"tk_[A-Za-z0-9]{40,}\b"),
            k("google_api_key", r"AIza[0-9A-Za-z_\-]{35}\b"),
            k("stripe_key", r"(?:sk|rk)_(?:live|test)_[0-9A-Za-z]{20,}\b"),
            k("jwt", r"eyJ[A-Za-z0-9_\-]{10,}\.eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b"),
            k("nostr_secret", r"nsec1[02-9ac-hj-np-z]{58}\b"),
            k("bearer", r"(?i:bearer)\s+[A-Za-z0-9_\-\.=]{20,}"),
            // Personal data and private network names, not only credentials.
            r("email", r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"),
            // Addresses turned into slugs (`alice-smith-gmail-com`), as contact tools and file
            // names write them. Only mail providers' domains: a slug ending in any domain
            // (`my-github-org`) is far more often a name than an address.
            r(
                "email_slug",
                r"(?i)\b[a-z0-9]+(?:[._-][a-z0-9]+)*-(?:gmail|googlemail|protonmail|proton|pm|icloud|outlook|hotmail|live|yahoo|ymail|aol|fastmail|gmx|zoho|hey|tutanota)-(?:com|me|net|ch|de)\b",
            ),
            // A slugged address at any domain, when the address itself sits right after it
            // (a contact list: `jdoe-example-org ['jdoe@example.org']`). Runs after `email`, so
            // the address is already a marker; the slug alone (`my-github-org`) is kept.
            r(
                "email_slug",
                r"(?i)\b[a-z0-9]+(?:[._-][a-z0-9]+)*-[a-z0-9]+-(?:com|org|net|edu|gov|io|co)\b(?P<after>[^\n]{0,40}?\[REDACTED:email\])",
            ),
            r("tailnet_host", r"(?i)\b[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.ts\.net\b"),
            // A tailnet name on its own (`tail0c0ffe`), left after output was cut short
            // (`box.tail0c0f`), or slugged (`box-tail0c0ffe-ts-net`).
            r(
                "tailnet_name",
                r"(?i)\b(?:[A-Za-z0-9-]+[.-])?tail[0-9a-f]{4,8}(?:[.-]ts[.-]net)?\b",
            ),
            r(
                "tailscale_ip",
                r"\b100\.(?:6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.[0-9]{1,3}\.[0-9]{1,3}\b",
            ),
        ]
    })
}

/// The name and separator of a secret assignment: `TINFOIL_API_KEY=`, `"db-password": "`,
/// `_SECRET: `, or `\"GH_TOKEN\":\n  \"` in JSON that sits inside a string. The name may carry
/// any prefix, since a bare `\b` would stop at the `_`. `scan_value` finds where the value ends.
const ASSIGNMENT_HEAD: &str = r#"(?i)\b(?:[A-Za-z0-9]*[_-]+)*(?:password|passwd|secret|client[_-]?secret|token|api[_-]?key|access[_-]?token|auth[_-]?token|private[_-]?key)\b(?P<nq>\\*["'])?(?:\s|\\[nrt])*[:=](?:\s|\\[nrt])*(?P<q>\\*["'])?"#;
/// Bump when `scan_value` changes, so builds made by the old scanner are not reused.
const ASSIGNMENT_SCANNER: u32 = 3;
/// Shorter values are rarely secrets (`csrf_token=short`, `max_token=4096`).
const MIN_SECRET_CHARS: usize = 8;
/// How far a quoted value is followed before giving up on its closing quote.
const MAX_QUOTED_BYTES: usize = 8192;

fn assignment_head() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(ASSIGNMENT_HEAD).expect("valid assignment regex"))
}

/// Changes whenever a rule changes, so dataset builds made with old rules are not reused.
pub fn fingerprint() -> String {
    let mut joined: Vec<String> = rules()
        .iter()
        .map(|r| format!("{}={}", r.name, r.re.as_str()))
        .collect();
    joined.push(format!(
        "assigned_secret={ASSIGNMENT_HEAD};scanner={ASSIGNMENT_SCANNER}"
    ));
    crate::util::sha256_hex(joined.join("\n").as_bytes())
}

/// Replace the value of every secret assignment, keeping its name, quotes and escapes, so
/// redacted JSON (at any nesting depth) stays valid.
fn redact_assignments(text: &str, n: &mut u64) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    while let Some(c) = assignment_head().captures_at(text, pos) {
        let head_end = c.get(0).expect("whole match").end();
        let rest = &text[head_end..];
        let found = scan_value(rest, c.name("q").map(|q| q.as_str())).filter(|&(skip, len)| {
            let v = &rest[skip..skip + len];
            v.chars().count() >= MIN_SECRET_CHARS && !v.starts_with("[REDACTED:")
        });
        let Some((skip, len)) = found else {
            out.push_str(&text[pos..head_end]);
            pos = head_end;
            continue;
        };
        *n += 1;
        out.push_str(&text[pos..head_end + skip]);
        // A bare value after a quoted name is a JSON number or literal; quote the marker the
        // same way the name is quoted, or the JSON breaks.
        let wrap = match (c.name("q"), c.name("nq")) {
            (None, Some(nq)) if nq.as_str().ends_with('"') && skip == 0 => nq.as_str(),
            _ => "",
        };
        out.push_str(&format!("{wrap}[REDACTED:assigned_secret]{wrap}"));
        pos = head_end + skip + len;
    }
    out.push_str(&text[pos..]);
    out
}

/// The nesting level a quote sits at after `m` backslashes. Each level of JSON inside a string
/// doubles the backslashes before it and adds one (`"`, `\"`, `\\\"`), so `\\"` is an
/// escaped backslash and then a level-0 quote, not an escaped quote.
fn level(mut m: usize) -> u32 {
    let mut l = 0;
    while m % 2 == 1 {
        m = (m - 1) / 2;
        l += 1;
    }
    l
}

/// Where the value starts and how long it is, in `rest` (the text after the separator).
/// `quote` is the opening quote with any backslashes that escape it.
fn scan_value(rest: &str, quote: Option<&str>) -> Option<(usize, usize)> {
    let b = rest.as_bytes();
    if let Some(q) = quote {
        // Closed by the same quote at the same escape depth (or shallower, if the text is cut).
        // A more deeply escaped quote is part of the value.
        let depth = q.len() - 1;
        let mark = q.as_bytes()[depth];
        let mut i = 0;
        while i < b.len() && i < MAX_QUOTED_BYTES {
            match b[i] {
                b'\\' => {
                    let m = b[i..].iter().take_while(|&&x| x == b'\\').count();
                    match b.get(i + m) {
                        // The quote's own escapes stay; the escaped backslashes before them
                        // are the end of the value.
                        Some(&x) if x == mark && level(m) == level(depth) => {
                            return Some((0, i + m - depth));
                        }
                        Some(&x) if x == mark && level(m) < level(depth) => return Some((0, i)),
                        // A single-quoted value never runs past the double quote that closes
                        // the string around it.
                        Some(b'"') if mark == b'\'' && level(m) <= level(depth) => {
                            return Some((0, i));
                        }
                        // An escaped newline at the string's own depth: the value was cut off.
                        Some(b'n') if depth > 0 && m == depth => return Some((0, i)),
                        _ => i += m + 1,
                    }
                }
                x if x == mark => return Some((0, i)),
                b'"' if mark == b'\'' => return Some((0, i)),
                b'\n' => return Some((0, i)),
                _ => i += 1,
            }
        }
        let mut end = i.min(b.len());
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        return Some((0, end));
    }
    // A YAML block scalar (`KEY: |`): the indented lines that follow are the value.
    if matches!(b.first(), Some(b'|' | b'>')) {
        let header = rest
            .bytes()
            .skip(1)
            .take_while(|x| matches!(x, b'-' | b'+' | b'0'..=b'9' | b' ' | b'\t' | b'\r'))
            .count();
        if b.get(1 + header) == Some(&b'\n') {
            let start = 2 + header;
            let mut end = start;
            for line in rest[start..].split_inclusive('\n') {
                if !line.starts_with([' ', '\t']) {
                    break;
                }
                end += line.len();
            }
            let block = rest[start..end].trim_end();
            let indent = block.len() - block.trim_start().len();
            return (!block.trim().is_empty()).then_some((start + indent, block.len() - indent));
        }
    }
    // A bare token. A backslash ends it only when it escapes a quote or whitespace; any other
    // escape (`s`, `\\`) is part of the value, so no half of it is left behind.
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\'' | b',' | b'}' => break,
            b'\\' => match b.get(i + 1) {
                None | Some(b'"' | b'\'' | b'n' | b'r' | b't') => break,
                Some(_) => i += 2,
            },
            _ => i += 1,
        }
    }
    let mut end = i.min(b.len());
    while !rest.is_char_boundary(end) {
        end += 1;
    }
    Some((0, end))
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Findings {
    pub by_rule: BTreeMap<String, u64>,
}

impl Findings {
    pub fn total(&self) -> u64 {
        self.by_rule.values().sum()
    }
    pub fn merge(&mut self, other: &Findings) {
        for (k, v) in &other.by_rule {
            *self.by_rule.entry(k.clone()).or_default() += v;
        }
    }
}

/// Replace secrets with `[REDACTED:<rule>]` and the user's home directory with `~`.
/// Addresses that identify a service, not a person: `git@github.com` remotes, `noreply@…`.
fn is_service_address(m: &str) -> bool {
    let local = m.split('@').next().unwrap_or("").to_ascii_lowercase();
    // Exact matches only: `noreply+alice@…` still identifies a person.
    matches!(local.as_str(), "git" | "noreply" | "no-reply")
}

pub fn redact(text: &str, home_dir: Option<&str>, findings: &mut Findings) -> String {
    let mut out = text.to_string();
    for rule in rules() {
        let mut n = 0u64;
        let hay = out.as_str();
        let replaced = rule.re.replace_all(hay, |c: &regex::Captures| {
            let m = c.get(0).expect("whole match");
            let mut whole = m.as_str();
            // A match that starts on the letter of an escape (`\nalice@…`, `\n@router.get`):
            // keep the letter, or the redacted text ends in a bare `\` and its JSON breaks.
            let escaped =
                hay[..m.start()].ends_with('\\') && whole.starts_with(['n', 'r', 't', 'b', 'f']);
            let lead = if escaped { &whole[..1] } else { "" };
            if escaped {
                whole = &whole[1..];
                if whole.starts_with('@') {
                    return m.as_str().to_string();
                }
            }
            if rule.name == "email" && is_service_address(whole) {
                return m.as_str().to_string();
            }
            n += 1;
            let keep = c.name("keep").map_or(lead, |k| k.as_str());
            let after = c.name("after").map_or("", |k| k.as_str());
            format!("{keep}[REDACTED:{}]{after}", rule.name)
        });
        if n > 0 {
            *findings.by_rule.entry(rule.name.to_string()).or_default() += n;
            out = replaced.into_owned();
        }
    }
    let mut n = 0u64;
    let replaced = redact_assignments(&out, &mut n);
    if n > 0 {
        *findings
            .by_rule
            .entry("assigned_secret".to_string())
            .or_default() += n;
        out = replaced;
    }
    if let Some(h) = home_dir
        && h.len() > 1
    {
        out = out.replace(h, "~");
    }
    out
}

fn is_credential_key(key: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^(?:[A-Za-z0-9]*[_-]+)*(?:password|passwd|pass|secret|client[_-]?secret|token|access[_-]?token|refresh[_-]?token|auth[_-]?token|api[_-]?key|apikey|private[_-]?key|authorization|credentials?)$")
            .expect("valid key regex")
    })
    .is_match(key)
}

/// Redact every string inside a JSON value. A string under a credential-named key (as in a
/// tool call's `{"password": "..."}`) is replaced whole.
pub fn redact_value(v: &mut serde_json::Value, home_dir: Option<&str>, findings: &mut Findings) {
    match v {
        serde_json::Value::String(s) => *s = redact(s, home_dir, findings),
        serde_json::Value::Array(a) => a
            .iter_mut()
            .for_each(|x| redact_value(x, home_dir, findings)),
        serde_json::Value::Object(o) => {
            // Keys can carry identifiers too (`{"alice@proton.me": "active"}`).
            if o.keys()
                .any(|k| redact(k, home_dir, &mut Findings::default()) != *k)
            {
                let old = std::mem::take(o);
                for (k, x) in old {
                    let base = redact(&k, home_dir, findings);
                    // Two keys can redact to the same placeholder; number the rest so no value
                    // is dropped.
                    let mut key = base.clone();
                    let mut n = 2;
                    while o.contains_key(&key) {
                        key = format!("{base} #{n}");
                        n += 1;
                    }
                    o.insert(key, x);
                }
            }
            for (k, x) in o.iter_mut() {
                if is_credential_key(k)
                    && let serde_json::Value::String(s) = x
                    && !s.is_empty()
                {
                    *findings
                        .by_rule
                        .entry("credential_field".into())
                        .or_default() += 1;
                    *s = "[REDACTED:credential_field]".to_string();
                } else {
                    redact_value(x, home_dir, findings);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_planted_secrets_and_home_paths() {
        let mut f = Findings::default();
        let text = "key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123 and ghp_abcdefghijklmnopqrstuvwxyz0123456789 \
                    AKIAABCDEFGHIJKLMNOP password=hunter2hunter2 at /Users/me/proj";
        let out = redact(text, Some("/Users/me"), &mut f);
        assert!(!out.contains("sk-ant-api03"), "{out}");
        assert!(!out.contains("ghp_"), "{out}");
        assert!(!out.contains("AKIA"), "{out}");
        assert!(!out.contains("hunter2"), "{out}");
        assert!(out.contains("~/proj"));
        assert_eq!(f.total(), 4);
    }

    #[test]
    fn catches_truncated_private_keys() {
        let mut f = Findings::default();
        let text = "--- head ---\n-----BEGIN EC PRIVATE KEY-----\nMHcCAQEEIB3abc+/def=\nMore";
        let out = redact(text, None, &mut f);
        assert!(!out.contains("MHcCAQEEIB3abc"), "{out}");
        assert!(!out.contains("BEGIN EC PRIVATE KEY"), "{out}");
        assert_eq!(f.by_rule["private_key"], 1);
    }

    #[test]
    fn catches_credential_fields_and_quoted_json() {
        let mut f = Findings::default();
        let mut v = serde_json::json!({"input": {"user": "me", "password": "synthetic-pass-1"}});
        redact_value(&mut v, None, &mut f);
        assert_eq!(v["input"]["password"], "[REDACTED:credential_field]");
        assert_eq!(v["input"]["user"], "me");
        let args = r#"{"password":"synthetic-pass-12345","token":"synthetic-token-12345","n":1}"#;
        let out = redact(args, None, &mut f);
        assert!(!out.contains("synthetic-"), "{out}");
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("still valid JSON");
        assert_eq!(parsed["n"], 1);
        assert_eq!(parsed["token"], "[REDACTED:assigned_secret]");
        assert_eq!(f.total(), 3);
    }

    #[test]
    fn catches_prefixed_secret_names() {
        let mut f = Findings::default();
        let text = "TINFOIL_API_KEY=synthetic-value-0001 DB_PASSWORD: synthetic-value-0002 \
                    \"GH_TOKEN\": \"synthetic-value-0003\" SSH_PRIVATE_KEY=synthetic-value-0004 \
                    MAX_TOKENS=4096 csrf_token=short {\\\"LLM_API_KEY\\\": \\\"synthetic-value-0005\\\"}";
        let out = redact(text, None, &mut f);
        assert!(!out.contains("synthetic-value"), "{out}");
        assert!(
            out.contains("TINFOIL_API_KEY=[REDACTED:assigned_secret]"),
            "{out}"
        );
        assert!(
            out.contains("MAX_TOKENS=4096") && out.contains("csrf_token=short"),
            "{out}"
        );
        assert_eq!(f.by_rule["assigned_secret"], 5);

        let key = format!("tk_{}", "Ab1".repeat(16));
        let out = redact(&format!("bare {key} here"), None, &mut f);
        assert_eq!(out, "bare [REDACTED:tinfoil_key] here");

        let mut v = serde_json::json!({"TINFOIL_API_KEY": "short", "max_tokens": 10});
        redact_value(&mut v, None, &mut f);
        assert_eq!(v["TINFOIL_API_KEY"], "[REDACTED:credential_field]");
        assert_eq!(v["max_tokens"], 10);
    }

    #[test]
    fn assignment_values_are_removed_whole_and_json_stays_valid() {
        let marker = "[REDACTED:assigned_secret]";
        let json = |s: &str| -> serde_json::Value {
            let mut f = Findings::default();
            let out = redact(s, None, &mut f);
            assert!(!out.contains("synthetic"), "{out}");
            serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"))
        };
        // Escapes inside a quoted value do not end it.
        assert_eq!(
            json(r#"{"password":"abc\\synthetic-value-1234"}"#)["password"],
            marker
        );
        assert_eq!(
            json(r#"{"password":"abcdefghij\\synthetic-tail"}"#)["password"],
            marker
        );
        assert_eq!(
            json(r#"{"password":"ab\"c synthetic-value"}"#)["password"],
            marker
        );
        // A value ending in an escaped backslash still ends at its quote, at any depth.
        let v = json(r#"{"password":"abcdefgh\\","n":1}"#);
        assert_eq!(
            (v["password"].as_str(), v["n"].as_i64()),
            (Some(marker), Some(1))
        );
        let v = json(r#"{"a":"{\"password\":\"abcdefgh\\\\\",\"n\":1}"}"#);
        let inner: serde_json::Value = serde_json::from_str(v["a"].as_str().unwrap()).unwrap();
        assert_eq!(
            (inner["password"].as_str(), inner["n"].as_i64()),
            (Some(marker), Some(1))
        );
        // A more deeply escaped quote is part of the value.
        let v = json(r#"{"a":"{\"password\":\"abc\\\"synthetic\"}"}"#);
        let inner: serde_json::Value = serde_json::from_str(v["a"].as_str().unwrap()).unwrap();
        assert_eq!(inner["password"], marker);
        // A bare value that starts with an escape is removed with it.
        let v = json(r#"{"command":"password=synthetic-value-1234 next"}"#);
        assert_eq!(v["command"], format!("password={marker} next"));
        // A bare JSON number under a quoted name gets a quoted marker.
        let v = json(r#"{"DB_PASSWORD":12345678901,"ok":true}"#);
        assert_eq!(v["DB_PASSWORD"], marker);
        assert_eq!(v["ok"], true);
        // JSON inside a string, with escaped whitespace around the colon.
        let v = json(
            r#"{"arguments":"{\"DB_PASSWORD\":\n\t \"synthetic-value-1234\", \"n\": \"x\\\\plain\"}"}"#,
        );
        let inner: serde_json::Value =
            serde_json::from_str(v["arguments"].as_str().unwrap()).expect("inner JSON valid");
        assert_eq!(inner["DB_PASSWORD"], marker);
        let v = json(r#"{"a":"{\"DB_PASSWORD\":12345678901}"}"#);
        let inner: serde_json::Value = serde_json::from_str(v["a"].as_str().unwrap()).unwrap();
        assert_eq!(inner["DB_PASSWORD"], marker);

        // Escapes right before a match: the letter stays, the JSON stays valid.
        let key = format!("tk_{}", "Ab1".repeat(16));
        let v = json(&format!(
            r#"{{"a":"x\n{key}\nalice@proton.me\n@router.get(\"/s\")"}}"#
        ));
        assert_eq!(
            v["a"],
            "x\n[REDACTED:tinfoil_key]\n[REDACTED:email]\n@router.get(\"/s\")"
        );
        // A single-quoted value stops at the double quote closing the string around it.
        let v = json(r#"{"command":"grep -v 'token:' x | head","n":1}"#);
        assert_eq!(v["n"], 1);
        let v = json(r#"{"command":"export T=$(grep '^BWS_ACCESS_TOKEN=' f)"}"#);
        assert!(
            v["command"]
                .as_str()
                .unwrap()
                .starts_with("export T=$(grep '^BWS_ACCESS_TOKEN='")
        );

        let mut f = Findings::default();
        let cases = [
            (
                "export _DB_PASSWORD=synthetic-value-1",
                "export _DB_PASSWORD=[REDACTED:assigned_secret]",
            ),
            (
                "APP__DB_PASSWORD=synthetic-value-2 x",
                "APP__DB_PASSWORD=[REDACTED:assigned_secret] x",
            ),
            (
                "DB_PASSWORD='abc synthetic-value-3' x",
                "DB_PASSWORD='[REDACTED:assigned_secret]' x",
            ),
            (
                "DB_PASSWORD: |\n  synthetic-value-4\n  more-synthetic\nnext: 1",
                "DB_PASSWORD: |\n  [REDACTED:assigned_secret]\nnext: 1",
            ),
            (
                "--api-key=synthetic-value-5",
                "--api-key=[REDACTED:assigned_secret]",
            ),
            (
                "token = '[REDACTED:tinfoil_key]'",
                "token = '[REDACTED:tinfoil_key]'",
            ),
            (
                "password = 'unclosed synthetic\nnext line",
                "password = '[REDACTED:assigned_secret]\nnext line",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(redact(input, None, &mut f), want);
        }
        let mut v = serde_json::json!({"APP__DB_PASSWORD": "short", "_token": "x"});
        redact_value(&mut v, None, &mut f);
        assert_eq!(v["APP__DB_PASSWORD"], "[REDACTED:credential_field]");
        assert_eq!(v["_token"], "[REDACTED:credential_field]");
    }

    #[test]
    fn catches_personal_and_network_identifiers() {
        let mut f = Findings::default();
        let text = "mail alice@proton.me, ssh box.tail1234.ts.net or 100.101.2.3, OTHER.tail1.TS.NET, noreply+bob@proton.me; \
                    clone git@github.com:o/r.git, from noreply@anthropic.com, not 10.0.0.1";
        let out = redact(text, None, &mut f);
        assert!(
            !out.contains("alice@") && !out.contains("ts.net") && !out.contains("100.101"),
            "{out}"
        );
        assert!(
            out.contains("git@github.com:o/r.git") && out.contains("noreply@anthropic.com"),
            "{out}"
        );
        assert!(out.contains("10.0.0.1"));
        assert!(!out.contains("TS.NET") && !out.contains("bob@"), "{out}");
        assert_eq!(f.by_rule["email"], 2);
        assert_eq!(f.by_rule["tailnet_host"], 2);
        assert_eq!(f.by_rule["tailscale_ip"], 1);
    }

    #[test]
    fn catches_slugged_addresses_and_cut_off_tailnet_names() {
        let mut f = Findings::default();
        let text = "contacts: jane-roe-gmail-com, user123-protonmail-com, J-Doe-OUTLOOK-com; \
                    ssh -L 1:2 me@box-a1b2.tail0c0f\nshot-gateway-tail0c0ffe-ts-net.png; \
                    on tailnet `tailbe9c07`, tailed and tailing stay; \
                    keep 2026-01-01-help-me, alice-drink-me, sys-subsystem-net, detail-view, \
                    git clone git@github.com:my-github-org/my-repo.git; gh api orgs/my-github-org/repos";
        let out = redact(text, None, &mut f);
        for leaked in [
            "jane-roe",
            "user123",
            "J-Doe",
            "tail0c0f",
            "a1b2",
            "tailbe9c07",
        ] {
            assert!(!out.contains(leaked), "{leaked} survived: {out}");
        }
        for kept in [
            "2026-01-01-help-me",
            "alice-drink-me",
            "sys-subsystem-net",
            "detail-view",
            "tailed and tailing",
            "git@github.com:my-github-org/my-repo.git",
            "gh api orgs/my-github-org/repos",
        ] {
            assert!(out.contains(kept), "{kept} was redacted: {out}");
        }
        assert_eq!(f.by_rule["email_slug"], 3);

        // Any domain, when the address follows: a contact list.
        let mut f = Findings::default();
        let out = redact(
            "unknown: jdoe-example-org ['jdoe@example.org'] []\npm-sync-org stays",
            None,
            &mut f,
        );
        assert!(!out.contains("jdoe"), "{out}");
        assert!(out.contains("pm-sync-org stays"), "{out}");
        assert_eq!(f.by_rule["email_slug"], 1);
    }

    #[test]
    fn redacts_identifiers_in_object_keys() {
        let mut f = Findings::default();
        let mut v = serde_json::json!({"alice@proton.me": "active", "box.tail1234.ts.net": "100.101.2.3", "plain": "x"});
        redact_value(&mut v, None, &mut f);
        let text = v.to_string();
        assert!(
            !text.contains("alice@") && !text.contains("ts.net") && !text.contains("100.101"),
            "{text}"
        );
        assert_eq!(v["plain"], "x");
        assert_eq!(v["[REDACTED:email]"], "active");
    }

    #[test]
    fn colliding_redacted_keys_keep_every_value() {
        let mut f = Findings::default();
        let mut v = serde_json::json!({
            "alice@example.com": {"active": true},
            "bob@example.com": {"active": false},
            "[REDACTED:email]": "already"
        });
        redact_value(&mut v, None, &mut f);
        let o = v.as_object().unwrap();
        assert_eq!(o.len(), 3, "{v}");
        assert!(!v.to_string().contains("@example.com"), "{v}");
        let actives: Vec<_> = o.values().filter_map(|x| x.get("active")).collect();
        assert_eq!(actives.len(), 2);
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        let mut f = Findings::default();
        let text = "The password field is empty; set token in the settings page.";
        assert_eq!(redact(text, None, &mut f), text);
        assert_eq!(f.total(), 0);
    }
}
