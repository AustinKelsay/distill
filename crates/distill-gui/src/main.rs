//! DISTILL's window: one window with navigation between Setup, Overview, Sessions, Findings,
//! Approvals, Read log and Guide (`nav.rs`; each screen is a module under `screens/`).
//!
//! The window holds no logic. It reads and writes only through distill-core, in jobs on a
//! worker thread (`jobs.rs`), so no file I/O runs in the frame loop. Setup and the overview
//! show no session text. This is the only crate that depends on Rune (ADR 0007).
//!
//! `cargo run -p distill-gui` opens the window; `-- --snap out.png --scene overview` renders a
//! fixture headless (`--list-scenes` lists them). A headless run never starts the worker, so it
//! reads nothing from disk and never installs the timer.

mod cover;
mod fixture;
mod jobs;
mod nav;
mod screens;
mod ui;

#[cfg(test)]
mod tests;

use std::sync::OnceLock;
use std::time::Duration;

use chrono::{DateTime, Utc};
use distill_core::{Config, Home};
use rune::{AppSpec, Cx, El, Tx, Waker, el, row};

use crate::jobs::{Core, Job, Link};
use crate::nav::Screen;
use crate::screens::{approvals, findings, overview, read_log, sessions, setup};
use crate::ui::Paint;

pub struct State {
    pub screen: Screen,
    pub home: Home,
    /// The config as last saved; None until setup has run.
    pub config: Option<Config>,
    /// Why the config could not be read, when it exists but doesn't load.
    pub config_error: Option<String>,
    pub setup: setup::SetupState,
    pub overview: overview::OverviewState,
    pub sessions: sessions::SessionsState,
    pub findings: findings::FindingsState,
    pub approvals: approvals::ApprovalsState,
    pub read_log: read_log::ReadLogState,
    /// Jobs waiting for the worker. `poll` sends them; tests run them with `jobs::run_queued`.
    pub outbox: Vec<Job>,
    pub link: Option<Link>,
    /// Scenes and tests install data directly; the worker is never started.
    pub fixture: bool,
    /// A fixed clock for scenes, so relative times don't change between snapshots.
    pub clock: Option<DateTime<Utc>>,
}

impl State {
    /// A window over `home` with `config` (None: setup has not run, so it opens on the Guide).
    pub fn new(home: Home, config: Option<Config>) -> State {
        let screen = if config.is_some() {
            Screen::Overview
        } else {
            Screen::Guide
        };
        State {
            screen,
            setup: setup::SetupState::new(config.as_ref()),
            home,
            config,
            config_error: None,
            overview: Default::default(),
            sessions: Default::default(),
            findings: Default::default(),
            approvals: Default::default(),
            read_log: Default::default(),
            outbox: Vec::new(),
            link: None,
            fixture: false,
            clock: None,
        }
    }

    /// Queue `job` for the worker.
    pub fn ask(&mut self, job: Job) {
        self.outbox.push(job);
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.clock.unwrap_or_else(Utc::now)
    }

    /// Reveal requests waiting for the owner, from the last status report.
    pub fn pending_reveals(&self) -> u64 {
        self.overview
            .status
            .as_ref()
            .map_or(0, |s| s.pending_reveals)
    }
}

/// What `main` reads before the window opens: where the home is and its config, if any.
struct Boot {
    home: Home,
    config: Option<Config>,
    config_error: Option<String>,
}

static BOOT: OnceLock<Boot> = OnceLock::new();

impl Boot {
    /// One small file read, done in `main` before any window or frame exists.
    fn load() -> anyhow::Result<Boot> {
        let home = Home::resolve(None)?;
        let (config, config_error) = if home.config_path().exists() {
            match home.load_config() {
                Ok(c) => (Some(c), None),
                Err(e) => (None, Some(format!("{e:#}"))),
            }
        } else {
            (None, None)
        };
        Ok(Boot {
            home,
            config,
            config_error,
        })
    }
}

fn init() -> State {
    match BOOT.get() {
        Some(b) => {
            let mut s = State::new(b.home.clone(), b.config.clone());
            s.config_error = b.config_error.clone();
            s
        }
        // Nothing resolved (no HOME): a window that reads nothing.
        None => {
            let mut s = State::new(Home::new("~/distill"), None);
            s.fixture = true;
            s
        }
    }
}

/// Start the worker and ask for what the first screen shows. Scenes and headless runs skip it.
fn start(s: &mut State, waker: Waker) {
    if s.fixture || rune::is_headless() {
        return;
    }
    s.link = Some(jobs::spawn(Core::system(s.home.clone()), waker));
    nav::go(s, s.screen);
}

fn poll(s: &mut State, clock: f64, tx: &mut Tx) -> bool {
    let Some(link) = &s.link else { return false };
    let landed: Vec<_> = link.landed.try_iter().collect();
    let changed = !landed.is_empty();
    for land in landed {
        land(s);
    }
    let waits = [overview::refresh(s, clock), approvals::refresh(s, clock)];
    if let Some(wait) = waits.into_iter().flatten().reduce(f64::min) {
        tx.wake_after(Duration::from_secs_f64(wait));
    }
    if let Some(link) = &s.link {
        for job in s.outbox.drain(..) {
            let _ = link.jobs.send(job);
        }
    }
    if changed {
        tx.instant();
    }
    changed
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let mut layer = Vec::new();
    let entry = nav::entry(s.screen);
    let body = (entry.view)(s, &p, &mut layer)
        .key(format!("screen-{}", entry.id))
        .grow(1.0)
        .min_w(0.0)
        .h_full();
    let rule = el().w(p.hair).h_full().fill(p.t.separator).a11y_hidden();
    let window = row()
        .key("window")
        .w_full()
        .h_full()
        .fill(p.t.bg)
        .child(nav::sidebar(s, &p))
        .child(rule)
        .child(body);
    rune_kit::root(window, layer)
}

pub fn spec() -> AppSpec<State> {
    AppSpec {
        min_size: Some((860.0, 600.0)),
        scenes: fixture::SCENES,
        setup: Some(start),
        poll: Some(poll),
        ..AppSpec::new("DISTILL", (1080.0, 780.0), init, view)
    }
}

fn main() {
    rune::prewarm_gpu();
    // A headless run (`--snap`, `--script`) renders fixtures and reads no home.
    if !rune::is_headless() {
        // The Dock and app-switcher icon (`assets/icon.svg` is its source).
        if let Err(e) = rune::set_app_icon(include_bytes!("../assets/icon.png").as_slice()) {
            eprintln!("distill-gui: {e}");
        }
        match Boot::load() {
            Ok(b) => {
                let _ = BOOT.set(b);
            }
            Err(e) => eprintln!("distill-gui: {e:#}"),
        }
    }
    rune::run(spec());
}
