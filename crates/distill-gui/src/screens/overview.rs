//! Overview: the status report (`distill_core::status::Status`), rendered. Sessions and capture
//! health per harness, format drift, the last sweep, warnings, each step's progress (scan, word
//! index, cards, vectors and the model detection pass), the reveal
//! requests waiting, the recent runs, the backup targets (each with Back up now and Verify) and
//! the exposure check, with a Fix button that calls `exposure::fix`. A step with nothing to report says it is not yet available. No session text
//! is shown.
//!
//! When a ticket adds a field to `Status`, it adds its line here: a step's progress goes in
//! [`steps`], anything else in its own section of [`report`].

use std::collections::{BTreeMap, BTreeSet};

use distill_core::backup::{self, BackupKey, TargetStatus};
use distill_core::exposure::{self, Exposure, Outcome, Rule};
use distill_core::runlog::{RunGroup, StepResult};
use distill_core::schedule;
use distill_core::status::{self, HarnessStatus, Health, Status, Warning, WarningKind};
use rune::{Align, El, ScrollSpec, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use crate::State;
use crate::jobs::{self, Job};
use crate::nav::{self, Screen};
use crate::screens::setup;
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
    /// Backup targets with a run or a check under way, by name.
    pub backing_up: BTreeSet<String>,
    pub verifying: BTreeSet<String>,
    /// The last run or check from this window, per target: its summary, or why it failed.
    pub backup_results: BTreeMap<String, Result<String, String>>,
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

/// The `[[backup]]` target `name` as config.toml has it now.
fn target(core: &jobs::Core, name: &str) -> anyhow::Result<distill_core::config::BackupTarget> {
    let config = core.home.load_config()?;
    Ok(backup::targets(&config, Some(name))?.remove(0))
}

/// Back up to `name` now, as `distill backup run --target` does, then reload the report. A first
/// run copies the whole archive, so it runs on its own thread. Setup's Finish calls it for each
/// folder, so the first backup to each starts with no further press.
pub(crate) fn back_up(s: &mut State, name: String) {
    if !s.overview.backing_up.insert(name.clone()) {
        return;
    }
    s.ask(Job::apart("back up", move |core| {
        let r = jobs::caught(|| backup::run(&core.home, &target(core, &name)?)).and_then(|r| {
            if r.ok {
                Ok(r.summary())
            } else {
                Err(r.summary())
            }
        });
        Box::new(move |s: &mut State| {
            s.overview.backing_up.remove(&name);
            s.overview.backup_results.insert(name, r);
            load(s);
        })
    }));
}

/// Check that `name` holds every object of its newest run, by name, as `distill backup verify`.
fn verify(s: &mut State, name: String) {
    if !s.overview.verifying.insert(name.clone()) {
        return;
    }
    s.ask(Job::apart("verify backup", move |core| {
        let r = jobs::caught(|| {
            let key = BackupKey::existing(&core.home)?
                .ok_or_else(|| anyhow::anyhow!("this home has no backup key"))?;
            let t = target(core, &name)?;
            let store = backup::store::open(&t, false)?;
            backup::verify(&core.home, &t.name, &*store, &key, false)
        })
        .and_then(|c| {
            if c.ok() {
                Ok(c.summary())
            } else {
                Err(c.summary())
            }
        });
        Box::new(move |s: &mut State| {
            s.overview.verifying.remove(&name);
            s.overview.backup_results.insert(name, r);
        })
    }));
}

pub fn view(s: &State, p: &Paint, _: &mut Vec<El>) -> El {
    let body = match (&s.overview.status, &s.overview.error) {
        (Some(st), _) if !st.configured => message(
            p,
            "DISTILL is not set up yet",
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
    // While a backup runs, its warnings (back up once, the last run failed) are out of date.
    let shown: Vec<&Warning> = st
        .warnings
        .iter()
        .filter(|w| s.overview.backing_up.is_empty() || w.kind != WarningKind::Backup)
        .collect();
    if !shown.is_empty() {
        body = body.child(warnings(&shown, p));
    }
    body.child(progress(st, p))
        .child(backups(s, st, p))
        .child(runs(s, st, p))
        .child(reveals(st, p))
        .child(left(st, p))
        .child(harnesses(s, st, p))
        .child(exposure_section(s, &st.exposure, p))
}

fn tiles(st: &Status, p: &Paint) -> El {
    let tile = |key: &str, label: &str, value: String| {
        p.card()
            .key(key)
            .grow(1.0)
            .gap(4.0)
            .gradient(p.t.card, p.t.card.mix(p.brand, 0.035))
            .child(
                row()
                    .gap(6.0)
                    .center_y()
                    .child(dot(p.brand, 6.0))
                    .child(text(label).font(p.caption()).color(p.muted())),
            )
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

fn warnings(shown: &[&Warning], p: &Paint) -> El {
    let mut card = p.section("warnings", "Needs your attention", None);
    for (i, w) in shown.iter().enumerate() {
        let kind = match w.kind {
            WarningKind::SweepOverdue => "Sweep overdue",
            WarningKind::PullOverdue => "Pull overdue",
            WarningKind::PrunesSooner => "Prunes sooner",
            WarningKind::Exposure => "Exposure",
            WarningKind::Backup => "Backup",
            WarningKind::ReadLog => "Read log",
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
    /// How much of the step is done, as (done, of), for its meter.
    done: (u64, u64),
    detail: Vec<String>,
}

/// `part` of `whole`, 0 to 1 (0 when there is nothing to do).
fn share(part: u64, whole: u64) -> f32 {
    if whole == 0 {
        0.0
    } else {
        (part as f64 / whole as f64).min(1.0) as f32
    }
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
            done: (scan.current, scan.sessions),
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
            done: (index.current, index.sessions),
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
            done: (cards.current + cards.stale, cards.sessions),
            detail: [
                waiting(cards.pending),
                (cards.stale > 0).then(|| format!("{} stale", thousands(cards.stale))),
                (cards.failing > 0).then(|| format!("{} to retry", thousands(cards.failing))),
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
            done: (vectors.current, vectors.sessions),
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
            done: (details.current, details.sessions),
            detail: [
                waiting(details.pending),
                (details.failing > 0).then(|| format!("{} to retry", thousands(details.failing))),
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
        let mut value = col().key("progress").w(270.0).gap(6.0);
        let (part, whole) = step.done;
        let pct = percent(part, whole);
        match step.value {
            Some(v) => {
                value = value
                    .child(
                        row()
                            .w_full()
                            .gap(8.0)
                            .center_y()
                            .justify_between()
                            .child(text(v).key("value").font(p.num()).color(p.ink()))
                            .child(
                                text(format!("{pct}%"))
                                    .key("percent")
                                    .font(p.k.ty.small_medium.features("'tnum' 1"))
                                    .color(if pct == 100 { p.brand } else { p.muted() }),
                            ),
                    )
                    .child(p.meter(share(part, whole)));
            }
            None => {
                value = value.child(
                    text("Not yet available")
                        .key("value")
                        .font(p.num())
                        .color(p.faint()),
                );
            }
        }
        card = card.child(
            row()
                .key(format!("step-{}", step.id))
                .w_full()
                .py(2.0)
                .gap(16.0)
                .center_y()
                .child(
                    text(step.name)
                        .font(p.k.ty.ui_medium)
                        .color(p.ink())
                        .w(150.0),
                )
                .child(value)
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

/// `part` of `whole` as a whole percent, rounded down so a step reads 100% only when it is
/// done. Counted in integers: through a float, 53 of 100 can come out as 52.
fn percent(part: u64, whole: u64) -> u64 {
    if whole == 0 {
        0
    } else {
        (u128::from(part.min(whole)) * 100 / u128::from(whole)) as u64
    }
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

/// What left this machine (#162): each place text went, with the last day's and the last 7
/// days' reads and sessions (runs and bytes for a backup), then what stayed here.
fn left(st: &Status, p: &Paint) -> El {
    use distill_core::readlog::{PlaceKind, Sent};
    let mut card = p.section("left", "What left this machine", None);
    let read = |s: &Sent| {
        let mut t = count(s.times, "read");
        if s.sessions > 0 {
            t += &format!(", {}", count(s.sessions, "session"));
        }
        if s.listed > 0 {
            t += &format!(", {} listed", s.listed);
        }
        t
    };
    let ran = |s: &Sent| format!("{}, {}", count(s.times, "run"), human_bytes(s.bytes));
    let exported = |s: &Sent| format!("{}, {}", count(s.times, "export"), human_bytes(s.bytes));
    let cell = |t: El, font, ink| t.font(font).color(ink).wrap().w(0.0).grow(1.0).min_w(0.0);
    let line = |key: String, name: String, day: String, week: String, head: bool| {
        let font = if head { p.caption() } else { p.text() };
        let ink = if head { p.muted() } else { p.ink() };
        row()
            .key(key)
            .w_full()
            .gap(8.0)
            .center_y()
            // Equal columns that share the card's width and wrap, so a long place name or count
            // never runs past the card.
            .child(cell(text(name).key("name"), font, ink))
            .child(cell(text(day).key("day"), font, ink))
            .child(cell(text(week).key("week"), font, ink))
    };
    card = card.child(line(
        "left-head".into(),
        "Where".into(),
        "Last day".into(),
        "Last 7 days".into(),
        true,
    ));
    if st.left.places.is_empty() {
        card = card.child(
            text("Nothing left this machine in the last 7 days.")
                .key("left-none")
                .font(p.text())
                .color(p.muted()),
        );
    }
    for (i, place) in st.left.places.iter().enumerate() {
        let (name, f): (String, &dyn Fn(&Sent) -> String) = match place.kind {
            PlaceKind::Reader => (place.name.clone(), &read),
            PlaceKind::Backup => (format!("Backup {}", place.name), &ran),
            PlaceKind::Hub => ("The hub, over SSH".into(), &exported),
        };
        card = card.child(line(
            format!("left-{i}"),
            name,
            f(&place.spans.day),
            f(&place.spans.week),
            false,
        ));
    }
    card = card.child(p.separator()).child(
        line(
            "left-here".into(),
            "Stayed on this machine".into(),
            read(&st.left.here.day),
            read(&st.left.here.week),
            false,
        )
        .opacity(0.8),
    );
    if st.left.maybe_short {
        card = card.child(p.note("The read log has been edited, so these counts may be short."));
    }
    card
}

/// Each backup target: its last whole run, how far behind it is, its last failure, and buttons
/// to back up now or check it. With none, a way to Setup's Backups step.
fn backups(s: &State, st: &Status, p: &Paint) -> El {
    let mut card = p.section("backups", "Backups", None);
    if st.backups.is_empty() {
        return card.child(
            row()
                .key("backups-none")
                .w_full()
                .gap(12.0)
                .center_y()
                .justify_between()
                .child(
                    text("No backup folder is set up, so this archive is the only copy.")
                        .key("message")
                        .font(p.text())
                        .color(p.muted())
                        .wrap()
                        .shrink(),
                )
                .child(
                    Button::new("Set up backups")
                        .control_size(ControlSize::Small)
                        .on_press(|s: &mut State| {
                            nav::go(s, Screen::Setup);
                            s.setup.go(setup::BACKUPS);
                        })
                        .build(&p.k)
                        .key("set-up-backups"),
                ),
        );
    }
    for (i, t) in st.backups.iter().enumerate() {
        if i > 0 {
            card = card.child(p.separator());
        }
        card = card.child(backup_row(s, t, p));
    }
    card
}

fn backup_row(s: &State, t: &TargetStatus, p: &Paint) -> El {
    let place = s
        .config
        .as_ref()
        .and_then(|c| c.backups.iter().find(|b| b.name == t.name))
        .and_then(|b| b.path.as_deref())
        .map_or(t.kind.clone(), crate::ui::tilde);
    let failed = t.last_failed.is_some();
    let running = s.overview.backing_up.contains(&t.name);
    let (color, state) = match (&t.last_ok, t.captures_behind) {
        (None, _) if running => (p.warn(), "First backup running…".to_string()),
        _ if failed => (p.bad(), "Last run failed".to_string()),
        (None, _) => (p.warn(), "Not backed up yet".to_string()),
        (Some(_), Some(n)) if n > 0 => (
            p.warn(),
            format!("{} not backed up yet", count(n, "session update")),
        ),
        (Some(_), _) => (p.good(), "Up to date".to_string()),
    };
    let when = match &t.last_ok {
        Some(at) => format!("Last backup {}", ago(at, s.now())),
        None => "No backup yet".to_string(),
    };
    let name = t.name.clone();
    let back_up_now = Button::new(if running {
        "Backing up…"
    } else {
        "Back up now"
    })
    .control_size(ControlSize::Small)
    .disabled(running)
    .on_press(move |s: &mut State| back_up(s, name.clone()))
    .build(&p.k)
    .key("back-up");
    let name = t.name.clone();
    let checking = s.overview.verifying.contains(&t.name);
    let check = Button::new(if checking { "Verifying…" } else { "Verify" })
        .variant(ButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .disabled(checking || t.last_ok.is_none())
        .on_press(move |s: &mut State| verify(s, name.clone()))
        .build(&p.k)
        .key("verify");
    let mut c = col()
        .key(format!("backup-{}", t.name))
        .w_full()
        .py(2.0)
        .gap(6.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(
                    col()
                        .gap(2.0)
                        .grow(1.0)
                        .shrink()
                        .child(
                            row()
                                .gap(8.0)
                                .center_y()
                                .child(text(t.name.clone()).font(p.k.ty.ui_medium).color(p.ink()))
                                .child(text(place).font(p.mono()).color(p.faint()).truncate()),
                        )
                        .child(
                            row()
                                .key("state")
                                .gap(6.0)
                                .center_y()
                                .child(dot(color, 7.0))
                                .child(text(state).font(p.caption()).color(p.ink()))
                                .child(text("·").font(p.caption()).color(p.faint()))
                                .child(text(when).key("when").font(p.caption()).color(p.muted())),
                        ),
                )
                .child(row().gap(6.0).child(check).child(back_up_now)),
        );
    if let (Some(at), Some(e)) = (&t.last_failed, &t.last_error) {
        c = c.child(
            text(format!("Failed {}: {e}", ago(at, s.now())))
                .key("failure")
                .font(p.caption())
                .color(p.bad())
                .wrap(),
        );
    }
    if let Some(r) = s.overview.backup_results.get(&t.name) {
        let (line, color) = match r {
            Ok(m) => (m.clone(), p.muted()),
            Err(e) => (e.clone(), p.bad()),
        };
        c = c.child(
            text(line)
                .key("result")
                .font(p.caption())
                .color(color)
                .wrap(),
        );
    }
    c
}

/// How many groups of runs the overview shows.
const RUN_ROWS: usize = 8;

/// The recent runs, newest first: runs in a row that came to the same results are one row, each
/// row leads with when it ran (with a muted caption only for grouped counts or single-step runs),
/// each step that did not simply finish is named, and a failure's sentence is shown under its row.
fn runs(s: &State, st: &Status, p: &Paint) -> El {
    let cadence = s.config.as_ref().map(|c| {
        let every = match schedule::interval_minutes(c) {
            60 => "Hourly".to_string(),
            m if m.is_multiple_of(60) => format!("Every {} h", m / 60),
            m => format!("Every {m} min"),
        };
        text(format!("{every} · sweep to models"))
            .key("cadence")
            .font(p.caption())
            .color(p.muted())
    });
    let mut card = p.section("runs", "Recent runs", cadence);
    if st.runs.is_empty() {
        return card.child(
            text("No runs yet: the timer's first scheduled run will show here.")
                .key("message")
                .font(p.text())
                .color(p.muted()),
        );
    }
    for (i, g) in st.runs.iter().take(RUN_ROWS).enumerate() {
        if i > 0 {
            card = card.child(p.separator());
        }
        card = card.child(run_row(s, i, g, p));
    }
    card
}

fn run_row(s: &State, i: usize, g: &RunGroup, p: &Paint) -> El {
    let now = s.now();
    let newest = ago(&g.newest, now);
    let when = if g.runs == 1 {
        newest
    } else {
        let oldest = ago(&g.oldest, now);
        if oldest == newest {
            newest
        } else {
            format!("{oldest} to {newest}")
        }
    };
    let what = if g.step == "tick" {
        (g.runs > 1).then(|| format!("{} scheduled runs", g.runs))
    } else {
        let mut name = g.step.clone();
        if let Some(first) = name.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        Some(if g.runs == 1 {
            format!("{name} on its own")
        } else {
            format!("{name} on its own · {} runs", g.runs)
        })
    };
    let mut marks = row().key("steps").gap(6.0).center_y().flex_wrap();
    let mut all_ok = true;
    for m in &g.steps {
        let (label, role) = match m.result {
            StepResult::Ok => continue,
            StepResult::Stopped => (format!("{}: time limit", m.name), p.k.c.info),
            StepResult::Skipped => (format!("{}: skipped", m.name), p.k.c.warning),
            StepResult::Failed => (format!("{}: failed", m.name), p.k.c.danger),
        };
        all_ok = false;
        marks = marks.child(pill(p, &label, role));
    }
    if all_ok {
        let label = if g.steps.len() == 1 {
            "OK"
        } else {
            "All steps OK"
        };
        marks = marks.child(pill(p, label, p.k.c.success));
    } else if g.steps.iter().any(|m| m.result == StepResult::Ok) {
        marks = marks.child(text("others OK").font(p.caption()).color(p.muted()));
    }
    let mut heading = col()
        .gap(2.0)
        .child(text(when).key("when").font(p.text()).color(p.ink()));
    if let Some(what) = what {
        heading = heading.child(text(what).key("what").font(p.caption()).color(p.muted()));
    }
    let mut out = col()
        .key(format!("run-{i}"))
        .w_full()
        .py(2.0)
        .gap(6.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .justify_between()
                .child(heading)
                .child(marks),
        );
    for (j, f) in g.failures.iter().enumerate() {
        out = out.child(
            text(f.clone())
                .key(format!("failure-{j}"))
                .font(p.caption())
                .color(p.bad())
                .wrap(),
        );
    }
    out
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
    let drift: u64 = st.harnesses.iter().map(|h| h.drift + h.partial_drift).sum();
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
            num("drift", thousands(h.drift + h.partial_drift), w[6]).color(
                if h.drift + h.partial_drift > 0 {
                    p.warn()
                } else {
                    p.muted()
                },
            ),
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

#[cfg(test)]
mod tests {
    use super::percent;

    #[test]
    fn a_percent_is_counted_exactly_and_reads_100_only_when_done() {
        assert_eq!(percent(53, 100), 53);
        assert_eq!(percent(59, 100), 59);
        assert_eq!(percent(999, 1000), 99);
        assert_eq!(percent(1000, 1000), 100);
        assert_eq!(percent(5, 0), 0);
        assert_eq!(percent(u64::MAX, u64::MAX), 100);
    }
}
