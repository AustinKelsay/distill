//! One open session: its heading (card or opening, facts) and its events in order, each told
//! apart by a heading and its own type, in virtual rows of at most 40 lines. Every finding is a
//! chip until clicked; a click shows that one, a second covers it again.

use std::borrow::Cow;

use distill_core::model::EventKind;
use distill_core::read::Opening;
use rune::{Color, El, GrowthSpec, ScrollSpec, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use super::doc::SessionDoc;
use super::list::{facts_line, outcome_label};
use super::{Loaded, Open, card_marks, marks};
use crate::State;
use crate::cover;
use crate::ui::{Paint, count, dot};

pub fn view(s: &State, o: &Open, p: &Paint) -> El {
    let body = match &o.loaded {
        Loaded::Loading => p.empty("loading", "Reading the session…"),
        Loaded::Refused(why) => p.empty("refused", why),
        Loaded::Failed(e) => p.empty("failed", &format!("Could not read the session: {e}")),
        Loaded::Shown { doc, .. } => events(o, doc, p),
    };
    col()
        .w_full()
        .h_full()
        .child(header(s, o, p))
        .child(p.separator())
        .child(body.grow(1.0).min_h(0.0))
}

fn shown(o: &Open) -> impl Fn(&str) -> bool + '_ {
    |id| o.revealed.contains(id)
}

fn on_chip(s: &mut State, id: &str, _: &mut Tx) {
    super::toggle(s, id);
}

fn header(s: &State, o: &Open, p: &Paint) -> El {
    let r = &o.row;
    let title = match (&r.card, &r.opening) {
        (Some(c), _) => cover::text(p, &c.card.title, &card_marks(c, "title"), shown(o), true),
        (None, Opening::Text(t)) => {
            cover::text(p, t, &marks(&r.opening_findings, "o"), shown(o), true)
        }
        (None, _) => text(r.facts.key()),
    }
    .key("session-title")
    .font(p.title())
    .color(p.ink())
    .wrap()
    .selectable(true)
    .on_link(on_chip);
    let facts = match &o.loaded {
        Loaded::Shown { facts, .. } => facts_line(s, facts),
        _ => facts_line(s, &r.facts),
    };
    let mut about = col().grow(1.0).min_w(0.0).gap(4.0).child(title).child(
        text(facts)
            .key("session-facts")
            .font(p.caption())
            .color(p.muted())
            .wrap(),
    );
    if let Some(c) = &r.card {
        about = about.child(
            cover::text(
                p,
                &c.card.summary,
                &card_marks(c, "summary"),
                shown(o),
                true,
            )
            .key("session-summary")
            .font(p.text())
            .color(p.ink())
            .wrap()
            .selectable(true)
            .on_link(on_chip),
        );
        // A tag with a finding stays covered and can't be pressed; any other filters the list.
        let mut tags = row()
            .key("session-tags")
            .gap(6.0)
            .center_y()
            .flex_wrap()
            .child(
                text(format!("Outcome: {}", outcome_label(c.card.outcome)))
                    .font(p.caption())
                    .color(p.muted()),
            );
        for (n, t) in c.card.tags.iter().enumerate() {
            let ms = card_marks(c, &format!("tags/{n}"));
            tags = tags.child(if ms.is_empty() {
                let tag = t.clone();
                Button::new(format!("#{t}"))
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut State| super::pick_tag(s, &tag))
                    .build(&p.k)
                    .key(format!("session-tag-{n}"))
            } else {
                text(format!(
                    "#{}",
                    cover::cover(p, t, &ms, |_| false, false).text
                ))
                .font(p.caption())
                .color(p.muted())
            });
        }
        about = about.child(tags);
    }
    if let Some(d) = o.doc() {
        let note = if d.findings == 0 {
            "No secrets or personal details found.".to_string()
        } else {
            let open = o.revealed.iter().filter(|id| id.starts_with('f')).count();
            let mut n = format!(
                "{} covered ({} secret{}). Click one to show it; closing the session covers it again.",
                count(d.findings as u64, "finding"),
                d.secrets,
                if d.secrets == 1 { "" } else { "s" }
            );
            if open > 0 {
                n = format!("{open} shown. {n}");
            }
            n
        };
        about = about.child(p.note(note).key("findings-note"));
    }
    let back = Button::new("Back")
        .variant(ButtonVariant::Ghost)
        .on_press(super::close)
        .build(&p.k)
        .key("back");
    row()
        .key("session-header")
        .w_full()
        .px(28.0)
        .pt(18.0)
        .pb(16.0)
        .gap(14.0)
        .child(col().pt(2.0).child(back))
        .child(about)
}

fn events(o: &Open, doc: &SessionDoc, p: &Paint) -> El {
    let p2 = p.clone();
    let list = col()
        .key("events")
        .w_full()
        .pb(28.0)
        .growth(GrowthSpec::default())
        .selection_group(true)
        .virtual_rows(
            doc.blocks.len(),
            60.0,
            |_: &State, i| Cow::Owned(format!("block-{i}")),
            move |s: &State, i| match s.sessions.open.as_ref() {
                Some(o) => block(o, i, &p2),
                None => col(),
            },
        )
        .virtual_rows_version(&o.key);
    col()
        .key("session")
        .w_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(list)
}

/// How an event kind looks: its heading, the colour of its mark, and its text's type.
struct Look {
    words: &'static str,
    shown: &'static str,
    mark: Color,
    mono: bool,
    quiet: bool,
}

fn look(p: &Paint, k: EventKind, error: bool) -> Look {
    match k {
        EventKind::System => Look {
            words: "system",
            shown: "System",
            mark: p.off(),
            mono: false,
            quiet: true,
        },
        EventKind::User => Look {
            words: "you",
            shown: "You",
            mark: p.k.c.info.icon,
            mono: false,
            quiet: false,
        },
        EventKind::Assistant => Look {
            words: "assistant",
            shown: "Assistant",
            mark: p.good(),
            mono: false,
            quiet: false,
        },
        EventKind::Reasoning => Look {
            words: "reasoning",
            shown: "Reasoning",
            mark: p.faint(),
            mono: false,
            quiet: true,
        },
        EventKind::ToolCall => Look {
            words: "tool call",
            shown: "Tool call",
            mark: p.warn(),
            mono: true,
            quiet: false,
        },
        EventKind::ToolResult => Look {
            words: "tool result",
            shown: "Tool result",
            mark: if error { p.bad() } else { p.faint() },
            mono: true,
            quiet: false,
        },
    }
}

fn block(o: &Open, i: usize, p: &Paint) -> El {
    let Some(doc) = o.doc() else { return col() };
    let Some(b) = doc.blocks.get(i) else {
        return col();
    };
    let e = &doc.events[b.event];
    let l = look(p, e.kind, e.is_error);
    let mut c = col()
        .w_full()
        .px(28.0)
        .pt(if b.first { 16.0 } else { 0.0 })
        .gap(6.0);
    if b.first {
        let mut label = format!("Event {}, {}", e.seq, l.words);
        let mut shown = l.shown.to_string();
        if let Some(t) = &e.tool {
            label.push_str(&format!(", {t}"));
            shown.push_str(&format!(" · {t}"));
        }
        if e.is_error {
            label.push_str(", error");
            shown.push_str(" · error");
        }
        if let Some(m) = e
            .model
            .as_deref()
            .filter(|_| e.kind == EventKind::Assistant)
        {
            shown.push_str(&format!(" · {m}"));
        }
        let mut head = row()
            .key("event-head")
            .role("heading")
            .label(label)
            .gap(8.0)
            .center_y()
            .child(dot(l.mark, 8.0))
            .child(
                text(shown)
                    .font(p.k.ty.ui_medium)
                    .color(if l.quiet { p.muted() } else { p.ink() })
                    .a11y_hidden(),
            )
            .child(
                text(format!("#{}", e.seq))
                    .font(p.caption())
                    .color(p.faint())
                    .a11y_hidden(),
            );
        if let Some(ts) = &e.ts {
            head = head.child(
                text(time_of(ts))
                    .font(p.caption())
                    .color(p.faint())
                    .a11y_hidden(),
            );
        }
        c = c.child(head);
    }
    let src = &e.text[b.start..b.end];
    let ms: Vec<cover::Mark> = e
        .marks
        .iter()
        .filter(|m| m.start >= b.start && m.end <= b.end)
        .map(|m| cover::Mark {
            start: m.start - b.start,
            end: m.end - b.start,
            ..m.clone()
        })
        .collect();
    let body = if src.is_empty() && b.first {
        text("(no text)").font(p.caption()).color(p.faint())
    } else {
        let font = if l.mono { p.mono() } else { p.text().lh(20.0) };
        cover::text(p, src, &ms, shown(o), true)
            .key(format!("text-{i}"))
            .font(font)
            .color(if l.quiet { p.muted() } else { p.ink() })
            .selectable(true)
            .wrap()
            .w_full()
            .on_link(on_chip)
    };
    // Tool calls and results sit on a quiet fill; messages and reasoning on the page.
    let body = if l.mono {
        col()
            .w_full()
            .px(12.0)
            .py(if b.first { 8.0 } else { 0.0 })
            .fill(p.t.hover)
            .child(body)
    } else {
        col().w_full().pl(16.0).child(body)
    };
    c.child(body)
}

/// `10:00:05` from an RFC 3339 time, in UTC.
fn time_of(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|t| t.with_timezone(&chrono::Utc).format("%H:%M:%S").to_string())
        .unwrap_or_else(|_| ts.to_string())
}
