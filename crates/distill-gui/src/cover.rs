//! Covering findings: every secret and personal detail is drawn as a chip naming its kind
//! (`‹email›`) until the owner clicks it (ADR 0007; the workaround from #84). The value is
//! replaced before the text is built, so while covered it is in no element, no `--tree` dump,
//! no accessibility name and no copy. A real blur can't target a range of text on the pinned
//! Rune rev, and would leave the value in the tree anyway.
//!
//! A screen places its findings as [`Mark`]s on the string they are in, then builds the text with
//! [`text`] and handles `on_link` itself: each chip, when clickable, is a link whose id is its
//! mark's id. Sessions (#86) uses it for events, cards, openings and passages; anything else that
//! shows session text should too.

use distill_core::detect::Kind;
use rune::{Color, El, Link, LinkPaint, Span, rich};

use crate::ui::Paint;

/// A finding placed on one string: its byte range there, its kind, and the id a click reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
    pub id: String,
}

/// What sits on each side of a chip or a shown value, outside its fill and never copied: Rune
/// draws a span's fill past its text by its padding, which would cover the spaces beside it.
/// A narrow no-break space is about as wide as that padding.
pub const PAD: &str = "\u{202f}";

/// What a chip shows in place of a value: `‹openai key›`.
pub fn chip(kind: Kind) -> String {
    format!("‹{}›", kind.name().replace('_', " "))
}

/// `marks` in order, overlapping ones merged into the first. When any mark does not fit `src`
/// (past its end, empty, or not on character boundaries), the whole string becomes that one
/// mark, so no part of a value can show because a range was wrong.
pub fn fit(src: &str, marks: &[Mark]) -> Vec<Mark> {
    let bad = marks.iter().find(|m| {
        m.start >= m.end
            || m.end > src.len()
            || !src.is_char_boundary(m.start)
            || !src.is_char_boundary(m.end)
    });
    if let Some(m) = bad {
        return vec![Mark {
            start: 0,
            end: src.len(),
            ..m.clone()
        }];
    }
    let mut sorted = marks.to_vec();
    sorted.sort_by_key(|m| (m.start, m.end));
    let mut out: Vec<Mark> = Vec::with_capacity(sorted.len());
    for m in sorted {
        match out.last_mut() {
            Some(last) if m.start < last.end => last.end = last.end.max(m.end),
            _ => out.push(m),
        }
    }
    out
}

/// `src` as it is drawn: each mark that `shown` doesn't name replaced by its chip, and each
/// mark between two [`PAD`]s, which copying leaves out.
pub struct Covered {
    pub text: String,
    pub spans: Vec<Span>,
    pub links: Vec<Link>,
    /// The pads' byte ranges of `text`.
    pub uncopied: Vec<std::ops::Range<usize>>,
}

/// Build what [`text`] draws. Clickable chips are links that report their mark's id.
pub fn cover(
    p: &Paint,
    src: &str,
    marks: &[Mark],
    shown: impl Fn(&str) -> bool,
    clickable: bool,
) -> Covered {
    let marks = fit(src, marks);
    let (cover_bg, cover_ink) = chip_colors(p);
    let shown_bg = rune::oklch(0.75, 0.12, 80.0).alpha(if p.k.dark { 0.30 } else { 0.35 });
    let mut out = String::with_capacity(src.len());
    let mut spans = Vec::new();
    let mut links = Vec::new();
    let mut uncopied = Vec::new();
    let mut pad = |out: &mut String| {
        uncopied.push(out.len()..out.len() + PAD.len());
        out.push_str(PAD);
    };
    let mut at = 0;
    for m in &marks {
        out.push_str(&src[at..m.start]);
        pad(&mut out);
        let start = out.len();
        let hidden = !shown(&m.id);
        let name = m.kind.name().replace('_', " ");
        if hidden {
            out.push_str(&chip(m.kind));
            spans.push(
                Span::new(start, out.len())
                    .bg(cover_bg, 3.0)
                    .color(cover_ink)
                    .mono()
                    .size(11.5),
            );
        } else {
            out.push_str(&src[m.start..m.end]);
            spans.push(Span::new(start, out.len()).bg(shown_bg, 2.0));
        }
        if clickable {
            let label = if hidden {
                format!("Covered {name}, click to show")
            } else {
                format!("Shown {name}, click to cover")
            };
            links.push(
                Link::new(start..out.len(), m.id.clone())
                    .role("button")
                    .label(label)
                    .hover(LinkPaint::new().ring(p.k.c.info.icon, 1.0)),
            );
        }
        pad(&mut out);
        at = m.end;
    }
    out.push_str(&src[at..]);
    Covered {
        text: out,
        spans,
        links,
        uncopied,
    }
}

/// `src` as a rich text with its marks covered. Clickable chips report their ids to the text's
/// `on_link`, which the caller adds.
pub fn text(
    p: &Paint,
    src: &str,
    marks: &[Mark],
    shown: impl Fn(&str) -> bool,
    clickable: bool,
) -> El {
    let c = cover(p, src, marks, shown, clickable);
    let mut t = rich(c.text, c.spans);
    for l in c.links {
        t = t.link(l);
    }
    for r in c.uncopied {
        t = t.uncopied(r);
    }
    t
}

/// A chip's fill and ink.
fn chip_colors(p: &Paint) -> (Color, Color) {
    if p.k.dark {
        (rune::oklch(0.38, 0.02, 265.0), Color::WHITE.alpha(0.66))
    } else {
        (rune::oklch(0.86, 0.012, 265.0), Color::BLACK.alpha(0.60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(start: usize, end: usize, id: &str) -> Mark {
        Mark {
            start,
            end,
            kind: Kind::Email,
            id: id.into(),
        }
    }

    #[test]
    fn a_covered_mark_is_replaced_by_its_chip_and_a_shown_one_keeps_its_value() {
        let p = Paint::new(true, 1.0);
        let src = "mail ab@example.com or cd@example.com";
        let marks = [mark(5, 19, "a"), mark(23, 37, "b")];
        let c = cover(&p, src, &marks, |_| false, true);
        assert_eq!(c.text.replace(PAD, ""), "mail ‹email› or ‹email›");
        assert_eq!(c.links.len(), 2);
        let c = cover(&p, src, &marks, |id| id == "b", false);
        assert_eq!(c.text.replace(PAD, ""), "mail ‹email› or cd@example.com");
        assert!(c.links.is_empty());
    }

    #[test]
    fn a_chip_keeps_the_spaces_beside_it_clear_of_its_fill() {
        let p = Paint::new(true, 1.0);
        let src = "the token is ab@example.com and";
        let c = cover(&p, src, &[mark(13, 27, "a")], |_| false, true);
        // A fill is drawn past its text by its padding, so a narrow, uncopied pad on each side
        // takes that room and the spaces beside the chip stay as wide as written.
        let chip = "‹email›";
        assert_eq!(c.text, format!("the token is {PAD}{chip}{PAD} and"));
        let at = 13 + PAD.len();
        assert_eq!(
            c.uncopied,
            [13..at, at + chip.len()..at + chip.len() + PAD.len()]
        );
        assert_eq!(
            (c.spans[0].start, c.spans[0].end),
            (at as u32, (at + chip.len()) as u32)
        );
    }

    #[test]
    fn overlapping_marks_merge_and_a_mark_that_does_not_fit_covers_the_whole_string() {
        let src = "abcdefgh";
        assert_eq!(
            fit(src, &[mark(4, 6, "b"), mark(1, 5, "a")]),
            [mark(1, 6, "a")]
        );
        assert_eq!(
            fit(src, &[mark(1, 2, "a"), mark(6, 40, "b")]),
            [mark(0, 8, "b")]
        );
        assert_eq!(fit("é", &[mark(1, 2, "a")]), [mark(0, 2, "a")]);
    }
}
