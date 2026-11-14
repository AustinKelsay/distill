//! Shared look: the kit and core theme for the current appearance, cards, dots and source colours.

use distill_core::SourceKind;
use distill_core::stats::Health;
use rune::tok::{self, Theme};
use rune::{Color, El, TextStyle, col, el, row, text};
use rune_kit::Kit;

/// Everything a view function needs to paint itself.
#[derive(Clone)]
pub struct Paint {
    pub k: Kit,
    pub t: Theme,
    pub dark: bool,
    pub hair: f32,
}

impl Paint {
    pub fn new(dark: bool, scale: f32) -> Paint {
        Paint {
            k: Kit::neutral(dark),
            t: tok::theme(dark),
            dark,
            hair: 1.0 / scale.max(1.0),
        }
    }

    pub fn title(&self) -> TextStyle {
        TextStyle::new(20.0, 650.0).lh(26.0)
    }
    pub fn heading(&self) -> TextStyle {
        TextStyle::new(13.0, 600.0).lh(18.0)
    }
    pub fn text(&self) -> TextStyle {
        self.k.ty.ui
    }
    pub fn caption(&self) -> TextStyle {
        self.k.ty.caption()
    }
    pub fn mono(&self) -> TextStyle {
        TextStyle::new(12.0, 450.0).mono().lh(18.0)
    }
    pub fn stat(&self) -> TextStyle {
        TextStyle::new(26.0, 650.0).lh(32.0).features("'tnum' 1")
    }

    pub fn ink(&self) -> Color {
        self.t.text
    }
    pub fn muted(&self) -> Color {
        self.t.text_secondary
    }
    pub fn faint(&self) -> Color {
        self.k.c.text_faint
    }

    /// One colour per source, in `SourceKind::ALL` order.
    pub fn source_color(&self, kind: SourceKind) -> Color {
        let i = SourceKind::ALL.iter().position(|&k| k == kind).unwrap_or(0);
        // Rune has six accents and repeats them; sources past the sixth get their own hues.
        const EXTRA_HUES: [f32; 4] = [195.0, 25.0, 110.0, 330.0];
        match i.checked_sub(6) {
            None => tok::accent(i, self.dark),
            Some(j) => {
                let l = if self.dark { 0.68 } else { 0.6 };
                rune::oklch(l, 0.15, EXTRA_HUES[j % EXTRA_HUES.len()])
            }
        }
    }

    pub fn health_color(&self, h: Health) -> Color {
        match h {
            Health::Ok => self.k.c.success.icon,
            Health::Warn => self.k.c.warning.icon,
            Health::Missing => self.k.c.danger.icon,
            Health::Disabled => self.t.control_off,
        }
    }

    /// A raised surface holding one section.
    pub fn card(&self) -> El {
        col()
            .padding(16.0)
            .gap(12.0)
            .fill(self.t.card)
            .radius(12.0)
            .border(self.hair, self.t.card_border)
            .shadow(1.0, 2.0, self.t.shadow_soft)
    }

    /// A card with a heading row (title on the left, `trailing` on the right).
    pub fn section(&self, key: &str, title: &str, trailing: Option<El>) -> El {
        let mut head = row().key("head").center_y().justify_between().child(
            text(title)
                .key("title")
                .font(self.heading())
                .color(self.ink()),
        );
        if let Some(t) = trailing {
            head = head.child(t);
        }
        self.card().key(key).role("group").label(title).child(head)
    }

    pub fn separator(&self) -> El {
        el().h(self.hair)
            .w_full()
            .fill(self.t.separator)
            .a11y_hidden()
    }

    /// A centred line of muted text for a list or chart with nothing in it.
    pub fn empty(&self, key: &str, message: &str) -> El {
        col().key(key).w_full().py(18.0).center().child(
            text(message)
                .key("message")
                .font(self.text())
                .color(self.faint())
                .wrap(),
        )
    }
}

/// A filled circle.
pub fn dot(color: Color, d: f32) -> El {
    el().size(d, d).radius(d / 2.0).fill(color).a11y_hidden()
}

/// A small rounded tag in a status role's colours.
pub fn pill(p: &Paint, label: &str, role: rune_kit::StateColors) -> El {
    row()
        .role("status")
        .label(label)
        .h(20.0)
        .px(8.0)
        .center()
        .radius(10.0)
        .fill(role.bg)
        .border(p.hair, role.border)
        .child(
            text(label)
                .font(p.k.ty.small_medium.lh(14.0))
                .color(role.fg)
                .a11y_hidden(),
        )
}
