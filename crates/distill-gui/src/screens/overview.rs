//! Overview: the status report (`distill_core::status::Status`), rendered. Sessions and capture
//! health per harness, format drift, the last sweep, warnings, each step's progress (scan, word
//! index, cards, vectors and the model detection pass), the reveal
//! requests waiting and the exposure check, with a Fix button that calls `exposure::fix`. A step
//! with nothing to report says it is not yet available. No session text is shown.
//!
//! When a ticket adds a field to `Status`, it adds its line here: a step's progress goes in
//! [`steps`], anything else in its own section of [`report`].

use distill_core::exposure::{self, Exposure, Outcome, Rule};
use distill_core::status::{self, HarnessStatus, Health, Status, WarningKind};
use rune::{Align, El, ScrollSpec, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use crate::State;
use crate::jobs::{self, Job};
use crate::nav::{self, Screen};
use crate::ui::{Paint, ago, count, dot, human_bytes, pill, thousands};

/// Seconds between reloads while the overview shows.
const REFRESH_SECS: f64 = 30.0;

#[derive(Default)]
pub struct OverviewState {
    pub status: Option<Status>,
    pub error: Option<String>,
    pub loading: bool,
    pub fixing: bool,
    /// A one-line result of the last action.
    pub notice: Option<String>,
    next_refresh: f64,
}

pub fn open(s: &mut State) {
    load(s);
}

/// Ask core for the status report.
pub fn load(s: &mut State) {
    if s.overview.loading {
        return;
    }
    s.overview.loading = true;
    s.ask(Job::new("status", |core| {
        let r = status::status(&core.home).map_err(jobs::line);
        Box::new(move |s: &mut State| {
            s.overview.loading = false;
            match r {
                Ok(st) => {
                    s.overview.status = Some(st);
                    s.overview.error = None;
                }
                Err(e) => s.overview.error = Some(e),
            }
        })
    }));
}

/// Reload the report every [`REFRESH_SECS`] while the overview shows; returns how long to sleep.
pub fn refresh(s: &mut State, clock: f64) -> Option<f64> {
    if s.screen != Screen::Overview {
        return None;
    }
    if clock >= s.overview.next_refresh {
        if s.overview.next_refresh > 0.0 {
            load(s);
        }
        s.overview.next_refresh = clock + REFRESH_SECS;
    }
    Some((s.overview.next_refresh - clock).max(0.5))
}

/// Tighten the home's modes (`exposure::fix`), then reload the report.
fn fix(s: &mut State) {
    if s.overview.fixing {
        return;
    }
    s.overview.fixing = true;
    s.ask(Job::new("fix exposure", |core| {
        let r = exposure::fix(&core.home).map_err(jobs::line);
        Box::new(move |s: &mut State| {
            s.overview.fixing = false;
            s.overview.notice = Some(match r {
                Ok(1) => "Made 1 file or folder owner-only".to_string(),
                Ok(n) => format!("Made {} files and folders owner-only", thousands(n)),
                Err(e) => format!("Could not fix the modes: {e}"),
            });
            load(s);
        })
    }));
}

pub fn view(s: &State, p: &Paint, _: &mut Vec<El>) -> El {
    let body = match (&s.overview.status, &s.overview.error) {
        (Some(st), _) if !st.configured => message(
            p,
            "Distill is not set up yet",
            "Setup chooses what to archive and starts the timer.",
            true,
        ),
        (Some(st), _) => report(s, st, p),
        (None, Some(e)) => message(p, "Could not read the status report", e, false),
        (None, None) => message(p, "Reading the archive…", "", false),
    };
    col().w_full().h_full().child(header(s, p)).child(body)
}

fn header(s: &State, p: &Paint) -> El {
    let st = s.overview.status.as_ref();
    let (color, sweep) = match st.and_then(|st| st.last_sweep.as_deref()) {
        Some(at) => {
            let overdue = st.is_some_and(|st| {
                st.warnings
                    .iter()
                    .any(|w| w.kind == WarningKind::SweepOverdue)
            });
            (
                if overdue { p.warn() } else { p.good() },
                format!("Last sweep {}", ago(at, s.now())),
            )
        }
        None if st.is_some() => (p.off(), "No sweep yet".to_string()),
        None => (p.off(), "Loading…".to_string()),
    };
    let mut right = row().gap(12.0).center_y();
    if let Some(n) = &s.overview.notice {
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
        Button::new(if s.overview.loading {
            "Loading…"
        } else {
            "Refresh"
        })
        .disabled(s.overview.loading)
        .on_press(load)
        .build(&p.k)
        .key("refresh"),
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
                .child(text("Overview").font(p.title()).color(p.ink()))
                .child(
                    row()
                        .key("sweep")
                        .role("status")
                        .label(sweep.clone())
                        .gap(6.0)
                        .center_y()
                        .child(dot(color, 7.0))
                        .child(
                            text(sweep)
                                .key("last-sweep")
                                .font(p.caption())
                                .color(p.muted()),
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
                .on_press(|s: &mut State| nav::go(s, Screen::Setup))
                .build(&p.k)
                .key("open-setup"),
        );
    }
    c
}

fn report(s: &State, st: &Status, p: &Paint) -> El {
    let mut body = col()
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
        .child(tiles(st, p));
    if !st.warnings.is_empty() {
        body = body.child(warnings(st, p));
    }
    body.child(progress(st, p))
        .child(reveals(st, p))
        .child(harnesses(s, st, p))
        .child(exposure_section(s, &st.exposure, p))
}

fn tiles(st: &Status, p: &Paint) -> El {
    let tile = |key: &str, label: &str, value: String| {
        p.card()
            .key(key)
            .grow(1.0)
            .gap(4.0)
            .child(text(label).font(p.caption()).color(p.muted()))
            .child(text(value).key("value").font(p.stat()).color(p.ink()))
    };
    let capturing = st.harnesses.iter().filter(|h| h.enabled).count() as u64;
    row()
        .key("tiles")
        .w_full()
        .gap(12.0)
        .child(tile("tile-sessions", "Sessions", thousands(st.sessions)))
        .child(tile(
            "tile-archive",
            "Raw archive",
            human_bytes(st.archive_bytes),
        ))
        .child(tile(
            "tile-harnesses",
            "Harnesses captured",
            thousands(capturing),
        ))
}

fn warnings(st: &Status, p: &Paint) -> El {
    let mut card = p.section("warnings", "Needs your attention", None);
    for (i, w) in st.warnings.iter().enumerate() {
        let kind = match w.kind {
            WarningKind::SweepOverdue => "Sweep overdue",
            WarningKind::PrunesSooner => "Prunes sooner",
            WarningKind::Exposure => "Exposure",
        };
        if i > 0 {
            card = card.child(p.separator());
        }
        card = card.child(
            row()
                .key(format!("warning-{i}"))
                .w_full()
                .gap(12.0)
                .child(pill(p, kind, p.k.c.warning).key("kind").w(110.0))
                .child(
                    text(w.message.clone())
                        .key("message")
                        .font(p.text())
                        .color(p.ink())
                        .wrap()
                        .grow(1.0)
                        .shrink(),
                ),
        );
    }
    card
}

/// One step's line in the progress section: what it has done, or that it has nothing yet.
struct Step {
    id: &'static str,
    name: &'static str,
    /// None: nothing to report yet.
    value: Option<String>,
    detail: Vec<String>,
}

/// The tick's steps that report progress, in the order they run.
fn steps(st: &Status) -> Vec<Step> {
    let waiting = |n: u64| (n > 0).then(|| format!("{} waiting", thousands(n)));
    let scan = &st.scan;
    let index = &st.index;
    let cards = &st.cards;
    let vectors = &st.vectors;
    let details = &st.details;
    vec![
        Step {
            id: "scan",
            name: "Scan for findings",
            value: (scan.sessions > 0).then(|| {
                format!(
                    "{} of {} scanned",
                    thousands(scan.current),
                    thousands(scan.sessions)
                )
            }),
            detail: [
                waiting(scan.pending),
                (scan.sessions > 0).then(|| format!("{} found", count(scan.secrets, "secret"))),
            ]
            .into_iter()
            .flatten()
            .collect(),
        },
        Step {
            id: "index",
            name: "Word index",
            value: (index.sessions > 0).then(|| {
                format!(
                    "{} of {} indexed",
                    thousands(index.current),
                    thousands(index.sessions)
                )
            }),
            detail: [
                waiting(index.pending),
                (index.bytes > 0).then(|| human_bytes(index.bytes)),
            ]
            .into_iter()
            .flatten()
            .collect(),
        },
        Step {
            id: "cards",
            name: "Cards",
            value: (cards.sessions > 0).then(|| {
                format!(
                    "{} of {} with a card",
                    thousands(cards.current + cards.stale),
                    thousands(cards.sessions)
                )
            }),
            detail: [
                waiting(cards.pending),
                (cards.stale > 0).then(|| format!("{} stale", thousands(cards.stale))),
                (cards.failing > 0).then(|| format!("{} failing", thousands(cards.failing))),
                cards.model.clone(),
                cards.skipped.as_ref().map(|why| format!("Skipped: {why}")),
            ]
            .into_iter()
            .flatten()
            .collect(),
        },
        Step {
            id: "vectors",
            name: "Search by meaning",
            value: (vectors.sessions > 0).then(|| {
                format!(
                    "{} of {} with vectors",
                    thousands(vectors.current),
                    thousands(vectors.sessions)
                )
            }),
            detail: [
                waiting(vectors.pending),
                (vectors.bytes > 0).then(|| human_bytes(vectors.bytes)),
                vectors.model.clone(),
                vectors
                    .skipped
                    .as_ref()
                    .map(|why| format!("Skipped: {why}")),
            ]
            .into_iter()
            .flatten()
            .collect(),
        },
        Step {
            id: "details",
            name: "Model detection pass",
            value: (details.sessions > 0).then(|| {
                format!(
                    "{} of {} checked by the model",
                    thousands(details.current),
                    thousands(details.sessions)
                )
            }),
            detail: [
                waiting(details.pending),
                (details.failing > 0).then(|| format!("{} failing", thousands(details.failing))),
                (details.sessions > 0)
                    .then(|| format!("{} found", count(details.found, "personal detail"))),
                details.model.clone(),
                details
                    .skipped
                    .as_ref()
                    .map(|why| format!("Skipped: {why}")),
            ]
            .into_iter()
            .flatten()
            .collect(),
        },
    ]
}

fn progress(st: &Status, p: &Paint) -> El {
    let mut card = p.section("progress", "Progress", None);
    for (i, step) in steps(st).into_iter().enumerate() {
        if i > 0 {
            card = card.child(p.separator());
        }
        let (value, color) = match step.value {
            Some(v) => (v, p.ink()),
            None => ("Not yet available".to_string(), p.faint()),
        };
        card = card.child(
            row()
                .key(format!("step-{}", step.id))
                .w_full()
                .gap(16.0)
                .center_y()
                .child(
                    text(step.name)
                        .font(p.k.ty.ui_medium)
                        .color(p.ink())
                        .w(150.0),
                )
                .child(text(value).key("value").font(p.num()).color(color).w(220.0))
                .child(
                    text(step.detail.join(" · "))
                        .key("detail")
                        .font(p.caption())
                        .color(p.muted())
                        .wrap()
                        .grow(1.0)
                        .shrink(),
                ),
        );
    }
    card
}

fn reveals(st: &Status, p: &Paint) -> El {
    let n = st.pending_reveals;
    let mut line = row()
        .key("reveals-line")
        .w_full()
        .gap(12.0)
        .center_y()
        .justify_between();
    if n == 0 {
        line = line.child(
            text("No reveal requests are waiting.")
                .key("message")
                .font(p.text())
                .color(p.muted()),
        );
    } else {
        line = line
            .child(
                row().gap(8.0).center_y().child(dot(p.warn(), 8.0)).child(
                    text(if n == 1 {
                        "1 reveal request waits for you".to_string()
                    } else {
                        format!("{} reveal requests wait for you", thousands(n))
                    })
                    .key("message")
                    .font(p.text())
                    .color(p.ink()),
                ),
            )
            .child(
                Button::new("Open approvals")
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut State| nav::go(s, Screen::Approvals))
                    .build(&p.k)
                    .key("open-approvals"),
            );
    }
    p.section("reveals", "Reveal requests", None).child(line)
}

fn health(h: Health) -> &'static str {
    match h {
        Health::Ok => "OK",
        Health::Warn => "Warning",
        Health::Missing => "Missing",
        Health::Disabled => "Off",
    }
}

fn harnesses(s: &State, st: &Status, p: &Paint) -> El {
    let drift: u64 = st.harnesses.iter().map(|h| h.drift).sum();
    let drift_line = if drift == 0 {
        text("No format drift").color(p.muted())
    } else {
        text(format!(
            "{} look like format drift",
            count(drift, "capture")
        ))
        .color(p.ink())
    };
    let mut card = p
        .section(
            "harnesses",
            "Harnesses",
            Some(drift_line.key("drift").font(p.caption())),
        )
        .gap(0.0);
    let col_w = [140.0, 90.0, 80.0, 80.0, 80.0, 100.0, 60.0];
    let head = [
        "Harness",
        "Health",
        "Sessions",
        "Captures",
        "Archive",
        "Last capture",
        "Drift",
    ];
    let mut header = row().key("columns").w_full().py(8.0).gap(8.0);
    for (h, w) in head.iter().zip(col_w) {
        header = header.child(text(*h).font(p.caption()).color(p.faint()).w(w));
    }
    card = card.child(header).child(p.separator());
    for h in st.harnesses.iter().filter(|h| h.enabled) {
        card = card.child(harness_row(s, h, p, &col_w));
    }
    let off: Vec<&str> = st
        .harnesses
        .iter()
        .filter(|h| !h.enabled)
        .map(|h| h.label.as_str())
        .collect();
    if !off.is_empty() {
        card = card.child(
            text(format!("Not captured: {}.", off.join(", ")))
                .key("off")
                .font(p.caption())
                .color(p.muted())
                .wrap()
                .pt(10.0),
        );
    }
    card
}

fn harness_row(s: &State, h: &HarnessStatus, p: &Paint, w: &[f32; 7]) -> El {
    let color = match h.health {
        Health::Ok => p.good(),
        Health::Warn => p.warn(),
        Health::Missing | Health::Disabled => p.off(),
    };
    let num = |key: &str, v: String, w: f32| text(v).key(key).font(p.num()).color(p.ink()).w(w);
    let line = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            text(h.label.clone())
                .font(p.k.ty.ui_medium)
                .color(p.ink())
                .w(w[0]),
        )
        .child(
            row()
                .w(w[1])
                .gap(6.0)
                .center_y()
                .child(dot(color, 7.0))
                .child(
                    text(health(h.health))
                        .key("health")
                        .font(p.text())
                        .color(p.ink()),
                ),
        )
        .child(num("sessions", thousands(h.sessions), w[2]))
        .child(num("captures", thousands(h.captures), w[3]))
        .child(num("size", human_bytes(h.archive_bytes), w[4]))
        .child(
            text(
                h.last_capture
                    .as_deref()
                    .map_or("Not yet".to_string(), |at| ago(at, s.now())),
            )
            .key("last")
            .font(p.text())
            .color(p.muted())
            .w(w[5]),
        )
        .child(
            num("drift", thousands(h.drift), w[6]).color(if h.drift > 0 {
                p.warn()
            } else {
                p.muted()
            }),
        );
    let mut c = col()
        .key(format!("harness-{}", h.harness.id()))
        .w_full()
        .py(8.0)
        .gap(2.0)
        .child(line);
    let mut notes = Vec::new();
    if let Some(n) = &h.note {
        notes.push(n.clone());
    }
    if h.capture_errors > 0 {
        notes.push(format!(
            "{} failed to capture",
            count(h.capture_errors, "file")
        ));
    }
    if h.parse_failures > h.drift {
        notes.push(format!(
            "{} failed to parse",
            count(h.parse_failures - h.drift, "capture")
        ));
    }
    if !notes.is_empty() {
        c = c.child(
            text(notes.join(" · "))
                .key("note")
                .font(p.caption())
                .color(p.muted())
                .pl(w[0] + 8.0),
        );
    }
    c
}

fn rule_id(r: Rule) -> &'static str {
    match r {
        Rule::OwnerOnly => "owner-only",
        Rule::KeyOwnerOnly => "key-owner-only",
        Rule::NotSynced => "not-synced",
        Rule::NoListener => "no-listener",
    }
}

fn exposure_section(s: &State, e: &Exposure, p: &Paint) -> El {
    // `exposure::fix` tightens modes, which is what the two owner-only rules check.
    let fixable = e.rules.iter().any(|r| {
        r.outcome == Outcome::Fail && matches!(r.rule, Rule::OwnerOnly | Rule::KeyOwnerOnly)
    });
    let trailing = fixable.then(|| {
        Button::new(if s.overview.fixing {
            "Fixing…"
        } else {
            "Fix modes"
        })
        .variant(ButtonVariant::Submit)
        .control_size(ControlSize::Small)
        .disabled(s.overview.fixing)
        .on_press(fix)
        .build(&p.k)
        .key("fix-exposure")
    });
    let mut card = p.section("exposure", "Exposure", trailing);
    for r in &e.rules {
        let (word, role) = match r.outcome {
            Outcome::Pass => ("Pass", p.k.c.success),
            Outcome::Fail => ("Fail", p.k.c.danger),
            Outcome::Unchecked => ("Not checked", p.k.c.info),
        };
        let mut c = col()
            .gap(2.0)
            .grow(1.0)
            .shrink()
            .child(text(r.rule.label()).font(p.k.ty.ui_medium).color(p.ink()))
            .child(p.note(r.message.clone()).key("message"));
        for path in &r.paths {
            c = c.child(
                text(path.clone())
                    .font(p.mono())
                    .color(p.muted())
                    .truncate(),
            );
        }
        card = card.child(
            row()
                .key(format!("rule-{}", rule_id(r.rule)))
                .w_full()
                .gap(12.0)
                .child(pill(p, word, role).key("outcome").w(90.0))
                .child(c),
        );
    }
    card = card
        .child(p.separator())
        .child(text(e.disk()).key("disk").font(p.text()).color(p.ink()))
        .child(p.note(e.backups.clone()).key("backups"));
    if !e.previous_build.is_empty() {
        let mut list = col().key("previous-build").gap(4.0).child(
            text("Left by the previous build (ignored, never deleted):")
                .font(p.caption())
                .color(p.muted()),
        );
        for f in &e.previous_build {
            list = list.child(
                row()
                    .gap(8.0)
                    .child(
                        text(format!("{} {}", f.path, f.mode))
                            .font(p.mono())
                            .color(if f.owner_only { p.muted() } else { p.warn() }),
                    )
                    .child(text(f.holds.clone()).font(p.caption()).color(p.muted())),
            );
        }
        card = card.child(list);
    }
    card
}
