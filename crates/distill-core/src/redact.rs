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
        vec![
            // A whole key, or a truncated one (tool output often shows only the head).
            r(
                "private_key",
                r"-----BEGIN [A-Z ]*PRIVATE KEY-----(?:[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----|[A-Za-z0-9+/=\s]*)",
            ),
            r("anthropic_key", r"sk-ant-[A-Za-z0-9_\-]{20,}"),
            r("openai_key", r"sk-(?:proj-)?[A-Za-z0-9_\-]{20,}"),
            r("github_token", r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{30,}\b|\bgithub_pat_[A-Za-z0-9_]{40,}\b"),
            r("aws_access_key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
            r("slack_token", r"\bxox[abprs]-[A-Za-z0-9\-]{10,}\b"),
            r("google_api_key", r"\bAIza[0-9A-Za-z_\-]{35}\b"),
            r("stripe_key", r"\b(?:sk|rk)_(?:live|test)_[0-9A-Za-z]{20,}\b"),
            r("jwt", r"\beyJ[A-Za-z0-9_\-]{10,}\.eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b"),
            r("nostr_secret", r"\bnsec1[02-9ac-hj-np-z]{58}\b"),
            r("bearer", r"(?i)\bbearer\s+[A-Za-z0-9_\-\.=]{20,}"),
            // Personal data and private network names, not only credentials.
            r("email", r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"),
            r("tailnet_host", r"(?i)\b[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.ts\.net\b"),
            r(
                "tailscale_ip",
                r"\b100\.(?:6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.[0-9]{1,3}\.[0-9]{1,3}\b",
            ),
            r(
                "assigned_secret",
                // Only the value is replaced (the `keep` group stays), so JSON stays valid.
                r#"(?i)(?P<keep>\b(?:password|passwd|secret|token|api[_-]?key|access[_-]?token|auth[_-]?token)\b["']?\s*[:=]\s*["']?)[^\s"',}]{8,}"#,
            ),
        ]
    })
}

/// Changes whenever a rule changes, so dataset builds made with old rules are not reused.
pub fn fingerprint() -> String {
    let joined: Vec<String> = rules()
        .iter()
        .map(|r| format!("{}={}", r.name, r.re.as_str()))
        .collect();
    crate::util::sha256_hex(joined.join("\n").as_bytes())
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
        let replaced = rule.re.replace_all(&out, |c: &regex::Captures| {
            let whole = &c[0];
            if rule.name == "email" && is_service_address(whole) {
                return whole.to_string();
            }
            n += 1;
            let keep = c.name("keep").map_or("", |k| k.as_str());
            format!("{keep}[REDACTED:{}]", rule.name)
        });
        if n > 0 {
            *findings.by_rule.entry(rule.name.to_string()).or_default() += n;
            out = replaced.into_owned();
        }
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
        Regex::new(r"(?i)^(?:password|passwd|pass|secret|client[_-]?secret|token|access[_-]?token|refresh[_-]?token|auth[_-]?token|api[_-]?key|apikey|private[_-]?key|authorization|credentials?)$")
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
