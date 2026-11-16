//! The body of a screen a later ticket builds.

use rune::{Align, El, col, text};

use crate::ui::Paint;

pub fn view(p: &Paint, title: &str, what: &str) -> El {
    col()
        .w_full()
        .h_full()
        .center()
        .gap(8.0)
        .child(text(title).key("heading").font(p.title()).color(p.ink()))
        .child(
            text("Not yet available")
                .key("not-yet")
                .font(p.heading())
                .color(p.muted()),
        )
        .child(
            text(what)
                .key("what")
                .font(p.text())
                .color(p.muted())
                .wrap()
                .max_w(460.0)
                .align(Align::Center),
        )
}
