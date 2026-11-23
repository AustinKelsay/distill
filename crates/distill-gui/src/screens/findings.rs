//! Findings (#87): the rotation list. Findings grouped by fingerprint as `distill findings` groups
//! them (`read::fingerprints`), secrets by default, with the same filters (kind, harness, from
//! and to days) and sorts (spread, recency), a page at a time. Pressing a fingerprint lists every
//! session it appears in (`read::where_found`), and each opens in the Sessions screen, where its
//! findings are covered until clicked.
//!
//! Both calls read as the owner through the read path, so each is a line in the read log. They
//! answer from stored findings and session facts alone: a fingerprint and a kind, never a value.

use distill_core::Harness;
use distill_core::detect::Kind;
use distill_core::read::{
    self, Answer, Appearance, FindingFilter, Fingerprints, Kinds, Lookup, Order, Page,
};
use distill_core::trust::Reader;
use rune::{El, ScrollSpec, TextEdit, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Select, SelectAppearance, TextInput};

use crate::State;
use crate::cover::chip;
use crate::jobs::{self, Job};
use crate::nav::Screen;
use crate::screens::sessions;
use crate::ui::{Paint, count, parse_day, thousands};

#[derive(Default)]
pub struct FindingsState {
    pub filters: Filters,
    /// The filter menu that is open, by key.
    pub menu: Option<&'static str>,
    /// The last page asked for; None until the first lands.
    pub list: Option<Fingerprints>,
    pub loading: bool,
    pub error: Option<String>,
    /// Why the filters can't be used as written.
    pub filter_error: Option<String>,
    /// The fingerprint pressed, whole, and where it was found once that lands.
    pub selected: Option<String>,
    pub lookup: Option<Lookup>,
}

/// The filters as the owner sets them; turned into a [`FindingFilter`] when the list is asked for.
#[derive(Default)]
pub struct Filters {
    pub kinds: Kinds,
    pub harness: Option<Harness>,
    pub since: TextEdit,
    pub until: TextEdit,
    pub order: Order,
}

impl Filters {
    fn filter(&self) -> Result<FindingFilter, String> {
        Ok(FindingFilter {
            kinds: self.kinds,
            harness: self.harness,
            since: parse_day(&self.since, "from")?,
            until: parse_day(&self.until, "to")?,
        })
    }
}

/// Moving to the screen reloads the list, and where the selected fingerprint was found.
pub fn open(s: &mut State) {
    s.findings.menu = None;
    ask_page(s, 0);
    if let Some(fp) = s.findings.selected.clone() {
        look_up(s, fp);
    }
}

/// The list from its first page with the current filters.
pub fn apply(s: &mut State) {
    s.findings.selected = None;
    s.findings.lookup = None;
    ask_page(s, 0);
}

fn more(s: &mut State) {
    if let Some(next) = s.findings.list.as_ref().and_then(|l| l.next) {
        ask_page(s, next);
    }
}

fn ask_page(s: &mut State, offset: usize) {
    let st = &mut s.findings;
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
    st.loading = true;
    let order = st.filters.order;
    let page = Page {
        offset,
        limit: read::PAGE_MAX,
    };
    s.ask(Job::new("findings", move |core| {
        let r = read::fingerprints(
            &core.home,
            &core.runtime,
            &Reader::Owner,
            &filter,
            order,
            page,
        )
        .map_err(jobs::line);
        Box::new(move |s: &mut State| {
            let st = &mut s.findings;
            st.loading = false;
            match r {
                Ok(Answer::Served(mut got)) => {
                    if offset > 0
                        && let Some(old) = st.list.take()
                    {
                        let mut rows = old.rows;
                        rows.append(&mut got.rows);
                        got.rows = rows;
                    }
                    st.list = Some(got);
                    st.error = None;
                }
                Ok(Answer::Refused(why)) => st.error = Some(why),
                Err(e) => st.error = Some(format!("Could not list the findings: {e}")),
            }
        })
    }));
}

/// Press a fingerprint: list where it was found, or fold it away again.
fn select(s: &mut State, fingerprint: String) {
    let st = &mut s.findings;
    st.menu = None;
    if st.selected.as_deref() == Some(fingerprint.as_str()) {
        st.selected = None;
        st.lookup = None;
        return;
    }
    st.selected = Some(fingerprint.clone());
    st.lookup = None;
    look_up(s, fingerprint);
}

fn look_up(s: &mut State, fingerprint: String) {
    s.ask(Job::new("where found", move |core| {
        let r = read::where_found(&core.home, &core.runtime, &Reader::Owner, &fingerprint)
            .map_err(jobs::line);
        Box::new(move |s: &mut State| {
            let st = &mut s.findings;
            if st.selected.as_deref() != Some(fingerprint.as_str()) {
                return;
            }
            match r {
                Ok(Answer::Served(l)) => st.lookup = Some(l),
                Ok(Answer::Refused(why)) => st.error = Some(why),
                Err(e) => st.error = Some(format!("Could not look it up: {e}")),
            }
        })
    }));
}

/// The sessions the selected fingerprint appears in, newest first.
fn appearances(st: &FindingsState) -> Vec<&Appearance> {
    st.lookup
        .iter()
        .flat_map(|l| &l.found)
        .flat_map(|f| &f.sessions)
        .collect()
}

fn open_where(s: &mut State, n: usize) {
    let Some(key) = appearances(&s.findings).get(n).map(|a| a.session.clone()) else {
        return;
    };
    sessions::open_key(s, &key, Screen::Findings, |s, e| s.findings.error = Some(e));
}

/// `anthropic key`.
fn kind_words(k: Kind) -> String {
    k.name().replace('_', " ")
}

const SECRETS: &str = "Secrets: the rotation list";
const PERSONAL: &str = "Personal details";
const ALL: &str = "All findings";
const ANY_HARNESS: &str = "Any harness";
const BY_SPREAD: &str = "Most sessions first";
const BY_RECENT: &str = "Most recently seen first";

fn kinds_label(k: Kinds) -> String {
    match k {
        Kinds::Secrets => SECRETS.into(),
        Kinds::PersonalDetails => PERSONAL.into(),
        Kinds::All => ALL.into(),
        Kinds::Only(k) => kind_words(k),
    }
}

fn kinds_of(label: &str) -> Kinds {
    match label {
        PERSONAL => Kinds::PersonalDetails,
        ALL => Kinds::All,
        _ => Kind::ALL
            .into_iter()
            .find(|k| kind_words(*k) == label)
            .map_or(Kinds::Secrets, Kinds::Only),
    }
}

fn toggle_menu(s: &mut State, key: &'static str) {
    s.findings.menu = if s.findings.menu == Some(key) {
        None
    } else {
        Some(key)
    };
}

pub fn view(s: &State, p: &Paint, layer: &mut Vec<El>) -> El {
    col()
        .w_full()
        .h_full()
        .child(header(&s.findings, p, layer))
        .child(p.separator())
        .child(list(&s.findings, p))
}

fn day_field(
    p: &Paint,
    key: &str,
    label: &str,
    edit: &TextEdit,
    get: fn(&mut State) -> &mut TextEdit,
) -> El {
    TextInput::new(edit)
        .label(label)
        .placeholder(format!("{label} YYYY-MM-DD"))
        .width(Some(150.0))
        .on_edit(get)
        .on_submit(|s: &mut State, _: &mut Tx| apply(s))
        .build(&p.k)
        .key(key)
}

fn header(st: &FindingsState, p: &Paint, layer: &mut Vec<El>) -> El {
    let f = &st.filters;
    let kinds: Vec<String> = [SECRETS, PERSONAL, ALL]
        .into_iter()
        .map(str::to_string)
        .chain(Kind::ALL.into_iter().map(kind_words))
        .collect();
    let kinds: Vec<&str> = kinds.iter().map(String::as_str).collect();
    let current = kinds_label(f.kinds);
    let kind = Select::new("filter-kind", &kinds)
        .label("Kind")
        .appearance(SelectAppearance::Field)
        .width(220.0)
        .current(Some(current.as_str()))
        .open(st.menu == Some("filter-kind"))
        .on_toggle(|s: &mut State| toggle_menu(s, "filter-kind"))
        .on_close(|s: &mut State| s.findings.menu = None)
        .on_choose(|s: &mut State, v: &str, _: &mut Tx| {
            s.findings.filters.kinds = kinds_of(v);
            apply(s);
        })
        .build(&p.k, layer);
    let harnesses: Vec<&str> = std::iter::once(ANY_HARNESS)
        .chain(Harness::ALL.iter().map(|h| h.label()))
        .collect();
    let harness = Select::new("filter-harness", &harnesses)
        .label("Harness")
        .appearance(SelectAppearance::Field)
        .width(160.0)
        .current(Some(f.harness.map_or(ANY_HARNESS, |h| h.label())))
        .open(st.menu == Some("filter-harness"))
        .on_toggle(|s: &mut State| toggle_menu(s, "filter-harness"))
        .on_close(|s: &mut State| s.findings.menu = None)
        .on_choose(|s: &mut State, v: &str, _: &mut Tx| {
            s.findings.filters.harness = Harness::ALL.into_iter().find(|h| h.label() == v);
            apply(s);
        })
        .build(&p.k, layer);
    let order = Select::new("filter-order", &[BY_SPREAD, BY_RECENT])
        .label("Sort")
        .appearance(SelectAppearance::Field)
        .width(200.0)
        .current(Some(match f.order {
            Order::Spread => BY_SPREAD,
            Order::Recent => BY_RECENT,
        }))
        .open(st.menu == Some("filter-order"))
        .on_toggle(|s: &mut State| toggle_menu(s, "filter-order"))
        .on_close(|s: &mut State| s.findings.menu = None)
        .on_choose(|s: &mut State, v: &str, _: &mut Tx| {
            s.findings.filters.order = if v == BY_RECENT {
                Order::Recent
            } else {
                Order::Spread
            };
            apply(s);
        })
        .build(&p.k, layer);
    let go = Button::new(if st.loading { "Listing…" } else { "Apply" })
        .variant(ButtonVariant::Submit)
        .on_press(apply)
        .build(&p.k)
        .key("apply");
    let filters = col()
        .key("filters")
        .w_full()
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(kind)
                .child(harness)
                .child(order),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(day_field(p, "filter-since", "From", &f.since, |s| {
                    &mut s.findings.filters.since
                }))
                .child(day_field(p, "filter-until", "To", &f.until, |s| {
                    &mut s.findings.filters.until
                }))
                .child(go),
        );
    let mut c = col()
        .key("header")
        .w_full()
        .px(28.0)
        .pt(20.0)
        .pb(16.0)
        .gap(12.0)
        .child(
            col()
                .gap(2.0)
                .child(
                    text("Findings")
                        .key("heading")
                        .font(p.title())
                        .color(p.ink()),
                )
                .child(p.note(
                    "Every value found in the archive, by fingerprint: what to rotate, and \
                     where it appears. Values are never shown here; open a session to see one.",
                )),
        )
        .child(filters);
    for (key, e) in [("filter-error", &st.filter_error), ("error", &st.error)] {
        if let Some(e) = e {
            c = c.child(
                text(e.clone())
                    .key(key)
                    .font(p.caption())
                    .color(p.bad())
                    .wrap(),
            );
        }
    }
    c
}

fn list(st: &FindingsState, p: &Paint) -> El {
    let body = col()
        .key("results")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let Some(l) = &st.list else {
        return body.child(p.empty("loading", "Reading the findings…"));
    };
    let mut top = col().w_full().px(28.0).pt(14.0).pb(6.0).gap(4.0).child(
        text(match l.total {
            1 => "1 fingerprint".to_string(),
            n => format!("{} fingerprints", thousands(n)),
        })
        .key("summary")
        .font(p.caption())
        .color(p.muted()),
    );
    if l.pending > 0 {
        top = top.child(
            p.note(format!(
                "{} not scanned yet, or scanned by an older detector, so not counted here; the \
                 next scheduled run scans {}.",
                count(l.pending, "session"),
                if l.pending == 1 { "it" } else { "them" }
            ))
            .key("pending"),
        );
    }
    let mut rows = col().key("finding-rows").w_full().px(28.0).gap(10.0);
    if l.rows.is_empty() {
        rows = rows.child(p.empty("none", "No fingerprints match these filters."));
    }
    for (i, r) in l.rows.iter().enumerate() {
        let open = st.selected.as_deref() == Some(r.fingerprint.as_str());
        let kinds: Vec<String> = r.kinds.iter().map(|k| chip(*k)).collect();
        let harnesses: Vec<&str> = r.harnesses.iter().map(|h| h.label()).collect();
        let mut seen = Vec::new();
        if let Some(t) = r.first_seen {
            seen.push(format!("First seen {}", t.format("%b %-d, %Y")));
        }
        if let Some(t) = r.last_seen {
            seen.push(format!("last seen {}", t.format("%b %-d, %Y")));
        }
        let fp = r.fingerprint.as_str().to_string();
        let head = p
            .card()
            .key(format!("finding-{i}"))
            .role("button")
            .checked(open)
            .w_full()
            .gap(6.0)
            .padding(14.0)
            .focusable(true)
            .on_press(move |s: &mut State| select(s, fp.clone()))
            .child(
                row()
                    .w_full()
                    .gap(12.0)
                    .center_y()
                    .child(
                        text(format!("#{}", r.fingerprint.short()))
                            .key("fingerprint")
                            .font(p.mono())
                            .color(p.ink()),
                    )
                    .child(
                        text(kinds.join(" "))
                            .key("kinds")
                            .font(p.text())
                            .color(p.ink())
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(
                        text(count(r.sessions, "session"))
                            .key("spread")
                            .font(p.num())
                            .color(p.ink()),
                    ),
            )
            .child(
                text(format!("{} · {}", harnesses.join(", "), seen.join(", ")))
                    .key("facts")
                    .font(p.caption())
                    .color(p.muted())
                    .wrap(),
            );
        let mut item = col().w_full().gap(6.0).child(p.k.interactive(head));
        if open {
            item = item.child(where_found(st, p));
        }
        rows = rows.child(item);
    }
    let mut body = body.child(top).child(rows);
    if l.next.is_some() {
        body = body.child(
            row().w_full().py(12.0).center().child(
                Button::new(if st.loading { "Loading…" } else { "More" })
                    .on_press(more)
                    .build(&p.k)
                    .key("more"),
            ),
        );
    }
    body.child(col().h(20.0))
}

/// The sessions the selected fingerprint appears in, each with a button that opens it.
fn where_found(st: &FindingsState, p: &Paint) -> El {
    let mut c = col().key("where").w_full().pl(16.0).gap(6.0);
    if st.lookup.is_none() {
        return c.child(p.note("Looking for its sessions…"));
    }
    for (n, a) in appearances(st).into_iter().enumerate() {
        let mut facts = vec![a.harness.label().to_string()];
        if let Some(pr) = &a.project {
            facts.push(pr.clone());
        }
        if let Some(t) = a.started_at {
            facts.push(format!("started {}", t.format("%b %-d, %Y %H:%M UTC")));
        }
        let events: Vec<String> = a.events.iter().map(u64::to_string).collect();
        facts.push(format!(
            "{} {}",
            if a.events.len() == 1 {
                "event"
            } else {
                "events"
            },
            events.join(", ")
        ));
        c = c.child(
            row()
                .key(format!("where-{n}"))
                .w_full()
                .gap(10.0)
                .center_y()
                .padding(8.0)
                .radius(8.0)
                .fill(p.t.hover)
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            text(a.session.clone())
                                .key("session")
                                .font(p.mono())
                                .color(p.ink())
                                .truncate(),
                        )
                        .child(
                            text(facts.join(" · "))
                                .key("facts")
                                .font(p.caption())
                                .color(p.muted())
                                .wrap(),
                        ),
                )
                .child(
                    Button::new("Open session")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut State| open_where(s, n))
                        .build(&p.k)
                        .key(format!("open-where-{n}")),
                ),
        );
    }
    c
}
