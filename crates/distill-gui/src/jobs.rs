//! Everything the window asks of distill-core runs as a [`Job`] on one worker thread, so no file
//! I/O and no call to the local runtime happens in the frame loop. A screen queues a job with
//! [`State::ask`]; `poll` hands queued jobs to the worker and lands each result on the state.
//!
//! A job is a closure over [`Core`], what it may reach: the home, the local runtime and the
//! timer installer. Each screen writes its own jobs in its own module, so adding a screen
//! touches no shared list of requests. Tests run queued jobs in place with [`run_queued`]
//! over a temporary home, the fake runtime and a stand-in installer.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use distill_core::runtime::LocalRuntime;
use distill_core::{Config, Home};
use rune::Waker;

use crate::State;

/// The timer that runs `distill tick`. In the window, [`distill_core::setup::install_timer`]
/// and [`distill_core::schedule::is_installed`]; tests use stand-ins, because installing replaces
/// the live `dev.distill.tick` job whatever `HOME` is.
#[derive(Clone)]
pub struct Timer {
    pub install: Arc<Install>,
    pub installed: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// Installs the timer for a home and config, returning where it was installed.
pub type Install = dyn Fn(&Home, &Config) -> anyhow::Result<PathBuf> + Send + Sync;

/// What a job may reach.
#[derive(Clone)]
pub struct Core {
    pub home: Home,
    pub runtime: LocalRuntime,
    pub timer: Timer,
}

impl Core {
    /// The window's own: Ollama on this machine and the real timer.
    pub fn system(home: Home) -> Core {
        Core {
            home,
            runtime: LocalRuntime::ollama(),
            timer: Timer {
                install: Arc::new(distill_core::setup::install_timer),
                installed: Arc::new(distill_core::schedule::is_installed),
            },
        }
    }
}

/// What a finished job does to the state, back on the UI thread.
pub type Land = Box<dyn FnOnce(&mut State) + Send>;

/// One piece of work for the worker, named so tests can see what was asked.
pub struct Job {
    pub what: &'static str,
    run: Box<dyn FnOnce(&Core) -> Land + Send>,
}

impl Job {
    pub fn new(what: &'static str, run: impl FnOnce(&Core) -> Land + Send + 'static) -> Job {
        Job {
            what,
            run: Box::new(run),
        }
    }

    pub fn run(self, core: &Core) -> Land {
        (self.run)(core)
    }
}

/// The two ends of the worker's channels, held in state while a window runs.
pub struct Link {
    pub jobs: Sender<Job>,
    pub landed: Receiver<Land>,
}

/// Start the worker. Each result wakes the window so `poll` lands it.
pub fn spawn(core: Core, waker: Waker) -> Link {
    let (jobs, job_rx) = channel::<Job>();
    let (land_tx, landed) = channel::<Land>();
    thread::Builder::new()
        .name("distill-gui-worker".into())
        .spawn(move || {
            for job in job_rx {
                if land_tx.send(job.run(&core)).is_err() {
                    break;
                }
                waker();
            }
        })
        .expect("spawn the worker thread");
    Link { jobs, landed }
}

/// Run every queued job in place, and the jobs their results queue, until none is left.
#[cfg(test)]
pub fn run_queued(s: &mut State, core: &Core) -> Vec<&'static str> {
    let mut ran = Vec::new();
    while !s.outbox.is_empty() {
        for job in std::mem::take(&mut s.outbox) {
            ran.push(job.what);
            job.run(core)(s);
        }
    }
    ran
}

/// An error as one line for the screen.
pub fn line(e: anyhow::Error) -> String {
    format!("{e:#}")
}
