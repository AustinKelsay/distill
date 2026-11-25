//! Read log (#87): who has read what. Every line of the read log, newest first: reads, lists and
//! index builds, refusals and failures, and each step of a reveal request (asked, approved,
//! denied, collected, expired). Filtered by reader (as `distill reads --reader` takes it) and by
//! the day a line was written (`readlog::find`). The log holds no session text and no value.

use distill_core::readlog::{self, LogFilter, ReadEntry, ReadResult};
use rune::{El, ScrollSpec, TextEdit, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, TextInput};

use crate::State;
use crate::jobs::{self, Job};
use crate::ui::{Paint, ago, count, parse_day, pill};

/// How many lines a page shows; More adds as many again.
pub const PAGE: usize = 200;

#[derive(Default)]
pub struct ReadLogState {
    /// `owner`, `distill`, a provider, or `provider/model`; empty for everyone.
    pub reader: TextEdit,
    pub since: TextEdit,
    pub until: TextEdit,
    /// The lines that match, newest first; None until the first list lands.
    pub entries: Option<Vec<ReadEntry>>,
    /// How many lines were asked for, and whether there may be more.
    pub limit: usize,
    pub more: bool,
    pub loading: bool,
    pub error: Option<String>,
    /// Why the filters can't be used as written.
    pub filter_error: Option<String>,
}

pub fn open(s: &mut State) {
    let limit = s.read_log.limit.max(PAGE);
    ask(s, limit);
}

/// The log from its newest line with the current filters.
pub fn apply(s: &mut State) {
    ask(s, PAGE);
}

fn more(s: &mut State) {
    let limit = s.read_log.limit + PAGE;
    ask(s, limit);
}

fn ask(s: &mut State, limit: usize) {
    let st = &mut s.read_log;
    let filter = match (parse_day(&st.since, "from"), parse_day(&st.until, "to")) {
        (Ok(since), Ok(until)) => LogFilter {
            reader: Some(st.reader.text.trim().to_string()).filter(|r| !r.is_empty()),
            since,
            until,
        },
        (Err(e), _) | (_, Err(e)) => {
            st.filter_error = Some(e);
            return;
        }
    };
    st.filter_error = None;
    if st.loading {
        return;
    }
    st.loading = true;
    s.ask(Job::new("read log", move |core| {
        // One more than shown, to know whether there are more.
        let r = readlog::find(&core.home, &filter, limit + 1).map_err(jobs::line);
        Box::new(move |s: &mut State| {
            let st = &mut s.read_log;
            st.loading = false;
            match r {
                Ok(mut lines) => {
                    st.more = lines.len() > limit;
                    lines.truncate(limit);
                    st.entries = Some(lines);
                    st.limit = limit;
                    st.error = None;
                }
                Err(e) => st.error = Some(format!("Could not read the log: {e}")),
            }
        })
    }));
}

pub fn view(s: &State, p: &Paint, _: &mut Vec<El>) -> El {
    col()
        .w_full()
        .h_full()
        .child(header(&s.read_log, p))
        .child(p.separator())
        .child(lines(s, p))
}

fn field(
    p: &Paint,
    key: &str,
    label: &str,
    placeholder: &str,
    w: Option<f32>,
    edit: &TextEdit,
    get: fn(&mut State) -> &mut TextEdit,
) -> El {
    TextInput::new(edit)
        .label(label)
        .placeholder(placeholder)
        .width(w)
        .on_edit(get)
        .on_submit(|s: &mut State, _: &mut Tx| apply(s))
        .build(&p.k)
        .key(key)
}

fn header(st: &ReadLogState, p: &Paint) -> El {
    let filters = row()
        .key("filters")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            field(
                p,
                "filter-reader",
                "Reader",
                "owner, distill, a provider, or provider/model",
                None,
                &st.reader,
                |s| &mut s.read_log.reader,
            )
            .grow(1.0)
            .shrink()
            .min_w(0.0),
        )
        .child(field(
            p,
            "filter-since",
            "From",
            "From YYYY-MM-DD",
            Some(150.0),
            &st.since,
            |s| &mut s.read_log.since,
        ))
        .child(field(
            p,
            "filter-until",
            "To",
            "To YYYY-MM-DD",
            Some(150.0),
            &st.until,
            |s| &mut s.read_log.until,
        ))
        .child(
            Button::new(if st.loading { "Reading…" } else { "Apply" })
                .variant(ButtonVariant::Submit)
                .on_press(apply)
                .build(&p.k)
                .key("apply"),
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
                    text("Read log")
                        .key("heading")
                        .font(p.title())
                        .color(p.ink()),
                )
                .child(p.note(
                    "Every read of your sessions, every refusal, and every step of a reveal \
                     request, newest first. Days are in UTC. The log holds no session text \
                     and no value.",
                )),
        )
        .child(filters);
    for (key, e) in [("filter-error", &st.filter_error), ("error", &st.error)] {
        if let Some(e) = e {
            c = c.child(
                text(e.clone())
                    .key(key)
                    .font(p.caption())
                    .color(p.bad())
                    .wrap(),
            );
        }
    }
    c
}

/// What kind of line: a read, a refusal or failure, or a step of a reveal.
fn sort_of(r: &ReadResult) -> &'static str {
    match r {
        ReadResult::Served { .. }
        | ReadResult::Listed { .. }
        | ReadResult::Indexed { .. }
        | ReadResult::Findings { .. }
        | ReadResult::Tags { .. } => "Read",
        ReadResult::Refused { .. } => "Refused",
        ReadResult::Failed { .. } => "Failed",
        ReadResult::RevealRequested { .. }
        | ReadResult::RevealApproved { .. }
        | ReadResult::RevealDenied { .. }
        | ReadResult::RevealCollected { .. }
        | ReadResult::RevealExpired { .. } => "Reveal",
    }
}

/// `Nov 15, 11:48:00 UTC`.
fn stamp(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| {
            t.with_timezone(&chrono::Utc)
                .format("%b %-d, %H:%M:%S UTC")
                .to_string()
        })
        .unwrap_or_else(|_| rfc3339.to_string())
}

fn lines(s: &State, p: &Paint) -> El {
    let st = &s.read_log;
    let body = col()
        .key("results")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let Some(entries) = &st.entries else {
        return body.child(p.empty("loading", "Reading the log…"));
    };
    let summary = if st.more {
        format!("The newest {}", count(entries.len() as u64, "line"))
    } else {
        count(entries.len() as u64, "line")
    };
    let mut list = col()
        .key("entries")
        .w_full()
        .px(28.0)
        .pt(14.0)
        .gap(2.0)
        .child(
            text(summary)
                .key("summary")
                .font(p.caption())
                .color(p.muted()),
        );
    if entries.is_empty() {
        list = list.child(p.empty("none", "No reads match these filters."));
    }
    for (i, e) in entries.iter().enumerate() {
        let sort = sort_of(&e.result);
        let role = match sort {
            "Refused" | "Failed" => p.k.c.danger,
            "Reveal" => p.k.c.warning,
            _ => p.k.c.info,
        };
        let session = e.session.clone().unwrap_or_else(|| {
            match e.result {
                ReadResult::Listed { .. } => "a list of sessions",
                ReadResult::Findings { .. } => "findings",
                ReadResult::Tags { .. } => "the tags in use",
                ReadResult::Indexed { .. } => "the index",
                _ => "no session",
            }
            .to_string()
        });
        list = list.child(
            row()
                .key(format!("entry-{i}"))
                .w_full()
                .gap(12.0)
                .py(8.0)
                .center_y()
                .child(pill(p, sort, role).w(72.0))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            text(e.result.text())
                                .key("what")
                                .font(p.text())
                                .color(p.ink())
                                .wrap(),
                        )
                        .child(
                            text(format!("{} · {} · {}", stamp(&e.at), e.reader, session))
                                .key("who")
                                .font(p.caption())
                                .color(p.muted())
                                .truncate(),
                        ),
                )
                .child(
                    text(ago(&e.at, s.now()))
                        .key("when")
                        .font(p.caption())
                        .color(p.faint()),
                ),
        );
        list = list.child(p.separator());
    }
    let mut body = body.child(list);
    if st.more {
        body = body.child(
            row().w_full().py(12.0).center().child(
                Button::new(if st.loading { "Loading…" } else { "More" })
                    .on_press(more)
                    .build(&p.k)
                    .key("more"),
            ),
        );
    }
    body.child(col().h(20.0))
}
