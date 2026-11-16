//! Text that is JSON (tool arguments, a tool's JSON output) is read token by token, so no
//! pattern has to read escaped JSON. Each string is decoded and detected on its own (a string
//! that is JSON itself recurses), and what is found is mapped back to bytes of the text. A value
//! under a credential-named key is one finding whole, a string or any number. Ported from the previous build's redactor.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

use super::{Detector, Hit, Kind};

/// What is found in `text` read as JSON, or `None` unless the whole text is a JSON object, array
/// or string.
pub(super) fn detect_json(det: &Detector, text: &str, depth: u32) -> Option<Vec<Hit>> {
    if !text.trim_start().starts_with(['{', '[', '"']) {
        return None;
    }
    let mut hits = Vec::new();
    for tok in json_tokens(text)? {
        let raw = &text[tok.start..tok.end];
        match tok.role {
            Role::Secret if raw.starts_with('"') => hits.push(Hit {
                kind: Kind::CredentialField,
                range: tok.start + 1..tok.end - 1,
                value: decode(raw).0,
            }),
            Role::Secret => hits.push(Hit {
                kind: Kind::CredentialField,
                range: tok.start..tok.end,
                value: raw.to_string(),
            }),
            Role::Str => {
                let (s, map) = decode(raw);
                for h in det.detect_at(&s, depth + 1) {
                    let range = map.encoded(&h.range);
                    hits.push(Hit {
                        kind: h.kind,
                        range: tok.start + range.start..tok.start + range.end,
                        value: h.value,
                    });
                }
            }
        }
    }
    Some(hits)
}

/// Whether a key names a credential, so that its value is a secret whole.
pub(super) fn is_credential_key(key: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^(?:[A-Za-z0-9]*[_-]+)*(?:password|passwd|pass|secret|client[_-]?secret|token|access[_-]?token|refresh[_-]?token|auth[_-]?token|api[_-]?key|apikey|private[_-]?key|authorization|credentials?)$")
            .expect("valid key regex")
    })
    .is_match(key)
}

/// Where each byte of a decoded string came from in its token.
enum Map {
    /// No escapes: decoded byte `i` is token byte `i + 1`, after the opening quote.
    Plain,
    /// For each decoded byte, the token bytes it was decoded from.
    Escaped(Vec<(usize, usize)>),
}

impl Map {
    /// The token bytes a non-empty range of decoded bytes came from.
    fn encoded(&self, r: &Range<usize>) -> Range<usize> {
        match self {
            Map::Plain => r.start + 1..r.end + 1,
            Map::Escaped(m) => m[r.start].0..m[r.end - 1].1,
        }
    }
}

/// Decode a JSON string token (quotes included, already checked by `string_end`), with where
/// each byte came from. A lone surrogate (`\ud800`), which no Rust string can hold, becomes
/// U+FFFD instead of failing, so one odd escape does not send a whole document to the text
/// scanner.
fn decode(raw: &str) -> (String, Map) {
    let inner = &raw[1..raw.len() - 1];
    if !inner.contains('\\') {
        return (inner.to_string(), Map::Plain);
    }
    let b = inner.as_bytes();
    let hex = |i: usize| {
        inner
            .get(i..i + 4)
            .and_then(|h| u32::from_str_radix(h, 16).ok())
    };
    let mut out = String::with_capacity(inner.len());
    let mut map = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            let len = inner[i..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&inner[i..i + len]);
            map.extend((i..i + len).map(|k| (k + 1, k + 2)));
            i += len;
            continue;
        }
        let (c, len) = match b[i + 1] {
            b'b' => ('\u{8}', 2),
            b'f' => ('\u{c}', 2),
            b'n' => ('\n', 2),
            b'r' => ('\r', 2),
            b't' => ('\t', 2),
            b'u' => {
                let hi = hex(i + 2).unwrap_or(0xFFFD);
                let pair = (0xD800..0xDC00).contains(&hi)
                    && b.get(i + 6..i + 8) == Some(b"\\u")
                    && hex(i + 8).is_some_and(|lo| (0xDC00..0xE000).contains(&lo));
                if pair {
                    let lo = hex(i + 8).expect("checked");
                    let c = char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00));
                    (c.unwrap_or('\u{FFFD}'), 12)
                } else {
                    (char::from_u32(hi).unwrap_or('\u{FFFD}'), 6)
                }
            }
            other => (other as char, 2),
        };
        out.push(c);
        map.extend(std::iter::repeat_n((i + 1, i + 1 + len), c.len_utf8()));
        i += len;
    }
    (out, Map::Escaped(map))
}

/// A JSON token to look into.
struct Tok {
    start: usize,
    end: usize,
    role: Role,
}

enum Role {
    /// A non-empty string, or any number, under a credential-named key.
    Secret,
    /// Any other string, object keys included.
    Str,
}

/// The strings (and numbers under credential-named keys) of a JSON document, with their byte ranges; `None` unless
/// the whole text is valid JSON. Iterative, so nesting depth costs no stack.
fn json_tokens(text: &str) -> Option<Vec<Tok>> {
    // `secret`: the value being read sits under a credential-named key. Arrays pass that on
    // (`"password": ["…"]`); objects do not, since a tool schema's `"password": {"type": …}`
    // describes a field rather than holding one.
    enum Frame {
        Obj { secret: bool },
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
    let (mut toks, mut stack) = (Vec::new(), Vec::new());
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
                    Some(Frame::Obj { secret: true } | Frame::Arr { secret: true })
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
                        stack.push(Frame::Obj { secret: false });
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
                        if secret {
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
                let Some(Frame::Obj { secret }) = stack.last_mut() else {
                    return None;
                };
                *secret = is_credential_key(&decode(&text[i..end]).0);
                toks.push(Tok {
                    start: i,
                    end,
                    role: Role::Str,
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
