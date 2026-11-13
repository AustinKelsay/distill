//! The read-only dashboard: totals, sessions per week, source health, what the steward did and
//! what needs a person. The only writes are approvals, sent through `State::approve`.

use chrono::Utc;
use distill_core::runlog::RunEntry;
use distill_core::stats::{Dashboard, Health, NeedsYou, SourceHealth};
use rune::{Align, El, ScrollSpec, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use crate::data::{ago, dollars, human_bytes, thousands};
use crate::ui::{Paint, dot, pill};
use crate::{State, chart};

/// How many steward runs the dashboard lists.
pub const RECENT_RUNS: usize = 6;

pub fn view(s: &State, p: &Paint) -> El {
    let body = match (&s.dash.data, &s.dash.error) {
        (Some(d), _) => content(s, d, p),
        (None, Some(e)) => message(p, "Could not read the archive", e, true),
        (None, None) => message(p, "Reading the archive…", "", false),
    };
    col()
        .key("dashboard")
        .w_full()
        .h_full()
        .child(header(s, p))
        .child(body)
}

fn header(s: &State, p: &Paint) -> El {
    let now = Utc::now();
    let last = s.dash.data.as_ref().and_then(|d| d.last_sweep.as_deref());
    let (color, status) = match last {
        Some(at) => (p.k.c.success.icon, format!("Last sweep {}", ago(at, now))),
        None if s.dash.data.is_some() => (p.k.c.warning.icon, "No sweep yet".to_string()),
        None => (p.t.control_off, "Loading…".to_string()),
    };
    let mut right = row().gap(12.0).center_y();
    if let Some(n) = &s.dash.notice {
        right = right.child(
            text(n.clone())
                .key("notice")
                .font(p.caption())
                .color(p.muted())
                .truncate()
                .max_w(320.0),
        );
    }
    right = right.child(
        Button::new("Settings")
            .on_press(|s: &mut State| s.open_settings())
            .build(&p.k)
            .key("settings"),
    );
    let bar = row()
        .key("header")
        .w_full()
        .h(68.0)
        .px(28.0)
        .center_y()
        .justify_between()
        .child(
            col()
                .gap(2.0)
                .child(text("Distill").font(p.title()).color(p.ink()))
                .child(
                    row()
                        .key("status")
                        .role("status")
                        .label(status.clone())
                        .gap(6.0)
                        .center_y()
                        .child(dot(color, 7.0))
                        .child(
                            text(status)
                                .key("last-sweep")
                                .font(p.caption())
                                .color(p.muted())
                                .a11y_hidden(),
                        ),
                ),
        )
        .child(right);
    col().w_full().child(bar).child(p.separator())
}

fn message(p: &Paint, title: &str, detail: &str, setup: bool) -> El {
    let mut c = col()
        .key("message")
        .grow(1.0)
        .w_full()
        .center()
        .gap(8.0)
        .child(text(title).font(p.heading()).color(p.ink()));
    if !detail.is_empty() {
        c = c.child(
            text(detail)
                .font(p.caption())
                .color(p.muted())
                .wrap()
                .max_w(520.0)
                .align(Align::Center),
        );
    }
    if setup {
        c = c.child(
            Button::new("Open setup")
                .on_press(|s: &mut State| s.open_settings())
                .build(&p.k)
                .key("open-setup"),
        );
    }
    c
}

fn content(s: &State, d: &Dashboard, p: &Paint) -> El {
    col()
        .key("body")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .px(28.0)
        .pt(20.0)
        .pb(28.0)
        .gap(16.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(tiles(d, p))
        .child(
            row()
                .key("row-1")
                .w_full()
                .gap(16.0)
                .items_stretch()
                .child(chart::view(p, &d.weeks).grow(1.6))
                .child(sources(d, p).grow(1.0)),
        )
        .child(
            row()
                .key("row-2")
                .w_full()
                .gap(16.0)
                .items_stretch()
                .child(runs(&d.recent_runs, p).grow(1.0))
                .child(needs_you(s, &d.needs_you, p).grow(1.0)),
        )
}

fn tiles(d: &Dashboard, p: &Paint) -> El {
    let t = &d.totals;
    let this_week = d.weeks.last().map_or(0, |w| w.total);
    let waiting = d
        .needs_you
        .iter()
        .filter(|n| matches!(n, NeedsYou::ApproveExport { .. }))
        .count();
    let enriched = (t.enriched.clamp(0.0, 1.0) * 100.0).round();
    let meter = el()
        .key("meter")
        .w_full()
        .h(4.0)
        .radius(2.0)
        .fill(p.t.control_track)
        .child(
            el().w_frac(t.enriched.clamp(0.0, 1.0) as f32)
                .h(4.0)
                .radius(2.0)
                .fill(rune::tok::accent(0, p.dark)),
        );
    let cap = |s: &str| text(s.to_string()).font(p.caption()).color(p.muted());
    row()
        .key("tiles")
        .w_full()
        .gap(12.0)
        .child(tile(
            p,
            "tile-sessions",
            "Sessions",
            thousands(t.sessions),
            cap(&format!("{} this week", thousands(this_week))),
        ))
        .child(tile(
            p,
            "tile-archive",
            "Archive size",
            human_bytes(t.archive_bytes),
            cap("raw captures, deduplicated"),
        ))
        .child(tile(
            p,
            "tile-enriched",
            "Enriched",
            format!("{enriched:.0}%"),
            col()
                .w_full()
                .gap(6.0)
                .pt(2.0)
                .child(meter)
                .child(cap(&format!("{} spent today", dollars(t.spent_today_usd)))),
        ))
        .child(tile(
            p,
            "tile-datasets",
            "Datasets",
            thousands(t.datasets),
            cap(&if waiting == 0 {
                "builds on disk".to_string()
            } else {
                format!("{waiting} waiting for approval")
            }),
        ))
}

fn tile(p: &Paint, key: &str, label: &str, value: String, sub: El) -> El {
    p.card()
        .key(key)
        .role("group")
        .label(format!("{label}: {value}"))
        .grow(1.0)
        .gap(4.0)
        .child(
            text(label)
                .font(p.k.ty.small_medium.lh(16.0))
                .color(p.muted())
                .a11y_hidden(),
        )
        .child(
            text(value)
                .key("value")
                .font(p.stat())
                .color(p.ink())
                .a11y_hidden(),
        )
        .child(sub)
}

fn sources(d: &Dashboard, p: &Paint) -> El {
    let ok = d.sources.iter().filter(|s| s.health == Health::Ok).count();
    let on = d
        .sources
        .iter()
        .filter(|s| s.health != Health::Disabled)
        .count();
    let trailing = text(format!("{ok} of {on} healthy"))
        .font(p.caption())
        .color(p.muted());
    let mut card = p.section("sources", "Sources", Some(trailing)).gap(8.0);
    let mut list = col().key("list").role("list").label("Sources");
    for (i, src) in d.sources.iter().enumerate() {
        if i > 0 {
            list = list.child(p.separator());
        }
        list = list.child(source_row(p, src));
    }
    card = card.child(list);
    card
}

fn source_row(p: &Paint, s: &SourceHealth) -> El {
    let off = s.health == Health::Disabled;
    let note = match (&s.note, s.health) {
        (Some(n), _) => n.clone(),
        (None, Health::Disabled) => "off".to_string(),
        (None, _) => format!("{} sessions", thousands(s.sessions)),
    };
    let size = if s.archive_bytes > 0 {
        human_bytes(s.archive_bytes)
    } else {
        "—".to_string()
    };
    row()
        .key(format!("source-{}", s.kind.id()))
        .role("listitem")
        .label(format!("{}: {note}", s.label))
        .w_full()
        .h(44.0)
        .gap(10.0)
        .center_y()
        .child(dot(p.health_color(s.health), 8.0))
        .child(
            col()
                .grow(1.0)
                .shrink()
                .gap(1.0)
                .a11y_hidden()
                .child(text(s.label.clone()).font(p.k.ty.ui_medium).color(if off {
                    p.muted()
                } else {
                    p.ink()
                }))
                .child(
                    text(note)
                        .font(p.caption())
                        .color(if s.health == Health::Warn {
                            p.k.c.warning.fg
                        } else {
                            p.faint()
                        })
                        .truncate(),
                ),
        )
        .child(
            text(size)
                .font(p.k.ty.ui.features("'tnum' 1"))
                .color(if off { p.faint() } else { p.muted() })
                .a11y_hidden(),
        )
}

fn runs(entries: &[RunEntry], p: &Paint) -> El {
    let card = p.section("runs", "What the steward did", None).gap(8.0);
    if entries.is_empty() {
        return card.child(p.empty(
            "runs-empty",
            "Nothing has run yet. The first sweep and steward run will show here.",
        ));
    }
    let now = Utc::now();
    let mut list = col().key("list").role("list").label("Recent runs");
    for (i, r) in entries.iter().take(RECENT_RUNS).enumerate() {
        if i > 0 {
            list = list.child(p.separator());
        }
        let mut meta = row()
            .gap(8.0)
            .center_y()
            .child(text(ago(&r.at, now)).font(p.caption()).color(p.faint()));
        if let Some(c) = r.cost_usd {
            meta = meta.child(
                text(dollars(c))
                    .font(p.caption().features("'tnum' 1"))
                    .color(p.faint()),
            );
        }
        list = list.child(
            row()
                .key(format!("run-{i}"))
                .role("listitem")
                .label(format!("{}: {}", r.step, r.summary))
                .w_full()
                .h(44.0)
                .gap(10.0)
                .center_y()
                .child(dot(
                    if r.ok {
                        p.k.c.success.icon
                    } else {
                        p.k.c.danger.icon
                    },
                    7.0,
                ))
                .child(
                    col()
                        .grow(1.0)
                        .shrink()
                        .gap(1.0)
                        .a11y_hidden()
                        .child(
                            text(r.summary.clone())
                                .font(p.text())
                                .color(p.ink())
                                .truncate(),
                        )
                        .child(text(r.step.clone()).font(p.caption()).color(p.muted())),
                )
                .child(meta.a11y_hidden()),
        );
    }
    card.child(list)
}

fn needs_you(s: &State, items: &[NeedsYou], p: &Paint) -> El {
    let count = if items.is_empty() {
        None
    } else {
        Some(pill(p, &items.len().to_string(), p.k.c.warning).key("needs-count"))
    };
    let mut card = p.section("needs-you", "Needs you", count).gap(8.0);
    if items.is_empty() {
        return card.child(
            col()
                .key("needs-empty")
                .w_full()
                .py(18.0)
                .gap(8.0)
                .center()
                .child(dot(p.k.c.success.icon, 10.0))
                .child(
                    text("Nothing needs you. The steward has it.")
                        .font(p.text())
                        .color(p.faint()),
                ),
        );
    }
    let mut list = col().key("list").role("list").label("Needs you");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            list = list.child(p.separator());
        }
        list = list.child(need_row(s, i, item, p));
    }
    card = card.child(list);
    card
}

fn need_row(s: &State, i: usize, item: &NeedsYou, p: &Paint) -> El {
    let (detail, action, tone): (String, Option<El>, _) = match item {
        NeedsYou::ApproveExport {
            recipe,
            build,
            records,
            redactions,
        } => {
            let busy = s.dash.approving.contains(&(recipe.clone(), build.clone()));
            let detail = format!(
                "build {build} · {} records · {} redactions",
                thousands(*records),
                thousands(*redactions)
            );
            let (recipe, build) = (recipe.clone(), build.clone());
            let button = Button::new(if busy { "Approving…" } else { "Approve" })
                .variant(ButtonVariant::Submit)
                .control_size(ControlSize::Regular)
                .disabled(busy)
                .on_press(move |s: &mut State| s.approve(recipe.clone(), build.clone()))
                .build(&p.k)
                .key(format!("approve-{i}"));
            (detail, Some(button), p.k.c.info.icon)
        }
        NeedsYou::NoParser { .. } => (
            "Archived as raw bytes; it is read once a parser exists.".to_string(),
            None,
            p.k.c.warning.icon,
        ),
        NeedsYou::ParserFailures { .. } => (
            "The steward can repair the parser, or run `distill steward repair`.".to_string(),
            None,
            p.k.c.warning.icon,
        ),
        NeedsYou::StewardFailed { at, .. } => (
            format!("at {}", ago(at, Utc::now())),
            None,
            p.k.c.danger.icon,
        ),
        NeedsYou::NotConfigured => {
            let b = Button::new("Open setup")
                .on_press(|s: &mut State| s.open_settings())
                .build(&p.k)
                .key("setup-now");
            (
                "Choose sources and a schedule to start archiving.".to_string(),
                Some(b),
                p.k.c.warning.icon,
            )
        }
    };
    let mut r = row()
        .key(format!("need-{i}"))
        .role("listitem")
        .label(item.title())
        .w_full()
        .py(10.0)
        .gap(10.0)
        .center_y()
        .child(dot(tone, 7.0))
        .child(
            col()
                .grow(1.0)
                .shrink()
                .gap(2.0)
                .child(
                    text(item.title())
                        .key("title")
                        .font(p.text())
                        .color(p.ink())
                        .wrap(),
                )
                .child(text(detail).font(p.caption()).color(p.muted()).wrap()),
        );
    if let Some(a) = action {
        r = r.child(a);
    }
    r
}
