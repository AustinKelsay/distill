//! Headless runs of the real view. Scenes and most tests use fixture state and touch no disk;
//! the tests that run the window's jobs do so against a temporary home, the fake local runtime
//! and a stand-in for the timer installer, so no test reaches a real model or the real timer.

mod approvals;
mod findings;
mod home;
mod overview;
mod read_log;
mod sessions;
mod setup;
mod shell;
mod snapshots;

use rune::{Env, Runtime};

use crate::{State, view};

pub const SIZE: (f32, f32) = (1040.0, 760.0);

pub fn runtime(s: State) -> Runtime<State> {
    runtime_sized(s, SIZE)
}

pub fn runtime_sized(s: State, size: (f32, f32)) -> Runtime<State> {
    let mut rt = Runtime::new(s, view, size, Env::new(true, 1.0));
    settle(&mut rt);
    rt
}

pub fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.0);
    for _ in 0..20 {
        rt.frame(1.0 / 60.0);
    }
}

pub fn press(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = rt
        .find(key)
        .unwrap_or_else(|| panic!("no element {key:?} on screen: {:?}", texts(rt)));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

pub fn texts(rt: &Runtime<State>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.text.or(e.label))
        .collect()
}

pub fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    texts(rt).iter().any(|t| t.contains(needle))
}

/// Run every queued job against `core`, as the worker would, and redraw.
pub fn run_jobs(rt: &mut Runtime<State>, core: &crate::jobs::Core) -> Vec<&'static str> {
    let mut ran = Vec::new();
    rt.update(|s, _| ran = crate::jobs::run_queued(s, core));
    settle(rt);
    ran
}
