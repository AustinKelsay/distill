//! Secret scanning for exports. Raw captures are never edited; this runs on the way out.

use std::collections::{BTreeMap, HashMap, HashSet};
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
        // A key's start: a word boundary, or an escaped newline, tab, backspace or form feed
        // (`\ntk_…` in JSON inside a string, where `\b` finds no boundary between the `n` and
        // the key).
        let k = |name, pat: &str| r(name, &format!(r"(?:\b|(?P<keep>\\[nrtbf])){pat}"));
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
/// any prefix, since a bare `\b` would stop at the `_`, and may follow an escaped newline
/// (`\npassword=`), where `\b` finds no boundary after the `n`. `scan_value` finds where the value ends.
const ASSIGNMENT_HEAD: &str = r#"(?i)(?:\b|\\+[nrtbf])(?P<name>(?:[A-Za-z0-9]*[_-]+)*(?:password|passwd|secret|client[_-]?secret|token|api[_-]?key|access[_-]?token|auth[_-]?token|private[_-]?key))\b(?P<nq>\\*["'])?(?:\s|\\+[nrt])*[:=](?:\s|\\+[nrt])*(?P<q>\\*["'])?"#;
/// Bump when `scan_value` changes, so builds made by the old scanner are not reused.
const ASSIGNMENT_SCANNER: u32 = 24;
/// Shorter values are rarely secrets (`csrf_token=short`, `max_token=4096`).
const MIN_SECRET_CHARS: usize = 8;

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
/// redacted JSON (at any nesting depth) stays valid. Every assignment is read, also one inside
/// another's value, and the union of their values is redacted: an assignment inside a quoted
/// value (`password='token=x abc'`) can neither hide the outer value nor be hidden by it.
fn redact_assignments(text: &str, n: &mut u64) -> String {
    // In valid JSON, every unescaped `"` delimits a string. In other text it may be a
    // character of the value (`DB_PASSWORD='abc"def'`).
    let t = text.trim_start();
    let json_text = (t.starts_with('{') || t.starts_with('['))
        && serde_json::from_str::<serde::de::IgnoredAny>(text).is_ok();
    let mut memo = Memo::default();
    // Each value to redact, and the quote its marker needs (see below).
    let mut ranges: Vec<(usize, usize, String)> = Vec::new();
    // The start of the current line, found incrementally so a long line is read once.
    let (mut line_start, mut seen) = (0, 0);
    // The current line's leading run of indent, measured once per line.
    let mut indent_of: Option<(usize, usize)> = None;
    // Scanning normally reads each byte a few times. Only adversarial text (thousands of
    // nested or chained candidates) costs more; past this budget, everything from the next
    // assignment on is redacted whole, which can over-redact but never leak.
    let budget = 64 * text.len() + 4096;
    let mut cur = assignment_head().captures_at(text, 0);
    while let Some(c) = cur {
        let whole = c.get(0).expect("whole match");
        let (head_start, head_end) = (whole.start(), whole.end());
        // The next name may start inside this one's separator (`token=\npassword:`), so it is
        // looked for from the end of this name.
        let name_end = c.name("name").expect("name group").end();
        let next_c = assignment_head().captures_at(text, name_end);
        if let Some(nl) = text[seen..head_start].rfind('\n') {
            line_start = seen + nl + 1;
        }
        seen = head_start;
        // A YAML sequence entry's `- ` counts as indent: its fields line up after it.
        let indent = match indent_of {
            Some((at, n)) if at == line_start => n,
            _ => {
                let n = text.as_bytes()[line_start..]
                    .iter()
                    .take_while(|c| matches!(c, b' ' | b'\t' | b'-'))
                    .count();
                indent_of = Some((line_start, n));
                n
            }
        };
        let quote = c.name("q").map(|q| q.as_str());
        let nq = c.name("nq").map(|q| q.as_str());
        let ctx = Context {
            json_text,
            json_key: nq.is_some_and(|q| q.ends_with('"')),
            key_indent: indent.min(head_start - line_start),
        };
        if memo.work > budget {
            ranges.push((head_end, text.len(), String::new()));
            break;
        }
        if let Some((s, e)) = scan_value(text, head_end, quote, &ctx, &mut memo)
            && s <= e
            && e <= text.len()
            && text.is_char_boundary(s)
            && text.is_char_boundary(e)
            // Counted only as far as needed: many candidates can share one long value.
            && text[s..e].chars().nth(MIN_SECRET_CHARS - 1).is_some()
            && !is_marker(&text[s..e])
        {
            // A bare value after a quoted name is a JSON number or literal; quote the marker
            // the same way the name is quoted, or the JSON breaks. Only the backslashes that
            // escape the quote; any before them belong to the name.
            let wrap = match (quote, nq) {
                (None, Some(q)) if ctx.json_key && s == head_end => {
                    format!("{}\"", "\\".repeat(own_escapes(q.len() - 1)))
                }
                _ => String::new(),
            };
            ranges.push((s, e, wrap));
        }
        cur = next_c;
    }
    ranges.sort_by_key(|r| r.0);
    let mut out = String::with_capacity(text.len());
    let (mut pos, mut i) = (0, 0);
    while i < ranges.len() {
        let (s, mut e, wrap) = ranges[i].clone();
        let mut merged = false;
        i += 1;
        while i < ranges.len() && ranges[i].0 < e {
            e = e.max(ranges[i].1);
            merged = true;
            i += 1;
        }
        let wrap = if merged { "" } else { wrap.as_str() };
        *n += 1;
        out.push_str(&text[pos..s]);
        out.push_str(&format!("{wrap}[REDACTED:assigned_secret]{wrap}"));
        pos = e;
    }
    out.push_str(&text[pos..]);
    out
}

/// A YAML block: the line break before it, its key's indent, and its range (if not empty).
type Block = (usize, usize, Option<(usize, usize)>);

/// What earlier scans of the same text found, so a run of candidates is read once. Every
/// entry comes from a scan to the end of the text, so it holds for any later offset.
#[derive(Default)]
struct Memo {
    /// For each kind of opening quote, the earliest offset after which no closing quote exists.
    unclosed: Vec<(String, usize)>,
    /// (searched from, the first line break at or after it) for unclosed values.
    line_break: Option<(usize, usize)>,
    /// (searched from, the next `\n`, if any).
    newline: Option<(usize, Option<usize>)>,
    /// The last YAML block read, for candidates that share its header line.
    block: Option<Block>,
    /// Bytes scanned so far, against the budget in `redact_assignments`.
    work: usize,
}

impl Memo {
    /// The end of the bare word at `at`.
    fn word_end(&mut self, b: &[u8], at: usize, ctx: &Context) -> usize {
        let len = bare_end(&b[at..], ctx);
        self.work += len + 1;
        at + len
    }
    fn next_newline(&mut self, b: &[u8], at: usize) -> Option<usize> {
        match self.newline {
            Some((from, found)) if from <= at && found.is_none_or(|f| at <= f) => found,
            _ => {
                let found = b[at..].iter().position(|&c| c == b'\n').map(|p| at + p);
                self.newline = Some((at, found));
                found
            }
        }
    }
}

/// A whole marker (`[REDACTED:email]`), not a value that only starts with one.
fn is_marker(v: &str) -> bool {
    v.len() <= 64
        && v.strip_prefix("[REDACTED:")
            .and_then(|r| r.strip_suffix(']'))
            .is_some_and(|name| {
                !name.is_empty() && name.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
            })
}

/// Whether an escaped double quote followed by `after` ends a JSON string: JSON punctuation (or the end
/// of the text) comes next, possibly after whitespace or escaped whitespace.
fn closes_json_string(after: &[u8]) -> bool {
    let mut i = 0;
    loop {
        match after.get(i) {
            None | Some(b',' | b'}' | b']' | b':') => return true,
            Some(b' ' | b'\t' | b'\r' | b'\n') => i += 1,
            Some(b'\\') => {
                let m = after[i..].iter().take_while(|&&x| x == b'\\').count();
                match after.get(i + m) {
                    Some(b'n' | b'r' | b't') => i += m + 1,
                    _ => return false,
                }
            }
            Some(_) => return false,
        }
    }
}

/// How many of `m` backslashes before a quote escape the quote itself (the rest are escaped
/// backslashes): `2^level - 1`.
fn own_escapes(m: usize) -> usize {
    1usize.checked_shl(level(m)).map_or(usize::MAX, |x| x - 1)
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

/// Where an assignment sits in the text being redacted.
struct Context {
    /// The whole text is valid JSON.
    json_text: bool,
    /// The name was a quoted JSON key, so a bare value is a JSON number, literal or container.
    json_key: bool,
    /// Indent of the line the name is on; a YAML block's lines are indented more.
    key_indent: usize,
}

/// The value after the separator ending at `at`, as a byte range of `text`. `quote` is the
/// opening quote with the backslashes that escape it. A value runs to its real end, even over
/// text that looks like another assignment (`password=token=x`, `token=x,password='y'`):
/// every assignment is also read on its own, and the union of their values is redacted.
fn scan_value(
    text: &str,
    at: usize,
    quote: Option<&str>,
    ctx: &Context,
    memo: &mut Memo,
) -> Option<(usize, usize)> {
    let b = text.as_bytes();
    if let Some(q) = quote {
        let escapes = q.len() - 1;
        let mark = q.as_bytes()[escapes];
        let lvl = level(escapes);
        // Backslashes before the opening quote that are escaped backslashes, not the quote's
        // own escapes (`\\"`), say nothing about the value; only the level counts.
        let own = own_escapes(escapes);
        let known_unclosed = memo.unclosed.iter().any(|(k, from)| k == q && *from <= at);
        if !known_unclosed {
            // Closed by the same quote at the same level; a shallower one means the string
            // around it closed first (the text was cut). A deeper one is part of the value, and
            // so are line breaks: a quoted secret may span lines.
            let mut i = at;
            while i < b.len() {
                let m = b[i..].iter().take_while(|&&x| x == b'\\').count();
                memo.work += m + 1;
                let Some(&x) = b.get(i + m) else { break };
                if x == b'\''
                    && mark == b'\''
                    && (lvl == 0 || m == 0)
                    && b.get(i + m + 1) == Some(&b'\'')
                {
                    // `''` is an apostrophe inside a YAML single-quoted string (where a backslash
                    // before it is a plain character), and joins two quoted pieces in a shell:
                    // either way the value goes on. After an escaped opening quote (`\'`), a
                    // `\''` is that quote closing, then the next one opening.
                    i += m + 2;
                    continue;
                }
                if x == mark {
                    match level(m).cmp(&lvl) {
                        std::cmp::Ordering::Equal => {
                            let close = i + m - own;
                            // In a shell, `"abc"def` is one word: the value goes on.
                            if own == 0 && continues_word(b.get(close + 1).copied(), ctx) {
                                // The whole word goes, opening quote included, so no quote is
                                // left unmatched.
                                let end = memo.word_end(b, close + 1, ctx);
                                return Some((at - q.len(), char_end(text, end)));
                            }
                            return Some((at, close));
                        }
                        std::cmp::Ordering::Less => return Some((at, i)),
                        std::cmp::Ordering::Greater => {}
                    }
                } else if x == b'"'
                    && mark == b'\''
                    && ctx.json_text
                    && (m == 0 || closes_json_string(&b[i + m + 1..]))
                {
                    // In valid JSON, a double quote that closes the string around the
                    // single-quoted value: an unescaped one, or an escaped one followed by JSON
                    // punctuation (JSON inside a string). Outside JSON a backslash and a double
                    // quote are characters of the value (`'abc\",def'` in a shell).
                    return Some((at, i));
                }
                i += m + 1;
            }
            memo.unclosed.push((q.to_string(), at));
        }
        // Never closed: the text was cut mid-value, so the value is the rest of its line.
        if let Some((from, brk)) = memo.line_break
            && from <= at
            && at <= brk
        {
            return Some((at, brk));
        }
        let brk = (at..b.len())
            .find(|&i| {
                b[i] == b'\n'
                    || (b[i] == b'n'
                        && b[at..i].iter().rev().take_while(|&&x| x == b'\\').count() % 2 == 1)
            })
            .map_or(b.len(), |i| {
                i - b[at..i].iter().rev().take_while(|&&x| x == b'\\').count()
            });
        memo.work += brk - at;
        memo.line_break = Some((at, brk));
        return Some((at, brk));
    }
    // A YAML block scalar (`KEY: |`, `KEY: >- # note`): the indented lines that follow, blank
    // lines among them included, are the value. The header is the indicator, its chomping and
    // indent digits, spaces, then a comment or the line break, checked byte by byte.
    let run = |from: usize, f: fn(&u8) -> bool| {
        b[from.min(b.len())..].iter().take_while(|c| f(c)).count()
    };
    if b.get(at).is_some_and(|c| matches!(c, b'|' | b'>'))
        && let h = at + 1 + run(at + 1, |c| matches!(c, b'-' | b'+' | b'0'..=b'9'))
        && matches!(
            b.get(h + run(h, |c| matches!(c, b' ' | b'\t'))),
            Some(b'#' | b'\r' | b'\n')
        )
        && let Some(nl) = memo.next_newline(b, at)
    {
        if let Some((n, k, block)) = memo.block
            && n == nl
            && k == ctx.key_indent
        {
            return block;
        }
        let start = nl + 1;
        let (mut end, mut off) = (start, start);
        for line in text[start..].split_inclusive('\n') {
            off += line.len();
            if line.trim().is_empty() {
                continue;
            }
            // A block's lines are indented more than its key; the first line that is not is
            // the next key.
            let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
            if indent <= ctx.key_indent {
                break;
            }
            end = off;
        }
        let body = text[start..end].trim_end();
        let indent = body.len() - body.trim_start().len();
        let block = (!body.trim().is_empty()).then_some((start + indent, start + body.len()));
        // Every line read counts, the one that ended the block included.
        memo.work += off - start;
        memo.block = Some((nl, ctx.key_indent, block));
        return block;
    }
    // A bare token. A backslash ends it only when it escapes a quote or whitespace; any other
    // escape (`\u0073`, `\\`) is part of the value, so no half of it is left behind. After a
    // quoted JSON key, a container is not a value to replace, and `]` ends a number in an array.
    if ctx.json_key && matches!(b.get(at), Some(b'[' | b'{')) {
        return None;
    }
    Some((at, char_end(text, memo.word_end(b, at, ctx))))
}

/// `i`, or the next char boundary after it.
fn char_end(s: &str, i: usize) -> usize {
    let mut end = i.min(s.len());
    while !s.is_char_boundary(end) {
        end += 1;
    }
    end
}

/// Whether a quote followed by `next` sits inside a shell word (`abc"def"`, `"abc"def`) rather
/// than ending it (`say "password=abcdefgh" now`, `"…".`). Never in JSON, where a raw quote
/// delimits a string.
fn continues_word(next: Option<u8>, ctx: &Context) -> bool {
    !ctx.json_text
        && !ctx.json_key
        && next.is_some_and(|c| {
            !c.is_ascii_whitespace()
                && !matches!(
                    c,
                    b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']' | b'}' | b'>'
                )
        })
}

/// The end of a bare token in `b`.
fn bare_end(b: &[u8], ctx: &Context) -> usize {
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b' ' | b'\t' | b'\n' | b'\r' => break,
            // A quoted piece of the same shell word (`abc"def"`): through its closing quote, if
            // that is on this line.
            q @ (b'"' | b'\'') if continues_word(b.get(i + 1).copied(), ctx) => {
                // In double quotes a backslash escapes the next character (`"a\"b"`); in
                // single quotes it does not. An escaped `\n` ends the line, as in text that is
                // itself escaped, so the piece never runs on across lines.
                let mut j = i + 1;
                let close = loop {
                    match b.get(j) {
                        None | Some(b'\n') => break None,
                        Some(b'\\') if b.get(j + 1) == Some(&b'n') => break None,
                        Some(b'\\') if q == b'"' => j += 2,
                        Some(&c) if c == q => break Some(j),
                        _ => j += 1,
                    }
                };
                match close {
                    Some(j) => i = j + 1,
                    // Never closed on this line (the text was cut): the piece is still part of
                    // the value, up to where the line ends.
                    None => {
                        i = j.min(b.len());
                        break;
                    }
                }
            }
            b'"' | b'\'' => break,
            // JSON punctuation ends a value only after a quoted JSON key; in a shell or config
            // line it is part of the value (`DB_PASSWORD=abc}def`).
            b',' | b'}' | b']' if ctx.json_key => break,
            b'\\' => match b.get(i + 1) {
                None | Some(b'"' | b'\'' | b'n' | b'r' | b't') => break,
                Some(_) => i += 2,
            },
            _ => i += 1,
        }
    }
    i.min(b.len())
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

/// How many levels of JSON inside JSON strings are read token by token; deeper text goes to the
/// text scanner rather than risk the stack.
const MAX_JSON_DEPTH: u32 = 64;

pub fn redact(text: &str, home_dir: Option<&str>, findings: &mut Findings) -> String {
    thread_local!(static DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) });
    let depth = DEPTH.with(|d| d.replace(d.get() + 1));
    let json = (depth < MAX_JSON_DEPTH)
        .then(|| redact_json_text(text, home_dir, findings))
        .flatten();
    DEPTH.with(|d| d.set(depth));
    json.unwrap_or_else(|| redact_text(text, home_dir, findings))
}

/// Text that is JSON (an object, array or string: tool arguments, a tool's JSON output) is
/// redacted token by token, so no pattern has to read escaped JSON. Each string is decoded,
/// redacted (a string that is JSON itself recurses) and re-encoded only if it changed; a value
/// under a credential-named key is replaced whole. Every other byte stays as it was: numbers,
/// key order, whitespace, and every occurrence of a duplicate key.
fn redact_json_text(text: &str, home_dir: Option<&str>, findings: &mut Findings) -> Option<String> {
    if !text.trim_start().starts_with(['{', '[', '"']) {
        return None;
    }
    let tokens = json_tokens(text)?;
    // Per object, the keys already in use, and the next suffix to try for each redacted key.
    // Built in one pass, the first time a key is redacted.
    let mut taken: Option<HashMap<usize, KeyNames>> = None;
    let mut out = String::new();
    let mut pos = 0;
    for tok in &tokens {
        let raw = &text[tok.start..tok.end];
        let new = match &tok.role {
            Role::Key { obj, name } => {
                let base = redact(name, home_dir, findings);
                (base != *name).then(|| {
                    let taken = taken.get_or_insert_with(|| {
                        let mut by_obj: HashMap<usize, KeyNames> = HashMap::new();
                        for t in &tokens {
                            if let Role::Key { obj, name } = &t.role {
                                by_obj.entry(*obj).or_default().0.insert(name.clone());
                            }
                        }
                        by_obj
                    });
                    let (used, next) = taken.entry(*obj).or_default();
                    encode(&unique_key(base, used, next))
                })
            }
            Role::Secret => {
                *findings
                    .by_rule
                    .entry("credential_field".into())
                    .or_default() += 1;
                Some(encode("[REDACTED:credential_field]"))
            }
            Role::Str => {
                let s = decode(raw);
                let r = redact(&s, home_dir, findings);
                (r != s).then(|| encode(&r))
            }
        };
        if let Some(new) = new {
            out.push_str(&text[pos..tok.start]);
            out.push_str(&new);
            pos = tok.end;
        }
    }
    if pos == 0 {
        return Some(text.to_string());
    }
    out.push_str(&text[pos..]);
    Some(out)
}

/// An object's keys in use, and the next suffix to try for each redacted key.
type KeyNames = (HashSet<String>, HashMap<String, usize>);

/// Decode a JSON string token (quotes included, already checked by `string_end`). A lone
/// surrogate (`\ud800`), which no Rust string can hold, becomes U+FFFD instead of failing, so
/// one odd escape does not send a whole document to the text scanner.
fn decode(raw: &str) -> String {
    let inner = &raw[1..raw.len() - 1];
    if !inner.contains('\\') {
        return inner.to_string();
    }
    let b = inner.as_bytes();
    let hex = |i: usize| {
        inner
            .get(i..i + 4)
            .and_then(|h| u32::from_str_radix(h, 16).ok())
    };
    let (mut out, mut i, mut run) = (String::with_capacity(inner.len()), 0, 0);
    while i < b.len() {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        out.push_str(&inner[run..i]);
        let c = match b[i + 1] {
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let hi = hex(i + 2).unwrap_or(0xFFFD);
                let pair = (0xD800..0xDC00).contains(&hi)
                    && b.get(i + 6..i + 8) == Some(b"\\u")
                    && hex(i + 8).is_some_and(|lo| (0xDC00..0xE000).contains(&lo));
                if pair {
                    let lo = hex(i + 8).expect("checked");
                    i += 6;
                    char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))
                } else {
                    char::from_u32(hi)
                }
                .unwrap_or('\u{FFFD}')
            }
            other => other as char,
        };
        out.push(c);
        i += if b[i + 1] == b'u' { 6 } else { 2 };
        run = i;
    }
    out.push_str(&inner[run..]);
    out
}

fn encode(s: &str) -> String {
    serde_json::to_string(s).expect("a string serializes")
}

/// `base`, or `base #2`, `base #3`, … when two keys redact to the same placeholder, so no value
/// is dropped. `next` remembers where each base's numbering got to.
fn unique_key(
    base: String,
    used: &mut HashSet<String>,
    next: &mut HashMap<String, usize>,
) -> String {
    let mut key = base.clone();
    if used.contains(&key) {
        let n = next.entry(base.clone()).or_insert(2);
        loop {
            key = format!("{base} #{n}");
            *n += 1;
            if !used.contains(&key) {
                break;
            }
        }
    }
    used.insert(key.clone());
    key
}

/// A JSON token that may need replacing.
struct Tok {
    start: usize,
    end: usize,
    role: Role,
}

enum Role {
    /// An object key, decoded, and which object it belongs to.
    Key { obj: usize, name: String },
    /// A non-empty string, or a number of 1e7 or more, under a credential-named key.
    Secret,
    /// Any other string.
    Str,
}

/// The strings (and secret numbers) of a JSON document, with their byte ranges; `None` unless
/// the whole text is valid JSON. Iterative, so nesting depth costs no stack.
fn json_tokens(text: &str) -> Option<Vec<Tok>> {
    // `secret`: the value being read sits under a credential-named key. Arrays pass that on
    // (`"password": ["…"]`); objects do not, since a tool schema's `"password": {"type": …}`
    // describes a field rather than holding one.
    enum Frame {
        Obj { id: usize, secret: bool },
        Arr { secret: bool },
    }
    #[derive(PartialEq)]
    enum Want {
        Value,
        ValueOrEnd,
        Key,
        KeyOrEnd,
        Colon,
        CommaOrEnd,
    }
    let b = text.as_bytes();
    let (mut toks, mut stack, mut objs) = (Vec::new(), Vec::new(), 0);
    let (mut want, mut i) = (Want::Value, 0);
    loop {
        while matches!(b.get(i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            i += 1;
        }
        let Some(&c) = b.get(i) else {
            return (stack.is_empty() && want == Want::CommaOrEnd).then_some(toks);
        };
        match want {
            Want::Value | Want::ValueOrEnd => {
                if want == Want::ValueOrEnd && c == b']' {
                    stack.pop();
                    i += 1;
                    want = Want::CommaOrEnd;
                    continue;
                }
                let secret = matches!(
                    stack.last(),
                    Some(Frame::Obj { secret: true, .. } | Frame::Arr { secret: true })
                );
                match c {
                    b'"' => {
                        let end = string_end(b, i)?;
                        let role = if secret && end - i > 2 {
                            Role::Secret
                        } else {
                            Role::Str
                        };
                        toks.push(Tok {
                            start: i,
                            end,
                            role,
                        });
                        i = end;
                    }
                    b'{' => {
                        objs += 1;
                        stack.push(Frame::Obj {
                            id: objs,
                            secret: false,
                        });
                        i += 1;
                        want = Want::KeyOrEnd;
                        continue;
                    }
                    b'[' => {
                        stack.push(Frame::Arr { secret });
                        i += 1;
                        want = Want::ValueOrEnd;
                        continue;
                    }
                    b't' | b'f' | b'n' => {
                        let lit = [&b"true"[..], b"false", b"null"]
                            .into_iter()
                            .find(|l| b[i..].starts_with(l))?;
                        i += lit.len();
                    }
                    b'-' | b'0'..=b'9' => {
                        let end = number_end(b, i)?;
                        if secret && text[i..end].parse::<f64>().is_ok_and(|f| f.abs() >= 1e7) {
                            toks.push(Tok {
                                start: i,
                                end,
                                role: Role::Secret,
                            });
                        }
                        i = end;
                    }
                    _ => return None,
                }
                want = Want::CommaOrEnd;
            }
            Want::Key | Want::KeyOrEnd => {
                if want == Want::KeyOrEnd && c == b'}' {
                    stack.pop();
                    i += 1;
                    want = Want::CommaOrEnd;
                    continue;
                }
                if c != b'"' {
                    return None;
                }
                let end = string_end(b, i)?;
                let name = decode(&text[i..end]);
                let Some(Frame::Obj { id, secret }) = stack.last_mut() else {
                    return None;
                };
                // Classified by the key as written, before the key itself is redacted.
                *secret = is_credential_key(&name);
                toks.push(Tok {
                    start: i,
                    end,
                    role: Role::Key { obj: *id, name },
                });
                i = end;
                want = Want::Colon;
            }
            Want::Colon => {
                if c != b':' {
                    return None;
                }
                i += 1;
                want = Want::Value;
            }
            Want::CommaOrEnd => {
                match (stack.last(), c) {
                    (Some(Frame::Obj { .. }), b',') => want = Want::Key,
                    (Some(Frame::Arr { .. }), b',') => want = Want::Value,
                    (Some(Frame::Obj { .. }), b'}') | (Some(Frame::Arr { .. }), b']') => {
                        stack.pop();
                    }
                    _ => return None,
                }
                i += 1;
            }
        }
    }
}

/// The end (past the closing quote) of the JSON string starting at `b[i]`.
fn string_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    loop {
        match *b.get(j)? {
            b'"' => return Some(j + 1),
            b'\\' => match *b.get(j + 1)? {
                b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => j += 2,
                b'u' if b.get(j + 2..j + 6)?.iter().all(u8::is_ascii_hexdigit) => j += 6,
                _ => return None,
            },
            0..0x20 => return None,
            _ => j += 1,
        }
    }
}

/// The end of the JSON number starting at `b[i]`.
fn number_end(b: &[u8], i: usize) -> Option<usize> {
    let digits = |j: &mut usize| {
        let s = *j;
        while b.get(*j).is_some_and(u8::is_ascii_digit) {
            *j += 1;
        }
        *j > s
    };
    let mut j = i + usize::from(b[i] == b'-');
    match *b.get(j)? {
        b'0' => j += 1,
        b'1'..=b'9' => {
            digits(&mut j);
        }
        _ => return None,
    }
    if b.get(j) == Some(&b'.') {
        j += 1;
        if !digits(&mut j) {
            return None;
        }
    }
    if matches!(b.get(j), Some(b'e' | b'E')) {
        j += 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if !digits(&mut j) {
            return None;
        }
    }
    Some(j)
}

/// Redact text that is not JSON, or JSON that does not parse (a truncated snippet).
fn redact_text(text: &str, home_dir: Option<&str>, findings: &mut Findings) -> String {
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
            // Values are classified by their keys as written, before the keys are redacted:
            // keys can carry identifiers too (`{"alice@proton.me": "active"}`).
            let old = std::mem::take(o);
            let originals: Vec<String> = old.keys().cloned().collect();
            let mut used: Option<HashSet<String>> = None;
            let mut next = HashMap::new();
            for (k, mut x) in old {
                if is_credential_key(&k) {
                    redact_credential(&mut x, home_dir, findings);
                } else {
                    redact_value(&mut x, home_dir, findings);
                }
                let base = redact(&k, home_dir, findings);
                let key = if base == k {
                    k
                } else {
                    // A redacted key never takes an original key's name, or that value is lost.
                    let used = used.get_or_insert_with(|| originals.iter().cloned().collect());
                    unique_key(base, used, &mut next)
                };
                o.insert(key, x);
            }
        }
        _ => {}
    }
}

/// A value under a credential-named key: a non-empty string or a number of 1e7 or more is
/// replaced whole, and so is each one in an array (`"password": ["…"]`). An object is
/// redacted as usual: a tool schema's `"password": {"type": "string"}` holds no secret.
fn redact_credential(x: &mut serde_json::Value, home_dir: Option<&str>, findings: &mut Findings) {
    let secret = match x {
        serde_json::Value::String(s) => !s.is_empty(),
        serde_json::Value::Number(n) => n.as_f64().is_some_and(|f| f.abs() >= 1e7),
        serde_json::Value::Array(a) => {
            a.iter_mut()
                .for_each(|y| redact_credential(y, home_dir, findings));
            return;
        }
        _ => false,
    };
    if secret {
        *findings
            .by_rule
            .entry("credential_field".into())
            .or_default() += 1;
        *x = serde_json::Value::String("[REDACTED:credential_field]".to_string());
    } else {
        redact_value(x, home_dir, findings);
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
        assert_eq!(parsed["token"], "[REDACTED:credential_field]");
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
        // Valid JSON is redacted field by field; the text scanner is the fallback for JSON that
        // does not parse (a cut-off snippet), so run both and check each. The scanner's result
        // is returned.
        let json = |s: &str| -> serde_json::Value {
            let mut f = Findings::default();
            for out in [redact(s, None, &mut f), redact_text(s, None, &mut f)] {
                assert!(!out.contains("synthetic"), "{out}");
                serde_json::from_str::<serde_json::Value>(&out)
                    .unwrap_or_else(|e| panic!("{e}: {out}"));
            }
            serde_json::from_str(&redact_text(s, None, &mut f)).expect("checked above")
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
        let v = json(r#"{"command":"password=\u0073ynthetic-value-1234 next"}"#);
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

        // Escaped whitespace two levels down.
        let v = json(r#"{"a":"{\"b\":\"{\\\"DB_PASSWORD\\\":\\n\\t\\\"synthetic-value\\\"}\"}"}"#);
        let b: serde_json::Value = serde_json::from_str(v["a"].as_str().unwrap()).unwrap();
        let c: serde_json::Value = serde_json::from_str(b["b"].as_str().unwrap()).unwrap();
        assert_eq!(c["DB_PASSWORD"], marker);
        // A value ending in a literal backslash, and one past 8 KiB.
        let v = json(r#"{"password":"synthetic-value\\","ok":true}"#);
        assert_eq!(
            (v["password"].as_str(), v["ok"].as_bool()),
            (Some(marker), Some(true))
        );
        let long =
            serde_json::json!({"private_key": format!("{}synthetic-tail", "a".repeat(9000))});
        assert_eq!(json(&long.to_string())["private_key"], marker);
        // A quoted secret spanning lines, in a shell and in JSON.
        let v = json(r#"{"command":"DB_PASSWORD=\"synthetic-first\nsynthetic-second\" x"}"#);
        assert_eq!(v["command"], format!("DB_PASSWORD=\"{marker}\" x"));
        // A single quote inside a nested JSON string stops at that string's closing quote.
        let v = json(r#"{"a":"{\"cmd\":\"DB_PASSWORD='synthetic-value\",\"ok\":true}"}"#);
        let inner: serde_json::Value = serde_json::from_str(v["a"].as_str().unwrap()).unwrap();
        assert_eq!(inner["ok"], true);
        // A JSON array under a secret name stays an array.
        let mut f = Findings::default();
        let out = redact(r#"{"token":[12345678,2],"ok":true}"#, None, &mut f);
        serde_json::from_str::<serde_json::Value>(&out).unwrap_or_else(|e| panic!("{e}: {out}"));

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
                "DB_PASSWORD='[REDACTED:any]synthetic-secret' x",
                "DB_PASSWORD='[REDACTED:assigned_secret]' x",
            ),
            (
                "DB_PASSWORD='alice@example.com synthetic-secret' x",
                "DB_PASSWORD='[REDACTED:assigned_secret]' x",
            ),
            (
                "DB_PASSWORD='synthetic-first\nsynthetic-second' x",
                "DB_PASSWORD='[REDACTED:assigned_secret]' x",
            ),
            (
                "DB_PASSWORD: | # note\n  synthetic-secret\nnext: 1",
                "DB_PASSWORD: | # note\n  [REDACTED:assigned_secret]\nnext: 1",
            ),
            (
                "DB_PASSWORD: |\n  synthetic-first\n\n  synthetic-second\nnext: 1",
                "DB_PASSWORD: |\n  [REDACTED:assigned_secret]\nnext: 1",
            ),
            (
                "DB_PASSWORD='abc\"synthetic-secret-tail' x",
                "DB_PASSWORD='[REDACTED:assigned_secret]' x",
            ),
            (
                "config:\n  password: |\n    synthetic-secret\n  user: alice",
                "config:\n  password: |\n    [REDACTED:assigned_secret]\n  user: alice",
            ),
            (
                "DB_PASSWORD='abc\",synthetic-secret-tail' x",
                "DB_PASSWORD='[REDACTED:assigned_secret]' x",
            ),
            ("password: |\nuser: alice", "password: |\nuser: alice"),
            (
                "  password: |\n  user: alice",
                "  password: |\n  user: alice",
            ),
            (
                "password: |2\n    more-indented\n  synthetic-secret\nnext: 1",
                "password: |2\n    [REDACTED:assigned_secret]\nnext: 1",
            ),
            (
                "DB_PASSWORD='abc\\\",synthetic-secret-tail' x",
                "DB_PASSWORD='[REDACTED:assigned_secret]' x",
            ),
            (
                "- password: |\n    synthetic-secret\n  user: alice",
                "- password: |\n    [REDACTED:assigned_secret]\n  user: alice",
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
    fn json_text_is_redacted_field_by_field_at_any_depth() {
        let mut f = Findings::default();
        // Wrap a command three JSON levels deep; every level must still parse afterwards.
        let mut s =
            serde_json::json!({"cmd": "DB_PASSWORD=synthetic-value", "ok": true}).to_string();
        for _ in 0..2 {
            s = serde_json::json!({ "a": s }).to_string();
        }
        let out = redact(&s, None, &mut f);
        assert!(!out.contains("synthetic"), "{out}");
        let l1: serde_json::Value = serde_json::from_str(&out).unwrap();
        let l2: serde_json::Value = serde_json::from_str(l1["a"].as_str().unwrap()).unwrap();
        let l3: serde_json::Value = serde_json::from_str(l2["a"].as_str().unwrap()).unwrap();
        assert_eq!(l3["ok"], true);

        // YAML and escaped newlines inside a JSON string are read decoded.
        let yaml = r#"{"content":"DB_PASSWORD: | # note\n  synthetic-first\n\n  synthetic-second\nnext: 1"}"#;
        let line = r#"{"cmd":"echo x\npassword=synthetic-value"}"#;
        for s in [yaml, line] {
            let out = redact(s, None, &mut f);
            assert!(!out.contains("synthetic"), "{out}");
            serde_json::from_str::<serde_json::Value>(&out).unwrap();
        }
        // A name right after an escaped newline, in text that is not valid JSON.
        let out = redact(
            r#"cut off: {"cmd":"echo x\npassword=synthetic-value"#,
            None,
            &mut f,
        );
        assert!(!out.contains("synthetic"), "{out}");

        // Odd escape counts must not panic, on either path, multibyte text included.
        for s in [
            r#"{"cmd":"password=\\","z":1}"#,
            r#"{"cmd":"password=\\","z":"🙂🙂"}"#,
            r#"{"DB_PASSWORD\\":12345678901,"ok":true}"#,
        ] {
            for out in [redact(s, None, &mut f), redact_text(s, None, &mut f)] {
                serde_json::from_str::<serde_json::Value>(&out)
                    .unwrap_or_else(|e| panic!("{e}: {out}"));
            }
        }

        // Brackets are part of a bare value outside JSON.
        for (input, want) in [
            (
                "DB_PASSWORD=[synthetic-value] x",
                "DB_PASSWORD=[REDACTED:assigned_secret] x",
            ),
            (
                "DB_PASSWORD=synthetic[value] x",
                "DB_PASSWORD=[REDACTED:assigned_secret] x",
            ),
            (
                "DB_PASSWORD=[REDACTED:email]synthetic-tail x",
                "DB_PASSWORD=[REDACTED:assigned_secret] x",
            ),
        ] {
            assert_eq!(redact(input, None, &mut f), want);
        }

        // Untouched JSON comes back byte for byte; changed JSON keeps its shape.
        let plain = "{ \"b\": 1,  \"a\": [1, 2] }";
        assert_eq!(redact(plain, None, &mut f), plain);
        let pretty = "{\n  \"z\": 1,\n  \"token\": \"synthetic-value\"\n}";
        let out = redact(pretty, None, &mut f);
        assert!(out.contains('\n') && !out.contains("synthetic"), "{out}");
    }

    #[test]
    fn json_is_redacted_token_by_token_and_keeps_its_bytes() {
        let mut f = Findings::default();
        let mut r = |s: &str| redact(s, Some("/Users/me"), &mut f);
        // Every occurrence of a duplicate key, also in JSON inside a string.
        assert_eq!(
            r(r#"{"password":"synthetic-secret","password":null}"#),
            r#"{"password":"[REDACTED:credential_field]","password":null}"#
        );
        let nested = serde_json::json!({"a": r#"{"password":"synthetic-secret","password":null}"#})
            .to_string();
        assert!(!r(&nested).contains("synthetic"));
        // Classified by the key as written, before the key is redacted.
        let out = r(r#"{"tail0c0ffe-password":"synthetic-secret"}"#);
        assert!(
            !out.contains("synthetic") && !out.contains("tail0c0ffe"),
            "{out}"
        );
        // Numbers: a long one under a credential key is redacted, whatever its notation;
        // every other number keeps its exact token, and key order and spacing stay.
        assert_eq!(
            r(r#"{"password":1000000000000000000000}"#),
            r#"{"password":"[REDACTED:credential_field]"}"#
        );
        assert_eq!(
            r(
                r#"{ "password": "synthetic-secret", "id": 18446744073709551617, "f": 1.0000000000000001 }"#
            ),
            r#"{ "password": "[REDACTED:credential_field]", "id": 18446744073709551617, "f": 1.0000000000000001 }"#
        );
        // An escaped home path is decoded, so it is found.
        assert_eq!(r(r#"{"p":"\/Users\/me\/project"}"#), r#"{"p":"~/project"}"#);
        // A top-level JSON string is decoded too, and stays a valid JSON string.
        let out = r(r#""password='synthetic-secret""#);
        assert_eq!(
            serde_json::from_str::<String>(&out).unwrap(),
            "password='[REDACTED:assigned_secret]"
        );
        let out = r(r#""DB_PASSWORD: |\n  synthetic-secret""#);
        assert!(!out.contains("synthetic"), "{out}");
        // Nesting deeper than serde_json's recursion limit.
        let deep = format!(
            "{}{}{}",
            "[".repeat(1000),
            r#"{"p\u0061ssword":"synthetic-secret"}"#,
            "]".repeat(1000)
        );
        let out = r(&deep);
        assert!(
            !out.contains("synthetic") && out.len() > 2000,
            "{}",
            &out[990..1060]
        );
        // Not JSON: left to the text scanner.
        assert!(json_tokens(r#"{"a":1,}"#).is_none() && json_tokens("[01]").is_none());
        assert!(json_tokens(r#"{"a":"\x"}"#).is_none() && json_tokens("nul").is_none());

        // A lone surrogate elsewhere does not undo the document's redactions.
        let out = r(r#"{"\u0070assword":"synthetic-secret","x":"\ud800","t":"\/Users\/me"}"#);
        assert!(!out.contains("synthetic") && out.contains("~"), "{out}");
        assert!(
            out.contains(r#""x":"\ud800""#),
            "the odd token is kept as written: {out}"
        );
        let out = r(r#"{"token":1e21,"x":"\ud800"}"#);
        assert!(!out.contains("1e21"), "{out}");
        // Arrays under a credential key are secret too; objects are redacted as usual, so a
        // tool schema survives.
        let out = r(r#"{"password":["synthetic-secret",[12345678901]],"n":[1]}"#);
        assert!(
            !out.contains("synthetic")
                && !out.contains("12345678901")
                && out.contains(r#""n":[1]"#),
            "{out}"
        );
        let schema = r#"{"password":{"type":"string","description":"Account password"}}"#;
        assert_eq!(r(schema), schema);

        // YAML doubled apostrophes, as text and inside JSON.
        let mut f = Findings::default();
        assert_eq!(
            redact("DB_PASSWORD: 'abc''synthetic-secret' x", None, &mut f),
            "DB_PASSWORD: '[REDACTED:assigned_secret]' x"
        );
        let out = redact(
            r#"{"content":"DB_PASSWORD: 'abc''synthetic-secret'"}"#,
            None,
            &mut f,
        );
        assert!(!out.contains("synthetic"), "{out}");
        // redact_value classifies by the key as written, and redacts long numbers in any form.
        let mut v = serde_json::json!({"tail0c0ffe-password": "synthetic-secret", "token": 1e21, "n": 12, "secret": ["synthetic-a"], "api_key": {"type": "string"}});
        redact_value(&mut v, None, &mut f);
        let out = v.to_string();
        assert!(
            !out.contains("synthetic") && !out.contains("1e21") && out.contains("12"),
            "{out}"
        );
        assert_eq!(v["api_key"]["type"], "string");
        // Braces and commas are part of a bare value outside JSON, also when the command
        // sits in a JSON string.
        assert_eq!(
            redact("DB_PASSWORD=abc}synthetic-secret-tail x", None, &mut f),
            "DB_PASSWORD=[REDACTED:assigned_secret] x"
        );
        let out = redact(
            r#"{"cmd":"DB_PASSWORD=abc},synthetic-secret-tail"}"#,
            None,
            &mut f,
        );
        assert_eq!(out, r#"{"cmd":"DB_PASSWORD=[REDACTED:assigned_secret]"}"#);
        // Quotes inside one shell word keep the value together; a quote that ends a quoted
        // phrase in prose stays.
        for (input, want) in [
            (
                "DB_PASSWORD=abc\"synthetic-secret-tail\" x",
                "DB_PASSWORD=[REDACTED:assigned_secret] x",
            ),
            (
                "DB_PASSWORD=\"abc\"synthetic-secret-tail x",
                "DB_PASSWORD=[REDACTED:assigned_secret] x",
            ),
            (
                "DB_PASSWORD='abc'\"synthetic-tail\" x",
                "DB_PASSWORD=[REDACTED:assigned_secret] x",
            ),
            (
                "DB_PASSWORD=abc\"def\\\" synthetic-secret-tail\" x",
                "DB_PASSWORD=[REDACTED:assigned_secret] x",
            ),
            (
                "say \"password=abcdefgh\" now",
                "say \"password=[REDACTED:assigned_secret]\" now",
            ),
            (
                "say \"password=abcdefgh\". Then",
                "say \"password=[REDACTED:assigned_secret]\". Then",
            ),
        ] {
            assert_eq!(redact(input, None, &mut f), want);
        }
        // Text that looks like another assignment never hides a secret, wherever it sits.
        for input in [
            "token=x,password='synthetic-secret'",
            "token=x,password=\"synthetic-secret\"",
            r"password=\'a\\\''' token='synthetic-secret'",
            "password=abc\"x token=synthetic-secret\" y",
            "password=token=token=synthetic",
            r#"DB_PASSWORD=abc"synthetic-secret\n" x"#,
            "password=x\"a\\\"synthetic-secret",
            "password=a\"token=x\"\",synthetic-secret\"",
            "password='x token: |\n  a'\n  token=\nsynthetic-secret",
            "DB_PASSWORD=abc\"def\\\" synthetic-secret-tail\" x",
            "password==synthetic-token=x",
            r"password=\nsynthetic-token=x",
            "password: |\n  token='x\nsynthetic-secret'",
            "password='x token: |\n  a'\n  token='x\nsynthetic-secret'",
        ] {
            let out = redact(input, None, &mut f);
            assert!(!out.contains("synthetic"), "{input} -> {out}");
            let json = serde_json::json!({ "cmd": input }).to_string();
            let out = redact(&json, None, &mut f);
            assert!(!out.contains("synthetic"), "{json} -> {out}");
        }
        // A name in a YAML block's header comment is still read.
        let out = redact(
            "x token=\n  |# token=synthetic-secret p\n  body-line\nnext",
            None,
            &mut f,
        );
        assert!(!out.contains("synthetic"), "{out}");
        // A value that starts like a name is still the value.
        for input in [
            "password=synthetic-token=x",
            "password=synthetic_api_key=y z",
        ] {
            let out = redact(input, None, &mut f);
            assert!(!out.contains("synthetic"), "{input} -> {out}");
        }
        assert!(
            !redact(r#"{"cmd":"password=synthetic-token=x"}"#, None, &mut f).contains("synthetic")
        );
        // An assignment inside a quoted value hides neither value.
        for (input, want) in [
            (
                "password='abc token=synthetic-secret' x",
                "password='[REDACTED:assigned_secret]' x",
            ),
            (
                "password='token=x synthetic-secret' x",
                "password='[REDACTED:assigned_secret]' x",
            ),
            (
                "password=\"token=x synthetic-secret\" x",
                "password=\"[REDACTED:assigned_secret]\" x",
            ),
            (
                "password: | # token=x\n  synthetic-secret\nnext: 1",
                "password: | # token=x\n  [REDACTED:assigned_secret]\nnext: 1",
            ),
            (
                "token=token=\\npassword: synthetic-secret",
                "token=token=\\npassword: [REDACTED:assigned_secret]",
            ),
        ] {
            let out = redact(input, None, &mut f);
            assert!(
                !out.contains("synthetic") && (out == want || out.contains("[REDACTED")),
                "{input} -> {out}"
            );
        }
        let out = redact(
            r#"{"cmd":"password='token=x synthetic-secret'"}"#,
            None,
            &mut f,
        );
        assert!(!out.contains("synthetic"), "{out}");
        let out = redact(
            r#"{"cmd":"token=x,password='synthetic-secret'"}"#,
            None,
            &mut f,
        );
        assert!(!out.contains("synthetic"), "{out}");
        // In escaped text (`\"`, `\n`), a quoted piece of a value ends at the escaped line
        // break: it does not swallow the code after it.
        let out = redact(
            r#"let api_key = abcdefgh"read key\")?;\n    let other = keep-this-text;"#,
            None,
            &mut f,
        );
        assert!(out.contains("keep-this-text"), "{out}");
        // A key right after any JSON whitespace-like escape, in text that is not JSON.
        let pat = format!("github_pat_{}", "Ab1".repeat(15));
        for esc in [r"\n", r"\t", r"\r", r"\b", r"\f"] {
            let out = redact(&format!("cut {{\"x\":\"a{esc}{pat} b"), None, &mut f);
            assert!(!out.contains("github_pat_"), "{esc}: {out}");
        }
        // Credential names after those escapes too.
        for esc in [r"\b", r"\f"] {
            let out = redact(
                &format!("cut {{\"x\":\"a{esc}password=synthetic-value b"),
                None,
                &mut f,
            );
            assert!(!out.contains("synthetic"), "{esc}: {out}");
        }
        // A long header comment, and doubled apostrophes after an escaped opening quote.
        let long = format!(
            "password: | #{}\n  synthetic-secret\nnext: 1",
            "x".repeat(300)
        );
        assert!(!redact(&long, None, &mut f).contains("synthetic"));
        assert_eq!(
            redact(
                r"password=\'synthetic-head''synthetic-tail\' x",
                None,
                &mut f
            ),
            r"password=\'[REDACTED:assigned_secret]\' x"
        );
        // After an escaped opening quote, `\''` closes it: the next assignment is its own.
        assert_eq!(
            redact(
                r"password=\'synthetic-first\'' token=\'synthetic-tail\' x",
                None,
                &mut f
            ),
            r"password=\'[REDACTED:assigned_secret]\'' token=\'[REDACTED:assigned_secret]\' x"
        );
        let out = redact(
            "password=\\'synthetic-head\nsynthetic-tail\\'' x",
            None,
            &mut f,
        );
        assert!(!out.contains("synthetic"), "{out}");
        // A backslash before a doubled apostrophe is a plain character in YAML.
        for s in [
            r"password: 'abc''synthetic-secret' x",
            r"password: 'synthetic-head''synthetic-tail' x",
        ] {
            assert_eq!(
                redact(s, None, &mut f),
                "password: '[REDACTED:assigned_secret]' x"
            );
        }
    }

    #[test]
    fn adversarial_text_is_redacted_whole_past_the_budget() {
        // Thousands of chained or nested candidates: past the scan budget the rest of the text
        // is redacted whole, so time stays linear and nothing leaks.
        let mut f = Findings::default();
        let chain = format!("{}synthetic-secret", "token=x,token\"=x,".repeat(20_000));
        let mut nested: String = (0..1500).map(|i| " ".repeat(i) + "token: |\n").collect();
        nested += &(" ".repeat(1500) + "synthetic-secret\n");
        for text in [chain, nested] {
            let start = std::time::Instant::now();
            let out = redact(&text, None, &mut f);
            assert!(
                !out.contains("synthetic"),
                "{}",
                &out[out.len().saturating_sub(80)..]
            );
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
        }
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
