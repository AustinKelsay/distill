//! Tags as the owner sees them (issue #156): a card keeps the tags its model wrote, and what is
//! shown and filtered is a view over them, the tag in its standard form and then through the
//! owner's aliases (`[tags.aliases]` in `config.toml`). Nothing here costs model time or
//! rewrites a card.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// `[tags]` in `config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TagsConfig {
    /// A tag (in any form) and the tag to show and filter it as.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: BTreeMap<String, String>,
}

impl TagsConfig {
    pub fn is_empty(&self) -> bool {
        self.aliases.is_empty()
    }
}

/// A tag's standard form: lowercase and trimmed, with spaces and `_` turned into `-` and runs
/// of `-` made one, so "Rust Lang", "rust_lang" and "rust--lang" are all "rust-lang".
pub fn standard(tag: &str) -> String {
    let mut out = String::new();
    for c in tag.trim().to_lowercase().chars() {
        let c = if c.is_whitespace() || c == '_' {
            '-'
        } else {
            c
        };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    out
}

/// The tag `tag` is shown and filtered as: its standard form, through the owner's aliases, to
/// the end of a chain of them (`js` -> `javascript` -> `ecmascript`), stopping where one loops.
pub fn view(tag: &str, config: &TagsConfig) -> String {
    view_through(tag, config).0
}

/// [`view`], with each alias destination it went through as the owner wrote it.
pub fn view_through<'a>(tag: &str, config: &'a TagsConfig) -> (String, Vec<&'a str>) {
    let mut tag = standard(tag);
    let mut seen = vec![tag.clone()];
    let mut through = Vec::new();
    while let Some((_, to)) = config
        .aliases
        .iter()
        .find(|(from, _)| standard(from) == tag)
    {
        through.push(to.as_str());
        tag = standard(to);
        if seen.contains(&tag) {
            break;
        }
        seen.push(tag.clone());
    }
    (tag, through)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_spellings_share_one_standard_form_and_aliases_apply_after_it() {
        for t in [
            "Rust Lang",
            "rust_lang",
            " rust-lang ",
            "RUST--lang",
            "rust - lang",
        ] {
            assert_eq!(standard(t), "rust-lang", "{t:?}");
        }
        assert_eq!(standard("c++"), "c++");
        let config = TagsConfig {
            aliases: [
                ("Rust_Lang".into(), "Rust".into()),
                ("js".into(), "javascript".into()),
            ]
            .into(),
        };
        assert_eq!(view("rust lang", &config), "rust");
        assert_eq!(view("JS", &config), "javascript");
        assert_eq!(view("python", &config), "python");

        let chain = TagsConfig {
            aliases: [
                ("js".into(), "javascript".into()),
                ("javascript".into(), "ecmascript".into()),
                ("a".into(), "b".into()),
                ("b".into(), "a".into()),
            ]
            .into(),
        };
        assert_eq!(view("js", &chain), "ecmascript");
        assert_eq!(view("javascript", &chain), "ecmascript");
        assert_eq!(view("a", &chain), "a", "a loop stops where it comes back");
        assert_eq!(view("b", &chain), "b");
    }
}
