//! The list: a search box, the filters and the results with their cards. Findings in titles,
//! summaries, tags, openings and passages are covered here and not clickable: a press on a row
//! opens the session, where each one can be shown.

use std::borrow::Cow;

use distill_core::Harness;
use distill_core::cards::Outcome;
use distill_core::home::HUB;
use distill_core::model::EventKind;
use distill_core::read::Opening;
use distill_core::tags;
use rune::{El, GrowthSpec, ScrollSpec, TextEdit, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Select, SelectAppearance, TextInput};

use super::{ResultRow, SessionsState, card_marks, marks};
use crate::State;
use crate::cover;
use crate::ui::{Paint, ago, count, pill, thousands};

const ANY_HARNESS: &str = "Any harness";
const ANY_MACHINE: &str = "Any machine";
/// The hub's own sessions, in the machine filter.
const THIS_HUB: &str = "This hub";
const ANY_OUTCOME: &str = "Any outcome";
/// How many of the tags in use the list shows as chips.
const TOP_TAGS: usize = 12;

pub fn view(s: &State, p: &Paint, layer: &mut Vec<El>) -> El {
    col()
        .w_full()
        .h_full()
        .child(header(s, p, layer))
        .child(p.separator())
        .child(results(s, p))
}

fn header(s: &State, p: &Paint, layer: &mut Vec<El>) -> El {
    let st = &s.sessions;
    let search = TextInput::new(&st.query)
        .label("Search sessions")
        .placeholder("Words or a description of past work")
        .width(None)
        .on_edit(|s: &mut State| &mut s.sessions.query)
        .on_submit(|s: &mut State, _: &mut Tx| super::search(s))
        .build(&p.k)
        .key("search")
        .grow(1.0)
        .shrink();
    let go = Button::new(if st.loading { "Searching…" } else { "Search" })
        .variant(ButtonVariant::Submit)
        .on_press(super::search)
        .build(&p.k)
        .key("search-button");
    let mut bar = row().w_full().gap(8.0).center_y().child(search).child(go);
    if st.filters.any() || !st.query.text.is_empty() {
        bar = bar.child(
            Button::new("Clear")
                .variant(ButtonVariant::Ghost)
                .on_press(super::clear)
                .build(&p.k)
                .key("clear"),
        );
    }

    let f = &st.filters;
    let harnesses: Vec<&str> = std::iter::once(ANY_HARNESS)
        .chain(Harness::ALL.iter().map(|h| h.label()))
        .collect();
    let harness = Select::new("filter-harness", &harnesses)
        .label("Harness")
        .appearance(SelectAppearance::Field)
        .width(170.0)
        .current(Some(f.harness.map_or(ANY_HARNESS, |h| h.label())))
        .open(st.menu == Some("filter-harness"))
        .on_toggle(|s: &mut State| toggle_menu(s, "filter-harness"))
        .on_close(|s: &mut State| s.sessions.menu = None)
        .on_choose(|s: &mut State, v: &str, _: &mut Tx| {
            s.sessions.filters.harness = Harness::ALL.into_iter().find(|h| h.label() == v);
            super::search(s);
        })
        .build(&p.k, layer);
    // On a hub, its spokes and its own sessions; a home without spokes has no machine filter.
    let spokes: Vec<&str> = s
        .overview
        .status
        .iter()
        .flat_map(|st| st.machines.iter().map(|m| m.machine.as_str()))
        // A spoke named `hub` can't be picked apart from the hub's own (docs/known-gaps.md).
        .filter(|m| *m != HUB)
        .collect();
    let machine = (!spokes.is_empty() || f.machine.is_some()).then(|| {
        let names: Vec<&str> = [ANY_MACHINE, THIS_HUB]
            .into_iter()
            .chain(spokes.iter().copied())
            .collect();
        let current = match f.machine.as_deref() {
            None => ANY_MACHINE,
            Some(HUB) => THIS_HUB,
            Some(m) => m,
        };
        Select::new("filter-machine", &names)
            .label("Machine")
            .appearance(SelectAppearance::Field)
            .width(150.0)
            .current(Some(current))
            .open(st.menu == Some("filter-machine"))
            .on_toggle(|s: &mut State| toggle_menu(s, "filter-machine"))
            .on_close(|s: &mut State| s.sessions.menu = None)
            .on_choose(|s: &mut State, v: &str, _: &mut Tx| {
                s.sessions.filters.machine = match v {
                    ANY_MACHINE => None,
                    THIS_HUB => Some(HUB.to_string()),
                    m => Some(m.to_string()),
                };
                super::search(s);
            })
            .build(&p.k, layer)
    });
    let outcomes: Vec<&str> = std::iter::once(ANY_OUTCOME)
        .chain(Outcome::ALL.iter().map(|o| outcome_label(*o)))
        .collect();
    let outcome = Select::new("filter-outcome", &outcomes)
        .label("Outcome")
        .appearance(SelectAppearance::Field)
        .width(150.0)
        .current(Some(f.outcome.map_or(ANY_OUTCOME, outcome_label)))
        .open(st.menu == Some("filter-outcome"))
        .on_toggle(|s: &mut State| toggle_menu(s, "filter-outcome"))
        .on_close(|s: &mut State| s.sessions.menu = None)
        .on_choose(|s: &mut State, v: &str, _: &mut Tx| {
            s.sessions.filters.outcome = Outcome::ALL.into_iter().find(|o| outcome_label(*o) == v);
            super::search(s);
        })
        .build(&p.k, layer);

    let filters = col()
        .key("filters")
        .w_full()
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(harness)
                .children(machine)
                .child(field(
                    p,
                    "filter-model",
                    "Model",
                    "any model",
                    &f.model,
                    |s| &mut s.sessions.filters.model,
                ))
                .child(field(
                    p,
                    "filter-project",
                    "Project",
                    "a folder, e.g. /work/app",
                    &f.project,
                    |s| &mut s.sessions.filters.project,
                )),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(field(
                    p,
                    "filter-since",
                    "From",
                    "YYYY-MM-DD",
                    &f.since,
                    |s| &mut s.sessions.filters.since,
                ))
                .child(field(
                    p,
                    "filter-until",
                    "To",
                    "YYYY-MM-DD",
                    &f.until,
                    |s| &mut s.sessions.filters.until,
                ))
                .child(field(p, "filter-tag", "Tags", "any tags", &f.tag, |s| {
                    &mut s.sessions.filters.tag
                }))
                .child(outcome),
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
                    text("Sessions")
                        .key("heading")
                        .font(p.title())
                        .color(p.ink()),
                )
                .child(p.note(
                    "Everything you have archived. Secrets and personal details are covered; \
                     open a session and click one to show it.",
                )),
        )
        .child(bar)
        .child(filters);
    if !st.tags.is_empty() {
        c = c.child(top_tags(st, p));
    }
    if let Some(e) = &st.filter_error {
        c = c.child(
            text(e.clone())
                .key("filter-error")
                .font(p.caption())
                .color(p.bad())
                .wrap(),
        );
    }
    c
}

/// The most used tags on the sessions the other filters match; a press filters by one too, and a
/// second press on it stops.
fn top_tags(st: &SessionsState, p: &Paint) -> El {
    let picked: Vec<String> = st
        .filters
        .tags()
        .iter()
        .map(|t| tags::standard(t))
        .collect();
    let mut chips = row().key("top-tags").w_full().gap(6.0).flex_wrap();
    for (i, t) in st.tags.iter().take(TOP_TAGS).enumerate() {
        let tag = t.tag.clone();
        chips = chips.child(
            Button::new(format!("#{} · {}", t.tag, thousands(t.sessions)))
                .variant(if picked.contains(&tags::standard(&t.tag)) {
                    ButtonVariant::Submit
                } else {
                    ButtonVariant::Outline
                })
                .control_size(ControlSize::Small)
                .disabled(st.loading || st.tags_loading)
                .on_press(move |s: &mut State| super::pick_tag(s, &tag))
                .build(&p.k)
                .key(format!("top-tag-{i}")),
        );
    }
    chips
}

fn field(
    p: &Paint,
    key: &str,
    label: &str,
    placeholder: &str,
    edit: &TextEdit,
    get: fn(&mut State) -> &mut TextEdit,
) -> El {
    TextInput::new(edit)
        .label(label)
        .placeholder(placeholder)
        .width(None)
        .on_edit(get)
        .on_submit(|s: &mut State, _: &mut Tx| super::search(s))
        .build(&p.k)
        .key(key)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
}

fn toggle_menu(s: &mut State, key: &'static str) {
    s.sessions.menu = if s.sessions.menu == Some(key) {
        None
    } else {
        Some(key)
    };
}

pub fn outcome_label(o: Outcome) -> &'static str {
    match o {
        Outcome::Done => "Done",
        Outcome::PartlyDone => "Partly done",
        Outcome::Failed => "Failed",
        Outcome::Unknown => "Unknown",
    }
}

fn results(s: &State, p: &Paint) -> El {
    let st = &s.sessions;
    let body = col()
        .key("results")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let Some(r) = &st.results else {
        let msg = match &st.error {
            Some(e) => e.clone(),
            None => "Reading the archive…".into(),
        };
        return body.child(p.empty("no-results", &msg));
    };
    let summary = match &r.query {
        None => format!("{}, newest first", count(r.total, "session")),
        Some(q) if r.total == 1 => format!("1 session matches “{q}”"),
        Some(q) => format!("{} sessions match “{q}”", thousands(r.total)),
    };
    let mut top = col().w_full().px(28.0).pt(14.0).pb(6.0).gap(4.0).child(
        text(summary)
            .key("summary")
            .font(p.caption())
            .color(p.muted()),
    );
    for (i, n) in r.notes.iter().enumerate() {
        top = top.child(p.note(n.clone()).key(format!("note-{i}")));
    }
    if let Some(e) = &st.error {
        top = top.child(
            text(e.clone())
                .key("error")
                .font(p.caption())
                .color(p.bad())
                .wrap(),
        );
    }
    let p2 = p.clone();
    let list = col()
        .key("result-rows")
        .w_full()
        .px(28.0)
        .growth(GrowthSpec::default())
        .virtual_rows(
            r.rows.len(),
            96.0,
            |_: &State, i| Cow::Owned(format!("result-{i}")),
            move |s: &State, i| match s.sessions.results.as_ref().and_then(|r| r.rows.get(i)) {
                Some(row) => result_row(s, row, i, &p2),
                None => col(),
            },
        )
        .virtual_rows_version(st.version);
    let mut body = body.child(top);
    if r.rows.is_empty() {
        body = body.child(p.empty(
            "no-results",
            if r.query.is_some() {
                "No session matches. Try other words, or fewer filters."
            } else {
                "No session matches these filters."
            },
        ));
    } else {
        body = body.child(list);
    }
    if r.next.is_some() {
        body = body.child(
            row().w_full().py(12.0).center().child(
                Button::new(if st.loading { "Loading…" } else { "More" })
                    .on_press(super::more)
                    .build(&p.k)
                    .key("more"),
            ),
        );
    }
    body.child(col().h(20.0))
}

/// One result: its card's title (or the opening), the facts, the summary and tags, the outcome,
/// and the matching passage of a search.
fn result_row(s: &State, r: &ResultRow, i: usize, p: &Paint) -> El {
    let never = |_: &str| false;
    let (title, title_color) = match (&r.card, &r.opening) {
        (Some(c), _) => (
            cover::text(p, &c.card.title, &card_marks(c, "title"), never, false),
            p.ink(),
        ),
        (None, Opening::Text(t)) => (
            cover::text(p, t, &marks(&r.opening_findings, "o"), never, false),
            p.ink(),
        ),
        (None, Opening::None) => (text("No message from you"), p.muted()),
        (None, Opening::Withheld(why)) => (text(format!("Withheld: {why}")), p.bad()),
    };
    let mut head = row().w_full().gap(10.0).center_y().child(
        title
            .key("title")
            .font(p.heading())
            .color(title_color)
            .wrap()
            .max_lines(2)
            .grow(1.0)
            .shrink()
            .min_w(0.0),
    );
    if let Some(c) = &r.card {
        let role = match c.card.outcome {
            Outcome::Done => p.k.c.success,
            Outcome::PartlyDone => p.k.c.warning,
            Outcome::Failed => p.k.c.danger,
            Outcome::Unknown => p.k.c.info,
        };
        head = head.child(pill(p, outcome_label(c.card.outcome), role).key("outcome"));
    }
    let mut card = col().w_full().gap(6.0).child(head).child(
        text(facts_line(s, &r.facts))
            .key("facts")
            .font(p.caption())
            .color(p.muted())
            .truncate(),
    );
    if let Some(from) = &r.started_from {
        card = card.child(
            text(format!(
                "Started from {} · {}",
                from.key(),
                facts_line(s, from)
            ))
            .key("started-from")
            .font(p.caption())
            .color(p.faint())
            .truncate(),
        );
    }
    if let Some(c) = &r.card {
        card = card.child(
            cover::text(p, &c.card.summary, &card_marks(c, "summary"), never, false)
                .key("card-summary")
                .font(p.text())
                .color(p.ink())
                .wrap()
                .max_lines(3),
        );
        if !c.card.tags.is_empty() {
            let mut tags = row().key("tags").gap(6.0).center_y();
            for (n, t) in c.card.tags.iter().enumerate() {
                let shown = cover::cover(p, t, &card_marks(c, &format!("tags/{n}")), never, false);
                tags = tags.child(
                    text(format!("#{}", shown.text))
                        .font(p.caption())
                        .color(p.muted()),
                );
            }
            card = card.child(tags);
        }
    }
    if let Some(m) = &r.passage {
        let mut shown = String::new();
        if m.cut_before {
            shown.push('…');
        }
        let base = shown.len();
        shown.push_str(&m.text);
        if m.cut_after {
            shown.push('…');
        }
        let mut ms = marks(&m.findings, "p");
        for x in &mut ms {
            x.start += base;
            x.end += base;
        }
        card = card.child(
            col()
                .key("passage")
                .w_full()
                .gap(2.0)
                .padding(8.0)
                .radius(8.0)
                .fill(p.t.hover)
                .child(
                    text(format!("Matched in {}", kind_words(m.kind)))
                        .font(p.caption())
                        .color(p.faint()),
                )
                .child(
                    cover::text(p, &shown, &ms, never, false)
                        .key("passage-text")
                        .font(p.mono())
                        .color(p.ink())
                        .wrap()
                        .max_lines(4),
                ),
        );
    }
    let item = p
        .card()
        .key(format!("result-{i}"))
        .role("button")
        .w_full()
        .gap(8.0)
        .padding(14.0)
        .focusable(true)
        .on_press(move |s: &mut State| super::open_row(s, i))
        .child(card);
    col().w_full().pb(10.0).child(p.k.interactive(item))
}

/// `Claude Code · /work/app · claude-opus-5-5 · Nov 10 · 6 events`, after the machine for a
/// spoke's session.
pub fn facts_line(s: &State, f: &distill_core::model::SessionMeta) -> String {
    let mut parts: Vec<String> = f.machine.iter().cloned().collect();
    parts.push(f.harness.label().to_string());
    if let Some(cwd) = &f.cwd {
        parts.push(cwd.clone());
    }
    if !f.models.is_empty() {
        parts.push(f.models.join(", "));
    }
    if let Some(t) = f.started_at.as_deref().or(f.ended_at.as_deref()) {
        parts.push(ago(t, s.now()));
    }
    parts.push(count(f.counts.events, "event"));
    parts.join(" · ")
}

/// `tool result`, as a passage or heading says it.
pub fn kind_words(k: EventKind) -> &'static str {
    match k {
        EventKind::System => "system text",
        EventKind::User => "your message",
        EventKind::Assistant => "the assistant's reply",
        EventKind::Reasoning => "reasoning",
        EventKind::ToolCall => "a tool call",
        EventKind::ToolResult => "a tool result",
    }
}
