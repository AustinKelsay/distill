//! "Sessions per week by source": a stacked bar chart drawn with plain boxes.

use distill_core::SourceKind;
use distill_core::stats::WeekBucket;
use rune::{El, col, el, row, text};

use crate::data::{short_date, thousands};
use crate::ui::{Paint, dot};

const PLOT_H: f32 = 196.0;
const AXIS_W: f32 = 30.0;

/// The axis top for a peak of `v`: a multiple of a 1, 2 or 5 × 10ⁿ step (about a sixth of the
/// peak), made even so the midline label is a whole number.
pub fn nice_max(v: u64) -> u64 {
    let v = v.max(1);
    let mut step = 1;
    'find: loop {
        for m in [1, 2, 5] {
            if m * step * 6 >= v {
                step *= m;
                break 'find;
            }
        }
        step *= 10;
    }
    let mut top = v.div_ceil(step) * step;
    if !top.is_multiple_of(2) {
        top += step;
    }
    top
}

pub fn view(p: &Paint, weeks: &[WeekBucket]) -> El {
    let total: u64 = weeks.iter().map(|w| w.total).sum();
    let trailing = text(format!("{} in {} weeks", thousands(total), weeks.len()))
        .font(p.caption())
        .color(p.muted());
    let card = p.section(
        "chart",
        "Sessions per week by source",
        Some(trailing.key("chart-total")),
    );
    if total == 0 {
        return card.child(
            col()
                .key("chart-empty")
                .w_full()
                .h(PLOT_H + 40.0)
                .center()
                .gap(4.0)
                .child(
                    text("No sessions in the last 12 weeks")
                        .font(p.k.ty.ui_medium)
                        .color(p.muted()),
                )
                .child(
                    text("The first sweep fills this in.")
                        .font(p.caption())
                        .color(p.faint()),
                ),
        );
    }
    card.child(legend(p, weeks)).child(plot(p, weeks))
}

fn legend(p: &Paint, weeks: &[WeekBucket]) -> El {
    let mut r = row()
        .key("legend")
        .gap(14.0)
        .flex_wrap()
        .role("list")
        .label("Sources");
    for kind in SourceKind::ALL {
        let n: u64 = weeks.iter().filter_map(|w| w.by_source.get(&kind)).sum();
        if n == 0 {
            continue;
        }
        r = r.child(
            row()
                .key(format!("legend-{}", kind.id()))
                .gap(6.0)
                .center_y()
                .label(format!("{}: {}", kind.label(), n))
                .child(dot(p.source_color(kind), 8.0))
                .child(
                    text(kind.label())
                        .font(p.caption())
                        .color(p.muted())
                        .a11y_hidden(),
                ),
        );
    }
    r
}

fn plot(p: &Paint, weeks: &[WeekBucket]) -> El {
    let peak = weeks.iter().map(|w| w.total).max().unwrap_or(0);
    let top = nice_max(peak);
    let scale = PLOT_H / top as f32;
    let grid = |y: f32, strong: bool| {
        el().absolute(0.0, y).w_full().h(p.hair).fill(if strong {
            p.t.separator
        } else {
            p.t.separator.alpha(0.5)
        })
    };
    let mut bars = row().key("bars").w_full().h(PLOT_H).gap(8.0);
    for (i, w) in weeks.iter().enumerate() {
        let mut stack = col()
            .key("stack")
            .w_frac(0.72)
            .max_w(30.0)
            .clip_rounded(3.0);
        // Top to bottom: the last source first, so `SourceKind::ALL[0]` sits on the baseline.
        for kind in SourceKind::ALL.iter().rev() {
            let n = w.by_source.get(kind).copied().unwrap_or(0);
            if n == 0 {
                continue;
            }
            stack = stack.child(
                el().key(kind.id())
                    .w_full()
                    .h(n as f32 * scale)
                    .fill(p.source_color(*kind)),
            );
        }
        bars = bars.child(
            col()
                .key(format!("week-{i}"))
                .role("img")
                .label(format!(
                    "Week of {}: {} sessions",
                    short_date(&w.week_start),
                    w.total
                ))
                .grow(1.0)
                .h_full()
                .justify_end()
                .items_center()
                .help(format!(
                    "{} · {} sessions",
                    short_date(&w.week_start),
                    w.total
                ))
                .child(stack.a11y_hidden()),
        );
    }
    let area = el()
        .key("plot")
        .grow(1.0)
        .h(PLOT_H)
        .child(grid(0.0, false))
        .child(grid(PLOT_H / 2.0, false))
        .child(grid(PLOT_H - p.hair, true))
        .child(bars);
    let axis = col()
        .key("axis")
        .w(AXIS_W)
        .h(PLOT_H)
        .justify_between()
        .a11y_hidden()
        .child(axis_label(p, top))
        .child(axis_label(p, top / 2))
        .child(axis_label(p, 0));
    let mut labels = row()
        .key("weeks")
        .w_full()
        .gap(8.0)
        .pl(AXIS_W + 8.0)
        .a11y_hidden();
    let n = weeks.len();
    for (i, w) in weeks.iter().enumerate() {
        // Every other week, ending on this one.
        let show = (n - 1 - i).is_multiple_of(2);
        labels = labels.child(
            col().grow(1.0).items_center().child(
                text(if show {
                    short_date(&w.week_start)
                } else {
                    String::new()
                })
                .font(p.k.ty.small.lh(14.0))
                .color(p.faint()),
            ),
        );
    }
    col()
        .key("plot-area")
        .w_full()
        .gap(6.0)
        .child(row().w_full().gap(8.0).child(axis).child(area))
        .child(labels)
}

fn axis_label(p: &Paint, v: u64) -> El {
    text(thousands(v))
        .font(p.k.ty.small.lh(12.0).features("'tnum' 1"))
        .color(p.faint())
        .align(rune::Align::End)
        .w_full()
}

#[cfg(test)]
mod tests {
    use super::nice_max;

    #[test]
    fn rounds_the_axis_up_to_a_nice_number() {
        assert_eq!(nice_max(0), 2);
        assert_eq!(nice_max(3), 4);
        assert_eq!(nice_max(7), 8);
        assert_eq!(nice_max(22), 30);
        assert_eq!(nice_max(42), 50);
        assert_eq!(nice_max(133), 150);
        for v in 1..2000 {
            let top = nice_max(v);
            assert!(
                top >= v && top.is_multiple_of(2) && top <= v * 2 + 2,
                "{v} → {top}"
            );
        }
    }
}
