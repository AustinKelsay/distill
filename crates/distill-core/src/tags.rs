//! Tags as the owner sees them (issues #156, #158): a card keeps the tags its model wrote, and
//! what is shown and filtered is a view over them, the tag in its standard form and then through
//! the owner's aliases (`[tags.aliases]` in `config.toml`), and last the tags the owner added to
//! or hid on the session (`tags.json`, [`OwnerTags`]). Nothing here costs model time or rewrites
//! a card.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::detect::Detector;
use crate::home::Home;

/// `[tags]` in `config.toml`, and the owner's edits from `tags.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TagsConfig {
    /// A tag (in any form) and the tag to show and filter it as.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: BTreeMap<String, String>,
    /// The tags the owner added to or hid on sessions: read from `tags.json` by
    /// [`Home::load_config`], never written to `config.toml`.
    #[serde(skip)]
    pub owner: OwnerTags,
}

/// The owner's tag edits, by session key (`tags.json`). Each tag is in its standard form; a tag
/// is never both added and hidden on one session. Written only through [`tag_session`], which the
/// GUI and the command line call and the MCP server never does.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OwnerTags {
    #[serde(default)]
    pub sessions: BTreeMap<String, Edit>,
}

/// One session's edits.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Edit {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
}

impl OwnerTags {
    /// The edits in `tags.json`; none when there is no such file.
    pub fn load(home: &Home) -> Result<Self> {
        let path = home.owner_tags_path();
        match std::fs::read(&path) {
            Ok(b) => {
                serde_json::from_slice(&b).with_context(|| format!("parse {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }

    /// The edits on the session `key`.
    pub fn of(&self, key: &str) -> Option<&Edit> {
        self.sessions.get(key)
    }
}

/// Longest tag the owner can add, in characters, as for a card's.
const TAG_MAX: usize = 40;

/// Add the tags `add` to the session `session` (its key, or its id when only one harness has it)
/// and hide the tags `hide` on it. Adding a hidden tag shows it again and hiding an added one
/// takes it off, so each undoes the other. A tag the detector finds a secret or personal detail
/// in is refused: `tags.json` holds no session text and no finding. Returns the session's key
/// and its edits now.
pub fn tag_session(
    home: &Home,
    config: &Config,
    session: &str,
    add: &[String],
    hide: &[String],
) -> Result<(String, Edit)> {
    let Some(facts) = crate::read::find(home, session)? else {
        bail!("no session {session} in the archive");
    };
    let key = facts.key();
    let detector = Detector::new(&crate::findings::owner(config));
    let clean = |t: &String| -> Result<String> {
        let s = standard(t);
        if s.is_empty() || s.chars().count() > TAG_MAX {
            bail!("a tag must be 1 to {TAG_MAX} characters");
        }
        if !detector.detect(t).is_empty() || !detector.detect(&s).is_empty() {
            bail!("a tag can't hold a secret or a personal detail");
        }
        Ok(s)
    };
    let add = add.iter().map(clean).collect::<Result<Vec<_>>>()?;
    let hide = hide.iter().map(clean).collect::<Result<Vec<_>>>()?;
    let _lock = home.lock("tags")?;
    let mut all = OwnerTags::load(home)?;
    let edit = all.sessions.entry(key.clone()).or_default();
    // Tags are told apart as the view shows them, so `js` and an alias of it are one tag.
    let same = |a: &str, b: &str| view(a, &config.tags) == view(b, &config.tags);
    for t in add {
        let hidden = edit.hide.len();
        edit.hide.retain(|h| !same(h, &t));
        if edit.hide.len() == hidden && !edit.add.iter().any(|a| same(a, &t)) {
            edit.add.push(t);
        }
    }
    for t in hide {
        let added = edit.add.len();
        edit.add.retain(|a| !same(a, &t));
        if edit.add.len() == added && !edit.hide.iter().any(|h| same(h, &t)) {
            edit.hide.push(t);
        }
    }
    let now = edit.clone();
    if now == Edit::default() {
        all.sessions.remove(&key);
    }
    crate::util::write_atomic(&home.owner_tags_path(), &serde_json::to_vec_pretty(&all)?)?;
    Ok((key, now))
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

/// Whether the session `key`'s edits hide `tag`, compared through the view.
pub(crate) fn hidden(tag: &str, key: &str, config: &TagsConfig) -> bool {
    let tag = view(tag, config);
    config
        .owner
        .of(key)
        .is_some_and(|e| e.hide.iter().any(|h| view(h, config) == tag))
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
            ..Default::default()
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
            ..Default::default()
        };
        assert_eq!(view("js", &chain), "ecmascript");
        assert_eq!(view("javascript", &chain), "ecmascript");
        assert_eq!(view("a", &chain), "a", "a loop stops where it comes back");
        assert_eq!(view("b", &chain), "b");
    }
}
