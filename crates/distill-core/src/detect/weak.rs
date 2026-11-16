//! What sits under a credential name without being a credential: identifier names that only end
//! like one (`element_token`, a tokenizer's `eos_token`), and values too short, too id-like or
//! too obviously made up to be a secret. Shared by the assignment rule and the JSON credential
//! field, so the same value under the same name is found by neither or both (issue #98).

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
    // Counted only as far as needed: a value can be the rest of a long text.
    if value.chars().nth(MIN_SECRET_CHARS - 1).is_none() {
        return false;
    }
    if is_token_name(name) && is_short_id(value) {
        return false;
    }
    !is_stand_in(value)
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
