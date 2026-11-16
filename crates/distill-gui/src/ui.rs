//! Shared look and formatting: the kit and core theme for the current appearance, cards,
//! dots, pills, and how counts, sizes and times are written.

use chrono::{DateTime, Utc};
use rune::tok::{self, Theme};
use rune::{Align, Color, El, TextStyle, col, el, row, text};
use rune_kit::Kit;

/// Everything a view function needs to paint itself.
#[derive(Clone)]
pub struct Paint {
    pub k: Kit,
    pub t: Theme,
    pub hair: f32,
}

impl Paint {
    pub fn new(dark: bool, scale: f32) -> Paint {
        Paint {
            k: Kit::neutral(dark),
            t: tok::theme(dark),
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
        TextStyle::new(24.0, 650.0).lh(30.0).features("'tnum' 1")
    }
    pub fn num(&self) -> TextStyle {
        self.k.ty.ui.features("'tnum' 1")
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
    pub fn good(&self) -> Color {
        self.k.c.success.icon
    }
    pub fn warn(&self) -> Color {
        self.k.c.warning.icon
    }
    pub fn bad(&self) -> Color {
        self.k.c.danger.icon
    }
    pub fn off(&self) -> Color {
        self.t.control_off
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
        let mut head = row()
            .key("head")
            .w_full()
            .center_y()
            .justify_between()
            .child(
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

    /// A centred line of muted text for a list or section with nothing in it.
    pub fn empty(&self, key: &str, message: &str) -> El {
        col().key(key).w_full().py(14.0).center().child(
            text(message)
                .key("message")
                .font(self.text())
                .color(self.faint())
                .wrap()
                .align(Align::Center),
        )
    }

    /// A small count in a filled circle, for the navigation.
    pub fn count_badge(&self, n: u64) -> El {
        row()
            .h(18.0)
            .min_w(18.0)
            .px(5.0)
            .center()
            .radius(9.0)
            .fill(self.k.c.warning.bg)
            .border(self.hair, self.k.c.warning.border)
            .child(
                text(n.to_string())
                    .font(self.k.ty.small_medium.lh(12.0))
                    .color(self.k.c.warning.fg),
            )
    }

    /// The muted explanation under a heading or setting.
    pub fn note(&self, s: impl Into<String>) -> El {
        text(s.into())
            .font(self.caption())
            .color(self.muted())
            .wrap()
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

// ---------------------------------------------------------------------------------------------
// Formatting

/// `1.4 GB`, `312 KB`, `0 B`.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1000.0 && i + 1 < UNITS.len() {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// `12,345`.
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `1 session`, `2,400 sessions`.
pub fn count(n: u64, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {one}s", thousands(n))
    }
}

/// "just now", "12 min ago", "3 h ago", "2 days ago", or the date for older times.
pub fn ago(rfc3339: &str, now: DateTime<Utc>) -> String {
    let Ok(t) = DateTime::parse_from_rfc3339(rfc3339) else {
        return rfc3339.to_string();
    };
    let secs = (now - t.with_timezone(&Utc)).num_seconds().max(0);
    match secs {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        86_400..604_800 => {
            let d = secs / 86_400;
            if d == 1 {
                "yesterday".to_string()
            } else {
                format!("{d} days ago")
            }
        }
        _ => t.format("%b %-d").to_string(),
    }
}

/// A path with the user's home folder written as `~`.
pub fn tilde(path: &std::path::Path) -> String {
    let s = path.to_string_lossy().into_owned();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes_counts_and_ages() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1_450_000_000), "1.4 GB");
        assert_eq!(human_bytes(312_000), "312 KB");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(count(1, "session"), "1 session");
        assert_eq!(count(2400, "session"), "2,400 sessions");
        let now = DateTime::parse_from_rfc3339("2026-11-13T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(ago("2026-11-13T11:48:00Z", now), "12 min ago");
        assert_eq!(ago("2026-11-13T09:00:00Z", now), "3 h ago");
        assert_eq!(ago("2026-11-12T09:00:00Z", now), "yesterday");
    }
}
