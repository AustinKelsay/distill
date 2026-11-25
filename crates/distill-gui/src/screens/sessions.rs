//! Sessions (#86): the owner searches and reads their own sessions. Every finding, secret or
//! personal detail, is covered by a chip naming its kind (`cover.rs`) until clicked; a click
//! shows that one finding, and closing the session, or leaving the screen, covers it again.
//!
//! Everything comes from core's read path as the owner: `read::list` with no words to search
//! for, `read::search` with some, and `read::read` to open a session, so each list, search and
//! open is a line in the read log. The calls run as jobs off the frame loop.

mod doc;
mod list;
mod session;

use std::collections::BTreeSet;

use distill_core::Harness;
use distill_core::cards::{Outcome, Shown};
use distill_core::findings::Finding;
use distill_core::model::SessionMeta;
use distill_core::read::{
    self, Filter, Listed, Opening, Page, Passage, Range, Read, Searched, TagCount, Tagged,
};
use distill_core::tags::{self, TagsConfig};
use distill_core::trust::Reader;
use rune::{El, TextEdit};

use crate::State;
use crate::jobs::{self, Job};
use crate::nav::{self, Screen};
use crate::ui::{Paint, parse_day};

#[cfg(test)]
pub use doc::BLOCK_LINES;
pub use doc::SessionDoc;

#[derive(Default)]
pub struct SessionsState {
    /// Words to search for; empty lists the archive newest first.
    pub query: TextEdit,
    pub filters: Filters,
    /// The filter menu that is open, by key.
    pub menu: Option<&'static str>,
    /// The last list or search; None until the first lands.
    pub results: Option<Results>,
    pub loading: bool,
    pub error: Option<String>,
    /// Why the filters can't be used as written.
    pub filter_error: Option<String>,
    /// Bumped whenever the results change, so the virtual list asks for its rows again.
    pub version: u64,
    /// The session being read, if one is open.
    pub open: Option<Open>,
    /// The tags in use on the sessions the other filters match, most used first (#157).
    pub tags: Vec<TagCount>,
    /// The filter `tags` was last asked for, without its tag.
    pub tags_for: Option<Filter>,
    /// Bumped with each ask for `tags`; only the answer to the latest lands.
    pub tags_asked: u64,
    /// Whether that answer is still to come.
    pub tags_loading: bool,
}

/// The filters as the owner writes them; turned into a [`Filter`] when a search runs.
#[derive(Default)]
pub struct Filters {
    pub harness: Option<Harness>,
    pub model: TextEdit,
    pub project: TextEdit,
    pub since: TextEdit,
    pub until: TextEdit,
    pub tag: TextEdit,
    pub outcome: Option<Outcome>,
}

impl Filters {
    fn filter(&self) -> Result<Filter, String> {
        let words = |t: &TextEdit| Some(t.text.trim().to_string()).filter(|s| !s.is_empty());
        Ok(Filter {
            harness: self.harness,
            project: words(&self.project),
            model: words(&self.model),
            since: parse_day(&self.since, "from")?,
            until: parse_day(&self.until, "to")?,
            tag: words(&self.tag),
            outcome: self.outcome,
            ..Filter::default()
        })
    }

    fn any(&self) -> bool {
        self.harness.is_some()
            || self.outcome.is_some()
            || [
                &self.model,
                &self.project,
                &self.since,
                &self.until,
                &self.tag,
            ]
            .iter()
            .any(|t| !t.text.trim().is_empty())
    }
}

/// A page of sessions as the list shows them.
#[derive(Debug, Clone)]
pub struct Results {
    pub rows: Vec<ResultRow>,
    pub total: u64,
    pub next: Option<usize>,
    /// The words searched for, or None for the whole archive.
    pub query: Option<String>,
    /// What the owner should know about the search: meaning off, sessions not indexed yet.
    pub notes: Vec<String>,
}

/// One session in the list: a [`read::Row`] or [`read::Hit`].
#[derive(Debug, Clone)]
pub struct ResultRow {
    pub facts: SessionMeta,
    pub card: Option<Shown>,
    pub opening: Opening,
    pub opening_findings: Vec<Finding>,
    pub passage: Option<Passage>,
    /// For a search hit an agent started: the session the owner started it from.
    pub started_from: Option<SessionMeta>,
}

/// The open session.
pub struct Open {
    pub key: String,
    /// Its row in the list: the card and opening its heading shows.
    pub row: ResultRow,
    pub loaded: Loaded,
    /// The findings shown, by mark id; all others are covered.
    pub revealed: BTreeSet<String>,
    /// The screen it was opened from, when not the list: Back returns there.
    pub from: Option<Screen>,
    /// A tag the owner is adding to it (#158).
    pub new_tag: TextEdit,
    /// Why the last tag edit didn't take.
    pub tag_error: Option<String>,
    /// Whether a tag edit is being written.
    pub tagging: bool,
}

impl Open {
    pub fn doc(&self) -> Option<&SessionDoc> {
        match &self.loaded {
            Loaded::Shown { doc, .. } => Some(doc),
            _ => None,
        }
    }
}

pub enum Loaded {
    Loading,
    Refused(String),
    Failed(String),
    Shown {
        facts: Box<SessionMeta>,
        /// Events in the session.
        total: u64,
        doc: Box<SessionDoc>,
    },
}

/// Moving to the screen covers every finding again, and loads the list when no session is open.
pub fn open(s: &mut State) {
    s.sessions.menu = None;
    match &mut s.sessions.open {
        Some(o) => o.revealed.clear(),
        None => search(s),
    }
}

pub fn view(s: &State, p: &Paint, layer: &mut Vec<El>) -> El {
    match &s.sessions.open {
        Some(o) => session::view(s, o, p),
        None => list::view(s, p, layer),
    }
}

/// Run the search (or the list, with no words) with the current filters, from the first page.
pub fn search(s: &mut State) {
    ask_page(s, 0);
}

/// The next page of the current results, added below them.
pub fn more(s: &mut State) {
    if let Some(next) = s.sessions.results.as_ref().and_then(|r| r.next) {
        ask_page(s, next);
    }
}

fn ask_page(s: &mut State, offset: usize) {
    let st = &mut s.sessions;
    st.menu = None;
    let filter = match st.filters.filter() {
        Ok(f) => f,
        Err(e) => {
            st.filter_error = Some(e);
            return;
        }
    };
    st.filter_error = None;
    if st.loading {
        return;
    }
    if offset == 0 {
        ask_tags(s, &filter);
    }
    let st = &mut s.sessions;
    st.loading = true;
    let query = Some(st.query.text.trim().to_string()).filter(|q| !q.is_empty());
    let page = Page {
        offset,
        limit: read::PAGE_MAX,
    };
    s.ask(Job::new("sessions", move |core| {
        let r = fetch(core, query, &filter, page).map_err(jobs::line);
        Box::new(move |s: &mut State| {
            let st = &mut s.sessions;
            st.loading = false;
            match r {
                Ok(Ok(mut got)) => {
                    if offset > 0
                        && let Some(old) = st.results.take()
                    {
                        let mut rows = old.rows;
                        rows.append(&mut got.rows);
                        got.rows = rows;
                    }
                    st.results = Some(got);
                    st.error = None;
                    st.version += 1;
                }
                Ok(Err(refused)) => st.error = Some(refused),
                Err(e) => st.error = Some(e),
            }
        })
    }));
}

/// The tags in use on the sessions `filter` matches whatever its tag, asked again with every
/// first page so a chip never outlives a finding. A refusal or failure leaves no chips; the list
/// says what went wrong.
fn ask_tags(s: &mut State, filter: &Filter) {
    let all = Filter {
        tag: None,
        ..filter.clone()
    };
    s.sessions.tags_for = Some(all.clone());
    s.sessions.tags_loading = true;
    s.sessions.tags_asked += 1;
    let asked = s.sessions.tags_asked;
    s.ask(Job::new("tags", move |core| {
        let r = read::tags(&core.home, &core.runtime, &Reader::Owner, &all);
        Box::new(move |s: &mut State| {
            if s.sessions.tags_asked == asked {
                s.sessions.tags_loading = false;
                s.sessions.tags = match r {
                    Ok(Tagged::Counts(c)) => c,
                    _ => Vec::new(),
                };
            }
        })
    }));
}

/// Filter by `tag`, or stop filtering by it when it is the tag filtered by: back on the list,
/// from the first page.
pub fn pick_tag(s: &mut State, tag: &str) {
    let f = &mut s.sessions.filters.tag;
    if f.text.trim() == tag {
        *f = TextEdit::default();
    } else {
        f.replace_text(tag);
    }
    s.sessions.open = None;
    search(s);
}

/// Add the tag `add` to the open session or hide `hide` on it, as its owner (#158). Core writes
/// it; the open session shows it at once, and the list and the chips at the search it starts.
pub fn retag(s: &mut State, add: Option<String>, hide: Option<String>) {
    let Some(o) = s.sessions.open.as_mut().filter(|o| !o.tagging) else {
        return;
    };
    o.tagging = true;
    let key = o.key.clone();
    s.ask(Job::new("tag session", move |core| {
        let r = core
            .home
            .load_config()
            .and_then(|config| {
                let (add, hide): (Vec<String>, Vec<String>) = (
                    add.iter().cloned().collect(),
                    hide.iter().cloned().collect(),
                );
                tags::tag_session(&core.home, &config, &key, &add, &hide)?;
                Ok(config.tags)
            })
            .map_err(jobs::line);
        Box::new(move |s: &mut State| {
            let Some(o) = s.sessions.open.as_mut().filter(|o| o.key == key) else {
                return;
            };
            o.tagging = false;
            match r {
                Ok(config) => {
                    o.tag_error = None;
                    if add.is_some() {
                        o.new_tag = TextEdit::default();
                    }
                    if let Some(c) = &mut o.row.card {
                        retagged(c, add.as_deref(), hide.as_deref(), &config);
                    }
                    search(s);
                }
                Err(e) => o.tag_error = Some(e),
            }
        })
    }));
}

/// `card` with `hide` gone, its findings' places following, and `add` after the rest, as core
/// shows them once the edit is written.
fn retagged(card: &mut Shown, add: Option<&str>, hide: Option<&str>, config: &TagsConfig) {
    if let Some(h) = hide.map(|h| tags::view(h, config)) {
        while let Some(j) = card
            .card
            .tags
            .iter()
            .position(|t| tags::view(t, config) == h)
        {
            card.card.tags.remove(j);
            card.findings.retain(|f| f.field != format!("tags/{j}"));
            for f in &mut card.findings {
                if let Some(k) = f
                    .field
                    .strip_prefix("tags/")
                    .and_then(|k| k.parse::<usize>().ok())
                    && k > j
                {
                    f.field = format!("tags/{}", k - 1);
                }
            }
        }
    }
    if let Some(a) = add.map(|a| tags::view(a, config))
        && !card.card.tags.iter().any(|t| tags::view(t, config) == a)
    {
        card.card.tags.push(a);
    }
}

/// A list or search as the owner. The inner error is a refusal.
fn fetch(
    core: &jobs::Core,
    query: Option<String>,
    filter: &Filter,
    page: Page,
) -> anyhow::Result<Result<Results, String>> {
    let (home, rt) = (&core.home, &core.runtime);
    let Some(q) = query else {
        return Ok(match read::list(home, rt, &Reader::Owner, filter, page)? {
            Listed::Rows(r) => Ok(Results {
                total: r.total,
                next: r.next,
                query: None,
                notes: Vec::new(),
                rows: r
                    .rows
                    .into_iter()
                    .map(|r| ResultRow {
                        facts: r.facts,
                        card: r.card,
                        opening: r.opening,
                        opening_findings: r.opening_findings,
                        passage: None,
                        started_from: None,
                    })
                    .collect(),
            }),
            Listed::Refused(why) => Err(why),
            Listed::Text(_) => Err("the read path answered as for a model".into()),
        });
    };
    Ok(
        match read::search(home, rt, &Reader::Owner, &q, filter, page)? {
            Searched::Hits(h) => {
                let mut notes = Vec::new();
                notes.extend(meaning_note(&h.meaning));
                if h.unindexed > 0 {
                    notes.push(format!(
                        "{} not indexed yet, so a search can't find {}.",
                        crate::ui::count(h.unindexed, "session"),
                        if h.unindexed == 1 { "it" } else { "them" }
                    ));
                }
                Ok(Results {
                    total: h.total,
                    next: h.next,
                    query: Some(q),
                    notes,
                    rows: h
                        .hits
                        .into_iter()
                        .map(|h| ResultRow {
                            facts: h.facts,
                            card: h.card,
                            opening: h.opening,
                            opening_findings: h.opening_findings,
                            passage: h.passage,
                            started_from: h.started_from,
                        })
                        .collect(),
                })
            }
            Searched::Refused(why) => Err(why),
            Searched::Text(_) => Err("the read path answered as for a model".into()),
        },
    )
}

/// Clear the words and filters and list the archive again.
pub fn clear(s: &mut State) {
    s.sessions.query = TextEdit::default();
    s.sessions.filters = Filters::default();
    search(s);
}

/// Open the `i`th result: read it through the read path as the owner.
pub fn open_row(s: &mut State, i: usize) {
    let Some(row) = s
        .sessions
        .results
        .as_ref()
        .and_then(|r| r.rows.get(i))
        .cloned()
    else {
        return;
    };
    let key = row.facts.key();
    s.sessions.menu = None;
    s.sessions.open = Some(Open {
        key: key.clone(),
        row,
        loaded: Loaded::Loading,
        revealed: BTreeSet::new(),
        from: None,
        new_tag: TextEdit::default(),
        tag_error: None,
        tagging: false,
    });
    // Its tags are pressable only as the tags in use list them now, so one that became a
    // finding since the list was read can't be put in the Tag field.
    if let Some(f) = s.sessions.tags_for.clone() {
        ask_tags(s, &f);
    }
    s.ask(Job::new("read session", move |core| {
        let r = read::read(
            &core.home,
            &core.runtime,
            &Reader::Owner,
            &key,
            Range::default(),
        )
        .map_err(jobs::line);
        // Laid out here, on the worker: a long session takes a while to cut into rows.
        let loaded = loaded(r);
        Box::new(move |s: &mut State| land(s, &key, loaded))
    }));
}

/// What a read gave, ready to draw.
pub fn loaded(r: Result<Read, String>) -> Loaded {
    match r {
        Ok(Read::Events(x)) => Loaded::Shown {
            doc: Box::new(SessionDoc::of(&x)),
            total: x.total,
            facts: Box::new(x.facts),
        },
        Ok(Read::Refused(why)) => Loaded::Refused(why),
        Ok(Read::Text(_)) => Loaded::Failed("the read path answered as for a model".into()),
        Err(e) => Loaded::Failed(e),
    }
}

/// A read of `key`, onto the open session if it is still that one.
pub fn land(s: &mut State, key: &str, loaded: Loaded) {
    if let Some(o) = s.sessions.open.as_mut().filter(|o| o.key == key) {
        o.loaded = loaded;
    }
}

/// Open the session `key` from another screen (Findings, Approvals): read it as the owner, then
/// show it here, where Back returns to `from`. A refusal or failure is handed to `failed`, for
/// the screen that asked, and nothing moves.
pub fn open_key(s: &mut State, key: &str, from: Screen, failed: fn(&mut State, String)) {
    let key = key.to_string();
    s.ask(Job::new("read session", move |core| {
        let r = read::read(
            &core.home,
            &core.runtime,
            &Reader::Owner,
            &key,
            Range::default(),
        );
        let got = match r {
            Ok(Read::Events(x)) => {
                let row = ResultRow {
                    facts: x.facts.clone(),
                    card: None,
                    opening: Opening::None,
                    opening_findings: Vec::new(),
                    passage: None,
                    started_from: None,
                };
                Ok((row, loaded(Ok(Read::Events(x)))))
            }
            Ok(Read::Refused(why)) => Err(format!("Could not open {key}: {why}")),
            Ok(Read::Text(_)) => Err("the read path answered as for a model".into()),
            Err(e) => Err(format!("Could not open {key}: {}", jobs::line(e))),
        };
        Box::new(move |s: &mut State| match got {
            Ok((row, loaded)) => {
                s.sessions.menu = None;
                s.sessions.open = Some(Open {
                    key: row.facts.key(),
                    row,
                    loaded,
                    revealed: BTreeSet::new(),
                    from: Some(from),
                    new_tag: TextEdit::default(),
                    tag_error: None,
                    tagging: false,
                });
                nav::go(s, Screen::Sessions);
            }
            Err(e) => failed(s, e),
        })
    }));
}

/// Close the open session: every finding in it is covered again next time. A session opened
/// from another screen goes back to it.
pub fn close(s: &mut State) {
    let from = s.sessions.open.take().and_then(|o| o.from);
    match from {
        Some(screen) => nav::go(s, screen),
        None if s.sessions.results.is_none() => search(s),
        None => {}
    }
}

/// Show the finding `id` of the open session, or cover it again.
pub fn toggle(s: &mut State, id: &str) {
    if let Some(o) = &mut s.sessions.open
        && !o.revealed.remove(id)
    {
        o.revealed.insert(id.to_string());
    }
}

/// The marks of a card's field (`title`, `summary`, `tags/<n>`); ids are `c<n>`.
fn card_marks(card: &Shown, field: &str) -> Vec<crate::cover::Mark> {
    card.findings
        .iter()
        .enumerate()
        .filter(|(_, f)| f.field == field)
        .map(|(n, f)| crate::cover::Mark {
            start: f.start,
            end: f.end,
            kind: f.kind,
            id: format!("c{n}"),
        })
        .collect()
}

/// The marks of findings placed on a short string (an opening, a passage), with ids `<prefix><n>`.
fn marks(found: &[Finding], prefix: &str) -> Vec<crate::cover::Mark> {
    found
        .iter()
        .enumerate()
        .map(|(n, f)| crate::cover::Mark {
            start: f.start,
            end: f.end,
            kind: f.kind,
            id: format!("{prefix}{n}"),
        })
        .collect()
}

/// For tests: every value the screen draws covered now, and every one it shows.
#[cfg(test)]
pub fn values(st: &SessionsState) -> (Vec<String>, Vec<String>) {
    type Out = (Vec<String>, Vec<String>);
    fn put(
        out: &mut Out,
        src: &str,
        ms: &[crate::cover::Mark],
        revealed: Option<&BTreeSet<String>>,
    ) {
        for m in crate::cover::fit(src, ms) {
            let v = src[m.start..m.end].to_string();
            if revealed.is_some_and(|r| r.contains(&m.id)) {
                out.1.push(v);
            } else if !v.trim().is_empty() {
                out.0.push(v);
            }
        }
    }
    fn card(out: &mut Out, c: &Shown, revealed: Option<&BTreeSet<String>>) {
        for (field, text) in c.card.fields() {
            put(out, text, &card_marks(c, &field), revealed);
        }
    }
    fn head(out: &mut Out, r: &ResultRow, revealed: Option<&BTreeSet<String>>) {
        match (&r.card, &r.opening) {
            (Some(c), _) => card(out, c, revealed),
            (None, Opening::Text(t)) => put(out, t, &marks(&r.opening_findings, "o"), revealed),
            _ => {}
        }
    }
    let mut out: Out = Default::default();
    // Only what the screen draws: the open session, or else the list.
    match &st.open {
        Some(o) => {
            head(&mut out, &o.row, Some(&o.revealed));
            for e in o.doc().iter().flat_map(|d| &d.events) {
                put(&mut out, &e.text, &e.marks, Some(&o.revealed));
            }
        }
        None => {
            for r in st.results.iter().flat_map(|r| &r.rows) {
                head(&mut out, r, None);
                if let Some(p) = &r.passage {
                    put(&mut out, &p.text, &marks(&p.findings, "p"), None);
                }
            }
        }
    }
    out
}

/// What a search's results say about search by meaning: off, and why; or on but covering only
/// part of the archive while the vectors catch up (issue #153); or nothing when it covered all.
pub(crate) fn meaning_note(m: &read::Meaning) -> Option<String> {
    match m {
        read::Meaning::Skipped(why) => Some(format!(
            "Search by meaning is off ({why}): word matches only."
        )),
        read::Meaning::Used { unembedded: 0 } => None,
        read::Meaning::Used { unembedded: n } => Some(format!(
            "{} {} no vectors yet, so only {} words find {}.",
            crate::ui::count(*n, "session"),
            if *n == 1 { "has" } else { "have" },
            if *n == 1 { "its" } else { "their" },
            if *n == 1 { "it" } else { "them" }
        )),
    }
}
