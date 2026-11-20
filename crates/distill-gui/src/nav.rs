//! The window's navigation: the screens, in order, and the sidebar that moves between them.
//!
//! Each screen is a module under `screens/` with an `open` (what to load when the owner moves to
//! it) and a `view`, and owns its state struct (a field of [`State`]) when it has one.

use rune::{El, col, el, row, text};

use crate::State;
use crate::screens::{approvals, findings, guide, overview, read_log, sessions, setup};
use crate::ui::Paint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Setup,
    Overview,
    Sessions,
    Findings,
    Approvals,
    ReadLog,
    Guide,
}

/// A screen's entry in the navigation.
pub struct Entry {
    pub screen: Screen,
    /// For keys: `nav-<id>`, `screen-<id>`.
    pub id: &'static str,
    pub label: &'static str,
    /// Called when the owner moves to the screen: queue what it shows.
    pub open: fn(&mut State),
    /// The screen's body. Overlays (menus, popovers) go in `layer`.
    pub view: fn(&State, &Paint, &mut Vec<El>) -> El,
}

pub const SCREENS: [Entry; 7] = [
    Entry {
        screen: Screen::Setup,
        id: "setup",
        label: "Setup",
        open: setup::open,
        view: setup::view,
    },
    Entry {
        screen: Screen::Overview,
        id: "overview",
        label: "Overview",
        open: overview::open,
        view: overview::view,
    },
    Entry {
        screen: Screen::Sessions,
        id: "sessions",
        label: "Sessions",
        open: sessions::open,
        view: sessions::view,
    },
    Entry {
        screen: Screen::Findings,
        id: "findings",
        label: "Findings",
        open: findings::open,
        view: findings::view,
    },
    Entry {
        screen: Screen::Approvals,
        id: "approvals",
        label: "Approvals",
        open: approvals::open,
        view: approvals::view,
    },
    Entry {
        screen: Screen::ReadLog,
        id: "read-log",
        label: "Read log",
        open: read_log::open,
        view: read_log::view,
    },
    Entry {
        screen: Screen::Guide,
        id: "guide",
        label: "Guide",
        open: guide::open,
        view: guide::view,
    },
];

pub fn entry(screen: Screen) -> &'static Entry {
    SCREENS
        .iter()
        .find(|e| e.screen == screen)
        .expect("every screen has an entry")
}

/// Move to `screen` and queue what it shows.
pub fn go(s: &mut State, screen: Screen) {
    s.screen = screen;
    (entry(screen).open)(s);
}

pub fn sidebar(s: &State, p: &Paint) -> El {
    let mut list = col().key("nav").role("list").label("Screens").gap(2.0);
    for e in &SCREENS {
        let current = e.screen == s.screen;
        let waiting = match e.screen {
            Screen::Approvals => s.pending_reveals(),
            _ => 0,
        };
        let label = if waiting > 0 {
            format!("{}, {waiting} waiting", e.label)
        } else {
            e.label.to_string()
        };
        let mut item = row()
            .key(format!("nav-{}", e.id))
            .role("button")
            .label(label)
            .checked(current)
            .h(32.0)
            .pl(4.0)
            .pr(10.0)
            .gap(8.0)
            .center_y()
            .radius(7.0)
            .fill(if current {
                p.brand_soft()
            } else {
                rune::Color::TRANSPARENT
            })
            .focusable(true)
            .on_press(move |s: &mut State| go(s, e.screen))
            // The current screen's marker: a short green bar at the left edge.
            .child(
                el().key("marker")
                    .w(3.0)
                    .h(16.0)
                    .radius(1.5)
                    .fill(if current {
                        p.brand
                    } else {
                        rune::Color::TRANSPARENT
                    })
                    .a11y_hidden(),
            )
            .child(
                text(e.label)
                    .font(if current { p.k.ty.ui_medium } else { p.text() })
                    .color(if current { p.ink() } else { p.muted() })
                    .grow(1.0)
                    .a11y_hidden(),
            );
        if waiting > 0 {
            item = item.child(p.count_badge(waiting).a11y_hidden());
        }
        list = list.child(p.k.interactive(item));
    }
    col()
        .key("sidebar")
        .w(200.0)
        .h_full()
        .px(12.0)
        // The window's own buttons sit over the top-left corner on macOS.
        .pt(if cfg!(target_os = "macos") {
            44.0
        } else {
            20.0
        })
        .gap(24.0)
        .fill(p.t.card)
        .gradient(p.t.card, p.t.card.mix(p.brand, 0.025))
        .child(
            row()
                .key("brand")
                .px(6.0)
                .gap(10.0)
                .center_y()
                .child(p.logo(34.0))
                .child(
                    col()
                        .gap(1.0)
                        .child(
                            text("DISTILL")
                                .key("title")
                                .font(p.wordmark(18.0))
                                .color(p.ink()),
                        )
                        .child(
                            text("Your session archive")
                                .font(p.caption())
                                .color(p.muted()),
                        ),
                ),
        )
        .child(list)
}
