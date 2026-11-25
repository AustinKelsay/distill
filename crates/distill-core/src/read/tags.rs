//! The tags in use (issue #157): each tag as the owner sees it ([`crate::tags::view`]) with how
//! many sessions carry it, over the sessions a [`Filter`] matches. Read from the stored cards, as
//! the tag filter is, never from events. Tags are written from session text, so this is a read:
//! the reader's trust is checked and the read log gets a line with the counts.

use std::cell::OnceCell;
use std::collections::BTreeMap;

use anyhow::Result;

use super::list::Filter;
use super::{NOTICE_END, NOTICE_START, defuse};
use crate::cards;
use crate::detect::Detector;
use crate::findings::{self, FingerprintKey};
use crate::home::Home;
use crate::model::Originals;
use crate::normalize;
use crate::readlog::{self, ReadEntry, ReadResult};
use crate::runtime::LocalRuntime;
use crate::trust::{Access, Reader, Trust};
use crate::util::now_rfc3339;

/// One tag and how many of the matching sessions carry it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TagCount {
    pub tag: String,
    pub sessions: u64,
}

/// What a reader gets.
#[derive(Debug, Clone, PartialEq)]
pub enum Tagged {
    /// For a model: the tags as text between [`NOTICE_START`] and [`NOTICE_END`].
    Text(String),
    /// For the owner and DISTILL itself: the tags, most sessions first.
    Counts(Vec<TagCount>),
    /// Nothing, and why.
    Refused(String),
}

/// The tags on the current cards of the sessions `filter` matches (its own tag filter included),
/// each with how many sessions carry it, most first, then by name. A tag with a finding in it
/// (a personal detail) is left out: a list has no place to cover it. One read log line.
pub fn tags(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    filter: &Filter,
) -> Result<Tagged> {
    let served = serve(home, runtime, reader, filter);
    let result = match &served {
        Ok((Tagged::Refused(reason), _)) => ReadResult::Refused {
            reason: reason.clone(),
        },
        Ok((_, counted)) => counted.clone(),
        Err(e) => ReadResult::Failed {
            error: format!("{e:#}"),
        },
    };
    readlog::append(
        home,
        &ReadEntry {
            at: now_rfc3339(),
            reader: reader.clone(),
            session: None,
            result,
        },
    )?;
    served.map(|(t, _)| t)
}

fn serve(
    home: &Home,
    runtime: &LocalRuntime,
    reader: &Reader,
    filter: &Filter,
) -> Result<(Tagged, ReadResult)> {
    let none = ReadResult::Tags {
        tags: 0,
        sessions: 0,
    };
    let config = home.load_config().unwrap_or_default();
    if let Access::Refused(why) = Trust::new(&config).check(reader, runtime) {
        return Ok((Tagged::Refused(why), none));
    }
    let all = normalize::read_session_list(home)?;
    let originals = Originals::of(&all);
    let key = FingerprintKey::existing(home).ok().flatten();
    let detector = OnceCell::new();
    let finds = |view: &str| {
        !detector
            .get_or_init(|| Detector::new(&findings::owner(&config)))
            .detect(view)
            .is_empty()
    };
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut sessions = 0;
    for m in all.iter().filter(|m| filter.matches(m, &originals)) {
        // The filter's tag and outcome are checked on the card that is counted.
        let Some((card, found)) = cards::stored_current(home, &config, key.as_ref(), m)
            .filter(|(card, _)| filter.fits(card, &config))
        else {
            continue;
        };
        sessions += 1;
        let (shown, found) = cards::viewed(&card.tags, &found, &config.tags, finds);
        for (i, t) in shown.into_iter().enumerate() {
            let field = format!("tags/{i}");
            if !found.iter().any(|f| f.field == field) {
                *counts.entry(t).or_default() += 1;
            }
        }
    }
    let mut list: Vec<TagCount> = counts
        .into_iter()
        .map(|(tag, sessions)| TagCount { tag, sessions })
        .collect();
    list.sort_by(|a, b| b.sessions.cmp(&a.sessions).then_with(|| a.tag.cmp(&b.tag)));
    let counted = ReadResult::Tags {
        tags: list.len() as u64,
        sessions,
    };
    Ok(match reader {
        Reader::Model { .. } => (Tagged::Text(render(&list, sessions)), counted),
        Reader::Owner | Reader::Distill => (Tagged::Counts(list), counted),
    })
}

/// The tags as text for a model.
fn render(list: &[TagCount], sessions: u64) -> String {
    if list.is_empty() {
        return format!("No tags on the {sessions} matching sessions with a current card.\n");
    }
    let body: String = list
        .iter()
        .map(|t| format!("{} ({})\n", t.tag, t.sessions))
        .collect();
    format!(
        "{} tags on {sessions} sessions with a current card, most used first:\n{NOTICE_START}\n{}{NOTICE_END}\n",
        list.len(),
        defuse(&body)
    )
}
