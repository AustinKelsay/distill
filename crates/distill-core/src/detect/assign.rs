//! Secret assignments: a credential-named key and a separator, then its value
//! (`DB_PASSWORD=…`, `"api_key": "…"`, `token: |` and a YAML block). Ported from the previous
//! build's redactor (commit `a6e4be0`), which rewrote the value; here only its range is found.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

/// The name and separator of a secret assignment: `TINFOIL_API_KEY=`, `"db-password": "`,
/// `_SECRET: `, or `\"GH_TOKEN\":\n  \"` in JSON that sits inside a string. The name may carry
/// any prefix, since a bare `\b` would stop at the `_`, and may follow an escaped newline
/// (`\npassword=`), where `\b` finds no boundary after the `n`. `scan_value` finds where the value ends.
const ASSIGNMENT_HEAD: &str = r#"(?i)(?:\b|\\+[nrtbf])(?P<name>(?:[A-Za-z0-9]*[_-]+)*(?:password|passwd|secret|client[_-]?secret|token|api[_-]?key|access[_-]?token|auth[_-]?token|private[_-]?key))\b(?P<nq>\\*["'])?(?:\s|\\+[nrtf])*[:=](?:\s|\\+[nrtf])*(?P<q>\\*["'])?"#;

fn assignment_head() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(ASSIGNMENT_HEAD).expect("valid assignment regex"))
}

/// The value of every secret assignment in `text`, as byte ranges, sorted and with overlapping
/// ones joined. Every assignment is read, also one inside another's value, and their values are
/// joined: an assignment inside a quoted value (`password='token=x abc'`) can neither hide the
/// outer value nor be hidden by it.
pub(super) fn assignments(text: &str) -> Vec<Range<usize>> {
    // In valid JSON, every unescaped `"` delimits a string. In other text it may be a
    // character of the value (`DB_PASSWORD='abc"def'`).
    let t = text.trim_start();
    let json_text = (t.starts_with('{') || t.starts_with('['))
        && serde_json::from_str::<serde::de::IgnoredAny>(text).is_ok();
    let mut memo = Memo::default();
    let placeholders = super::placeholders(text);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    // The start of the current line, found incrementally so a long line is read once.
    let (mut line_start, mut seen) = (0, 0);
    // The current line's leading run of indent, measured once per line.
    let mut indent_of: Option<(usize, usize)> = None;
    // Scanning normally reads each byte a few times. Only adversarial text (thousands of
    // nested or chained candidates) costs more; past this budget, everything from the next
    // assignment on is one value, which can find too much but never too little.
    let budget = 64 * text.len() + 4096;
    let mut cur = assignment_head().captures_at(text, 0);
    while let Some(c) = cur {
        let whole = c.get(0).expect("whole match");
        let (head_start, head_end) = (whole.start(), whole.end());
        // The next name may start inside this one's separator (`token=\npassword:`), so it is
        // looked for from the end of this name.
        let name = c.name("name").expect("name group");
        let name_end = name.end();
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
            ranges.push((head_end, text.len()));
            break;
        }
        if !super::weak::is_identifier_name(name.as_str())
            && let Some((s, e)) = scan_value(text, head_end, quote, &ctx, &mut memo)
            && s <= e
            && e <= text.len()
            && text.is_char_boundary(s)
            && text.is_char_boundary(e)
            // The gate reads the value again, also when it came from the memo (a YAML block or
            // an unclosed quote shared by many candidates), so that read counts too.
            && { memo.work += e - s; true }
            // Only an unquoted value after a quoted JSON key can be a JSON number.
            && super::weak::can_be_bare_secret(
                name.as_str(),
                &text[s..e],
                ctx.json_key && quote.is_none(),
            )
            && !placeholders.iter().any(|p| p.start <= s && e <= p.end)
        {
            ranges.push((s, e));
        }
        cur = next_c;
    }
    ranges.sort_by_key(|r| r.0);
    let mut out: Vec<Range<usize>> = Vec::new();
    for (s, e) in ranges {
        match out.last_mut() {
            Some(last) if s < last.end => last.end = last.end.max(e),
            _ if s < e => out.push(s..e),
            _ => {}
        }
    }
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
    /// Bytes scanned so far, against the budget in `assignments`.
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

/// Where an assignment sits in the text being scanned.
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
/// every assignment is also read on its own, and the union of their values is found.
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
    if (ctx.json_key && matches!(b.get(at), Some(b'[' | b'{'))) || opens_json(&b[at..]) {
        return None;
    }
    Some((at, char_end(text, memo.word_end(b, at, ctx))))
}

/// Whether a bare value opens a JSON object or array (`{"a":`, `[{`, `{\"a\"`, `[]`): a
/// fragment the scan would run on into, not a secret. `[synthetic-value]` is a value.
fn opens_json(b: &[u8]) -> bool {
    let next = || {
        b[1..]
            .iter()
            .copied()
            .find(|c| !matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'\\'))
    };
    match b.first() {
        Some(b'{') => matches!(next(), Some(b'"' | b'}')),
        Some(b'[') => matches!(next(), Some(b'"' | b'{' | b'[' | b']')),
        _ => false,
    }
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
            // A token in a URL query ends at the next parameter (`token=abc&page=2`). Only then:
            // an `&` inside a password (`pw&x9`) is part of it.
            b'&' if next_query_param(&b[i + 1..]) => break,
            // JSON punctuation ends a value only after a quoted JSON key; in a shell or config
            // line it is part of the value (`DB_PASSWORD=abc}def`).
            b',' | b'}' | b']' if ctx.json_key => break,
            b'\\' => match b.get(i + 1) {
                None | Some(b'"' | b'\'' | b'n' | b'r' | b't' | b'f') => break,
                Some(_) => i += 2,
            },
            _ => i += 1,
        }
    }
    i.min(b.len())
}

/// Whether `b`, after an `&`, starts another query parameter: a name and `=`.
fn next_query_param(b: &[u8]) -> bool {
    let name = b
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
        .count();
    name > 0 && b.get(name) == Some(&b'=')
}
