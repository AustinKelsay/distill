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
pub fn redact(text: &str, home_dir: Option<&str>, findings: &mut Findings) -> String {
    let mut out = text.to_string();
    for rule in rules() {
        let n = rule.re.find_iter(&out).count() as u64;
        if n > 0 {
            *findings.by_rule.entry(rule.name.to_string()).or_default() += n;
            out = rule
                .re
                .replace_all(&out, format!("${{keep}}[REDACTED:{}]", rule.name))
                .into_owned();
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
    fn leaves_ordinary_text_alone() {
        let mut f = Findings::default();
        let text = "The password field is empty; set token in the settings page.";
        assert_eq!(redact(text, None, &mut f), text);
        assert_eq!(f.total(), 0);
    }
}
