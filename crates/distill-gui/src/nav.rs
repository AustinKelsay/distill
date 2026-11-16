//! The window's navigation: the spec's six screens, in order, and the sidebar that moves
//! between them.
//!
//! Each screen is a module under `screens/` with an `open` (what to load when the owner moves to
//! it) and a `view`, and owns its state struct (a field of [`State`]). Building a screen means
//! filling in its module; this table already lists all six.

use rune::{El, col, row, text};

use crate::State;
use crate::screens::{approvals, findings, overview, read_log, sessions, setup};
use crate::ui::Paint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Setup,
    Overview,
    Sessions,
    Findings,
    Approvals,
    ReadLog,
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

pub const SCREENS: [Entry; 6] = [
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
            .px(10.0)
            .gap(8.0)
            .center_y()
            .justify_between()
            .radius(7.0)
            .fill(if current {
                p.t.hover
            } else {
                rune::Color::TRANSPARENT
            })
            .focusable(true)
            .on_press(move |s: &mut State| go(s, e.screen))
            .child(
                text(e.label)
                    .font(if current { p.k.ty.ui_medium } else { p.text() })
                    .color(if current { p.ink() } else { p.muted() })
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
        .gap(20.0)
        .fill(p.t.card)
        .child(
            col()
                .px(10.0)
                .gap(2.0)
                .child(text("Distill").key("title").font(p.title()).color(p.ink()))
                .child(
                    text("Your session archive")
                        .font(p.caption())
                        .color(p.muted()),
                ),
        )
        .child(list)
}
