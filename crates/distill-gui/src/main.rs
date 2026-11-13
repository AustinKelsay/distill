//! Distill's window: a one-time Setup flow that writes `config.toml`, and a read-only Dashboard
//! of the archive with approve buttons for datasets waiting to leave the machine.
//!
//! The window holds no business logic. It reads and writes only through distill-core, on a
//! worker thread (`data.rs`), so no file I/O runs in the frame loop.
//!
//! `cargo run -p distill-gui` opens the window; `-- --snap out.png --scene dashboard` renders a
//! fixture headless (`--list-scenes` lists them).

mod chart;
mod dashboard;
mod data;
mod fixture;
mod setup;
mod ui;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::sync::OnceLock;
use std::time::Duration;

use distill_core::Home;
use distill_core::config::Config;
use distill_core::source::Detection;
use distill_core::stats::Dashboard;
use rune::{AppSpec, Cx, El, Tx, Waker, col};

use crate::data::{Boot, Link, Reply, Request};
use crate::setup::SetupDraft;
use crate::ui::Paint;

/// Seconds between dashboard refreshes while it shows.
const REFRESH_SECS: f64 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Setup,
    Dashboard,
}

/// What the dashboard shows and is doing.
#[derive(Default)]
pub struct DashState {
    pub data: Option<Dashboard>,
    pub error: Option<String>,
    pub loading: bool,
    /// (recipe, build) pairs whose approval is on its way to disk.
    pub approving: BTreeSet<(String, String)>,
    /// A one-line result of the last action ("Approved …", "Saved settings").
    pub notice: Option<String>,
}

pub struct State {
    pub screen: Screen,
    pub home: Home,
    /// The config as last saved (or the defaults before setup has run).
    pub config: Config,
    pub configured: bool,
    pub setup: SetupDraft,
    pub dash: DashState,
    /// Per source, what setup's detection found; None until the worker answers.
    pub detections: Option<Vec<Detection>>,
    pub schedule_installed: Option<bool>,
    /// Requests waiting for the worker. `poll` sends them; tests read them.
    pub outbox: Vec<Request>,
    pub link: Option<Link>,
    /// Scenes and tests install data directly; nothing is loaded from disk.
    pub fixture: bool,
    next_refresh: f64,
}

impl State {
    /// A window over `home` with `config` (None: setup has not run, so it opens on Setup).
    pub fn new(home: Home, config: Option<Config>) -> State {
        let configured = config.is_some();
        let config = config.unwrap_or_default();
        let setup = SetupDraft::from_config(&config, configured);
        State {
            screen: if configured {
                Screen::Dashboard
            } else {
                Screen::Setup
            },
            home,
            config,
            configured,
            setup,
            dash: DashState {
                loading: configured,
                ..DashState::default()
            },
            detections: None,
            schedule_installed: None,
            outbox: Vec::new(),
            link: None,
            fixture: false,
            next_refresh: 0.0,
        }
    }

    /// Queue work for the worker.
    pub fn request(&mut self, r: Request) {
        if !self.outbox.contains(&r) {
            self.outbox.push(r);
        }
    }

    /// Open the setup flow over the current config (the dashboard's Settings button).
    pub fn open_settings(&mut self) {
        self.setup = SetupDraft::from_config(&self.config, self.configured);
        self.screen = Screen::Setup;
        if self.detections.is_none() && !self.fixture {
            self.request(data::detect_request(&self.config));
        }
    }

    /// Setup's Finish: save the draft and show the dashboard.
    pub fn finish_setup(&mut self) {
        let config = self.setup.finish();
        self.config = config.clone();
        self.configured = true;
        self.request(Request::SaveConfig(Box::new(config)));
        self.screen = Screen::Dashboard;
        self.dash.notice = Some("Saving settings…".to_string());
    }

    pub fn approve(&mut self, recipe: String, build: String) {
        if self.dash.approving.insert((recipe.clone(), build.clone())) {
            self.request(Request::Approve { recipe, build });
        }
    }

    fn land(&mut self, reply: Reply) {
        match reply {
            Reply::Dashboard(Ok(d)) => {
                self.dash.data = Some(*d);
                self.dash.error = None;
                self.dash.loading = false;
            }
            Reply::Dashboard(Err(e)) => {
                self.dash.error = Some(e);
                self.dash.loading = false;
            }
            Reply::Detected(found) => self.detections = Some(found),
            Reply::Saved(Ok(())) => {
                self.dash.notice = Some("Settings saved".to_string());
                self.dash.loading = self.dash.data.is_none();
                self.request(Request::LoadDashboard);
                self.request(Request::CheckSchedule);
            }
            Reply::Saved(Err(e)) => {
                self.dash.notice = Some(format!("Could not save settings: {e}"))
            }
            Reply::Approved {
                recipe,
                build,
                result,
            } => {
                self.dash.approving.remove(&(recipe.clone(), build.clone()));
                self.dash.notice = Some(match result {
                    Ok(()) => format!("Approved \"{recipe}\" build {build}"),
                    Err(e) => format!("Could not approve \"{recipe}\": {e}"),
                });
                self.request(Request::LoadDashboard);
            }
            Reply::Schedule(on) => self.schedule_installed = Some(on),
        }
    }
}

static BOOT: OnceLock<Boot> = OnceLock::new();

fn init() -> State {
    match BOOT.get() {
        Some(b) => {
            let mut s = State::new(b.home.clone(), b.config.clone());
            if let Some(e) = &b.config_error {
                s.dash.error = Some(e.clone());
            }
            s
        }
        None => State::new(Home::new("~/distill"), None),
    }
}

/// Start the worker and ask for what the first screens show. Scenes skip it.
fn start(s: &mut State, waker: Waker) {
    if s.fixture {
        return;
    }
    s.link = Some(data::spawn(s.home.clone(), waker));
    s.request(data::detect_request(&s.config));
    s.request(Request::CheckSchedule);
    if s.configured {
        s.request(Request::LoadDashboard);
    }
}

fn poll(s: &mut State, clock: f64, tx: &mut Tx) -> bool {
    let Some(link) = &s.link else { return false };
    let replies: Vec<Reply> = link.replies.try_iter().collect();
    let changed = !replies.is_empty();
    for reply in replies {
        s.land(reply);
    }
    if s.screen == Screen::Dashboard && s.configured {
        if clock >= s.next_refresh {
            s.request(Request::LoadDashboard);
            s.next_refresh = clock + REFRESH_SECS;
        }
        tx.wake_after(Duration::from_secs_f64((s.next_refresh - clock).max(0.5)));
    }
    if let Some(link) = &s.link {
        for r in s.outbox.drain(..) {
            let _ = link.jobs.send(r);
        }
    }
    if changed {
        tx.instant();
    }
    changed
}

fn view(s: &State, cx: &mut Cx<State>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let body = match s.screen {
        Screen::Setup => setup::view(s, &p),
        Screen::Dashboard => dashboard::view(s, &p),
    };
    col()
        .key("window")
        .w_full()
        .h_full()
        .fill(p.t.bg)
        .child(body)
}

/// A named fixture state for `--snap --scene`.
type Scene = (&'static str, fn(&mut State));

const SCENES: &[Scene] = &[
    ("setup-sources", fixture::setup_sources),
    ("setup-agent", fixture::setup_agent),
    ("setup-privacy", fixture::setup_privacy),
    ("setup-schedule", fixture::setup_schedule),
    ("dashboard", fixture::dashboard),
    ("dashboard-empty", fixture::dashboard_empty),
];

pub fn spec() -> AppSpec<State> {
    AppSpec {
        min_size: Some((820.0, 600.0)),
        scenes: SCENES,
        setup: Some(start),
        poll: Some(poll),
        ..AppSpec::new("Distill", (1040.0, 760.0), init, view)
    }
}

fn main() {
    rune::prewarm_gpu();
    match Boot::load() {
        Ok(b) => {
            let _ = BOOT.set(b);
        }
        Err(e) => eprintln!("distill-gui: {e:#}"),
    }
    rune::run(spec());
}
