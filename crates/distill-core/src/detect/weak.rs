//! What sits under a credential name without being a credential: identifier names that only end
//! like one (`element_token`, a tokenizer's `eos_token`), and values too short, too id-like or
//! too obviously made up to be a secret. Shared by the assignment rule and the JSON credential
//! field, so the same value under the same name is found by neither or both (issue #98). Since
//! #120 it also holds the value shapes that are code, references to a secret, Distill's own text
//! or stand-ins (`refused_shape`). A value refused here is read as any other string, so a vendor
//! key inside it is still found.

use std::sync::OnceLock;

use regex::Regex;

use super::Kind;

/// Shorter values are rarely secrets (`csrf_token=short`, `max_token=4096`, a `pass` counter).
pub(super) const MIN_SECRET_CHARS: usize = 8;

/// Under a token name, an id-shaped value shorter than this is an id, not a token.
const MIN_ID_SHAPED_TOKEN_CHARS: usize = 16;

/// Placeholders are short; longer values are not checked for placeholder words, which keeps the
/// check cheap however many candidates share one long value.
const MAX_STAND_IN_BYTES: usize = 256;

/// Names that end like a credential's but name an identifier: T3 Code's element ids in page
/// snapshots, and a tokenizer's special tokens. Compared whole, ignoring case and `-` for `_`.
const IDENTIFIER_NAMES: &[&str] = &[
    "element_token",
    "bos_token",
    "eos_token",
    "pad_token",
    "unk_token",
    "sep_token",
    "cls_token",
    "mask_token",
    // Pagination cursors (#120).
    "next_page_token",
    "nextpagetoken",
    "page_token",
    "pagetoken",
    "prev_page_token",
    "previous_page_token",
    "next_token",
    "continuation_token",
    "pagination_token",
];

/// Words that mark a value as a stand-in for a secret (`your-api-key`, `changeme`, `xxxx…`),
/// matched anywhere in the value, ignoring case. `example` is matched too, in [`is_stand_in`].
const STAND_IN_WORDS: &[&str] = &[
    "xxxx",
    "****",
    "your-",
    "your_",
    "<your",
    "changeme",
    "change-me",
    "change_me",
    "placeholder",
    "redacted",
];

/// Whether `name` (an assignment's name or a JSON key) names an identifier, not a credential.
pub(super) fn is_identifier_name(name: &str) -> bool {
    let name = name.replace('-', "_");
    IDENTIFIER_NAMES
        .iter()
        .any(|n| n.eq_ignore_ascii_case(&name))
}

/// Whether `value`, found under the credential name `name`, can be a secret: long enough, not a
/// short id under a token name, and not a stand-in.
pub(super) fn can_be_secret(name: &str, value: &str) -> bool {
    gate(name, value, false)
}

/// [`can_be_secret`] for the bare value after a quoted JSON key in text that does not parse
/// (cut off): a number there is gated as the JSON path gates it.
pub(super) fn can_be_bare_secret(name: &str, value: &str, json_key: bool) -> bool {
    let number = json_key
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'));
    gate(name, value, number)
}

/// [`can_be_secret`] for a value under a JSON credential key, which also refuses prose and a
/// shell command (C1 and C4, issue #120).
pub(super) fn can_be_field_secret(name: &str, value: &str) -> bool {
    can_be_secret(name, value) && !is_prose(value) && !is_bang_command(value)
}

/// C4: an editor's `!command args`, which reads the value from elsewhere: `!`, a lowercase
/// command word, then a space. Not any leading `!`, which human passwords have.
fn is_bang_command(value: &str) -> bool {
    value.strip_prefix('!').is_some_and(|rest| {
        let word = rest.split(' ').next().unwrap_or("");
        rest.len() > word.len() && !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase())
    })
}

fn gate(name: &str, value: &str, number: bool) -> bool {
    // Counted only as far as needed: a value can be the rest of a long text.
    if value.chars().nth(MIN_SECRET_CHARS - 1).is_none() {
        return false;
    }
    if is_token_name(name) && is_short_id(value) {
        return false;
    }
    !is_stand_in(value) && !refused_shape(value, number)
}

/// [`can_be_secret`] for a JSON number, on one form whatever its notation, so the text path
/// (`5.00000000`, `1e21`) and the `Value` path (`5.0`, `1e+21`) agree: an integral number as all
/// its digits (`5`, `1000000000000000000000`), any other as serde writes it.
pub(super) fn can_be_secret_number(name: &str, n: &serde_json::Number) -> bool {
    let text = match n.as_f64() {
        Some(f) if n.is_f64() && f.is_finite() && f.fract() == 0.0 => format!("{f:.0}"),
        _ => n.to_string(),
    };
    // A number is a deliberate typed value, so R15 (digits only) does not apply to it.
    gate(name, &text, true)
}

/// `token`, or a name ending in `_token` or `-token`.
fn is_token_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "token" || name.ends_with("_token") || name.ends_with("-token")
}

/// Shorter than [`MIN_ID_SHAPED_TOKEN_CHARS`], and only lowercase letters, digits and `:`.
fn is_short_id(value: &str) -> bool {
    value.len() < MIN_ID_SHAPED_TOKEN_CHARS
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b':')
}

fn is_stand_in(value: &str) -> bool {
    if value.len() > MAX_STAND_IN_BYTES {
        return false;
    }
    let value = value.to_ascii_lowercase();
    // `example`, but not the reserved domains (`example.com`), which can sit in a real value.
    let example = value
        .match_indices("example")
        .any(|(i, w)| value.as_bytes().get(i + w.len()) != Some(&b'.'));
    example || STAND_IN_WORDS.iter().any(|w| value.contains(w))
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid weak-value regex"))
}

/// A value's quotes, and the backslashes that escape them, at its start.
fn unquoted_start(v: &str) -> &str {
    v.trim_start_matches(['"', '\'', '\\', '`'])
}

/// Both ends, for the rules that test a whole value.
fn unquoted(v: &str) -> &str {
    unquoted_start(v).trim_end_matches(['"', '\'', '\\', '`'])
}

/// Where the `{` at the start of `v` closes, if it does, counting nested braces.
fn close_of_brace(v: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in v.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Value shapes that are not secrets whatever the name (issue #120, R1 to R16 but R9). Each is
/// kept narrow: it tests the shape the audit showed, since real secrets sit next to these shapes
/// (base64 has `/` and `=`, passwords hold parentheses, test-mode vendor keys are keys). `number`
/// is set for a JSON number, which the digit rules (R15 and R13's run of a digit) leave alone.
fn refused_shape(value: &str, number: bool) -> bool {
    let value = match code_span(value) {
        Some(inner) if inner.chars().nth(MIN_SECRET_CHARS - 1).is_none() => return true,
        Some(inner) => inner,
        None => value,
    };
    let t = unquoted(value);
    let s = unquoted_start(value);
    let lower = s.to_ascii_lowercase();
    is_env_reference(s)
        || s.starts_with("$(")
        || is_env_read(&lower)
        || is_distill_text(t)
        || is_masking_run(t)
        || is_next_key(t)
        || is_regex_text(t)
        || is_code_shape(t)
        || is_placeholder_wrapper(t)
        || is_template(t)
        || is_word_stand_in(t)
        || is_test_marker(t, number)
        || is_path(t)
        || (!number && is_number(t))
        || is_fixture_key(t)
}

/// The inside of a value written as a markdown code span, as prose writes a password
/// (``(password: `abc`)``): the punctuation after the span closes the prose, so the rules judge
/// the span alone. Only on one line, and never for a shell's `` `cmd args` ``, which has a space
/// and so ends the bare value before its closing backtick.
fn code_span(value: &str) -> Option<&str> {
    let rest = value.strip_prefix('`')?;
    rest.find(['`', '\n'])
        .filter(|&i| rest[i..].starts_with('`'))
        .map(|i| &rest[..i])
}

/// R1: `$NAME`, `${NAME}` or `${NAME:-default}`, the whole value. An unbraced name is all
/// upper or all lower case, so `$Xy7…` (a password that starts with `$`) is not one.
fn is_env_reference(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('$') else {
        return false;
    };
    let rest = rest.trim_end_matches(['"', '\'', '\\', '`']);
    if let Some(inner) = rest.strip_prefix('{') {
        let name_len = inner
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
            .count();
        let starts_ok = inner
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_');
        return starts_ok
            && close_of_brace(rest).is_some_and(|i| i == rest.len() - 1)
            && matches!(
                inner.as_bytes().get(name_len),
                Some(b'}' | b':' | b'-' | b'=' | b'?' | b'+')
            );
    }
    static NAME: OnceLock<Regex> = OnceLock::new();
    re(&NAME, r"^(?:[A-Z_][A-Z0-9_]*|[a-z_][a-z0-9_]*)$").is_match(rest)
}

/// R3: code that reads an environment variable. `lower` is the value in lower case.
fn is_env_read(lower: &str) -> bool {
    [
        "process.env.",
        "process.env[",
        "os.environ",
        "os.getenv(",
        "std::env::var(",
        "env::var(",
        "getenv(",
        "deno.env",
        "env[",
        "dotenv",
    ]
    .iter()
    .any(|p| lower.starts_with(p))
}

/// R4: Distill's own text: a placeholder, a synthetic test value, a finding kind's name.
fn is_distill_text(t: &str) -> bool {
    t.starts_with("[secret:")
        || t.to_ascii_lowercase().contains("synthetic")
        || Kind::ALL.iter().any(|k| k.name() == t)
}

/// R5: one masking character repeated.
fn is_masking_run(t: &str) -> bool {
    let mut chars = t.chars();
    chars
        .next()
        .is_some_and(|c| matches!(c, '•' | '*' | 'x' | 'X' | '#') && chars.all(|d| d == c))
}

/// R6: the next key, read as the value (`API_KEY=`, `NAME:`). Not a bare trailing `=`, which is
/// base64 padding: with `=`, the name needs an underscore or no digit.
fn is_next_key(t: &str) -> bool {
    let Some(name) = t.strip_suffix(['=', ':']) else {
        return false;
    };
    if name.is_empty()
        || !name
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
    {
        return false;
    }
    t.ends_with(':') || name.contains('_') || name.bytes().all(|c| c.is_ascii_uppercase())
}

/// R7: regex text: `.*` with an alternation, or an opening character class.
fn is_regex_text(t: &str) -> bool {
    static CLASS: OnceLock<Regex> = OnceLock::new();
    (t.contains(".*") && t.contains('|'))
        || re(&CLASS, r"^\[(?:\^|\\[wdsWDS]|[A-Za-z0-9]-[A-Za-z0-9])").is_match(t)
}

/// R8: code. Only the ending is tested for punctuation: a real password can hold parentheses
/// in the middle.
fn is_code_shape(t: &str) -> bool {
    static DOTTED: OnceLock<Regex> = OnceLock::new();
    // Not `!` or `?`: human passwords end with them.
    t.ends_with([';', ',', ')', '(', ']', ':'])
        || t.starts_with('&')
        || t.contains("::")
        || t.contains("=>")
        || re(&DOTTED, r"^[A-Za-z_][A-Za-z_]*(?:\.[A-Za-z_][A-Za-z_]*)+$").is_match(t)
}

/// R10: `<…>`, `<<<…>>>`, or a vendor prefix then `...`.
fn is_placeholder_wrapper(t: &str) -> bool {
    static ELLIPSIS: OnceLock<Regex> = OnceLock::new();
    (t.len() >= 3 && t.starts_with('<') && t.ends_with('>'))
        || re(
            &ELLIPSIS,
            r"^(?:(?:[A-Za-z0-9]{1,12}[-_]){1,2}|AKIA|AIza|eyJ)\.\.\.",
        )
        .is_match(t)
}

/// R11: one `{…}` or `${…}` expression.
fn is_template(t: &str) -> bool {
    let body = t.strip_prefix('$').unwrap_or(t);
    body.starts_with('{') && close_of_brace(body).is_some_and(|i| i == body.len() - 1)
}

/// R12: lowercase words joined by `-` or `_` with a credential word as one of them
/// (`prod-password`), or the credential word alone.
fn is_word_stand_in(t: &str) -> bool {
    const HEADS: &[&str] = &[
        "password",
        "passwd",
        "secret",
        "token",
        "apikey",
        "credential",
        "credentials",
    ];
    if !t
        .bytes()
        .all(|c| c.is_ascii_lowercase() || c == b'-' || c == b'_')
    {
        return false;
    }
    let mut parts = t.split(['-', '_']);
    parts.clone().all(|p| !p.is_empty()) && parts.any(|p| HEADS.contains(&p))
}

/// R13: a `test`, `fixture`, `fake` or `smoke` part, or a run of one repeated digit. A test-mode
/// vendor key is refused here too, but is then found by its own pattern.
fn is_test_marker(t: &str, number: bool) -> bool {
    let marked = t
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(|p| p.trim_end_matches(|c: char| c.is_ascii_digit()))
        .any(|p| {
            ["test", "fixture", "fake", "smoke"]
                .iter()
                .any(|m| p.eq_ignore_ascii_case(m))
        });
    let (mut run, mut prev) = (0usize, 0u8);
    let mut repeated = false;
    for c in t.bytes() {
        run = if c == prev { run + 1 } else { 1 };
        prev = c;
        repeated |= c.is_ascii_digit() && run >= 6;
    }
    marked || (repeated && !number)
}

/// R14: a path: from a home root or `./`, or `/`-separated parts ending in a file extension.
/// A `/` alone is not enough, since base64 has them.
fn is_path(t: &str) -> bool {
    static FILE: OnceLock<Regex> = OnceLock::new();
    ["~/", "/Users/", "/home/", "/root/", "./", "../"]
        .iter()
        .any(|p| t.starts_with(p))
        || re(
            &FILE,
            r"^/?[A-Za-z0-9_@.-]+(?:/[A-Za-z0-9_@.-]+)+\.[A-Za-z][A-Za-z0-9]{0,4}$",
        )
        .is_match(t)
}

/// R15: all digits, or a decimal number.
fn is_number(t: &str) -> bool {
    static N: OnceLock<Regex> = OnceLock::new();
    re(&N, r"^[0-9]+(?:\.[0-9]+)?$").is_match(t)
}

/// R16: a generated fixture key, `<prefix>-` and short uppercase groups (`abc-AB12-CD34-EF56`).
fn is_fixture_key(t: &str) -> bool {
    static K: OnceLock<Regex> = OnceLock::new();
    re(&K, r"^[A-Za-z]{2,8}-(?:[A-Z0-9]{2,6}-)+[A-Z0-9]{2,6}$").is_match(t)
}

/// C1: prose, three or more words each short. A chunk of 64 or more characters (a PEM's lines)
/// keeps a multiline key.
fn is_prose(value: &str) -> bool {
    if value.contains("-----BEGIN") {
        return false;
    }
    let mut words = 0;
    for chunk in value.split_whitespace() {
        if chunk.chars().nth(63).is_some() {
            return false;
        }
        words += 1;
    }
    words >= 3
}

/// C2: a key such as `db_pass`, ending `_pass` or `-pass` after a word character: a counter or
/// a status, not a credential name. A bare `pass`, or `_pass`, still is.
pub(super) fn is_pass_suffixed(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.strip_suffix("_pass")
        .or_else(|| k.strip_suffix("-pass"))
        .and_then(|head| head.chars().last())
        .is_some_and(|c| c.is_ascii_alphanumeric())
}
