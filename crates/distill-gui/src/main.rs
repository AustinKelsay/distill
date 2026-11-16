//! Distill's window. For now a shell that exposes nothing: the previous build's setup and
//! dashboard screens are gone, and the spec's screens (Setup, Overview, Sessions, Findings,
//! Approvals, Read log) arrive with their tickets.
//!
//! The window holds no logic. It will read and write only through distill-core, off the frame
//! loop. This is the only crate that depends on Rune (ADR 0007).
//!
//! `cargo run -p distill-gui` opens the window; `-- --snap out.png --scene shell` renders a scene
//! headless (`--list-scenes` lists them).

mod ui;

#[cfg(test)]
mod tests;

use std::sync::OnceLock;

use distill_core::Home;
use rune::{Align, AppSpec, Cx, El, col, text};

use crate::ui::Paint;

pub struct State {
    pub home: Home,
}

impl State {
    pub fn new(home: Home) -> State {
        State { home }
    }
}

static HOME: OnceLock<Home> = OnceLock::new();

fn init() -> State {
    State::new(
        HOME.get()
            .cloned()
            .unwrap_or_else(|| Home::new("~/distill")),
    )
}

fn view(s: &State, cx: &mut Cx<State>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    col()
        .key("window")
        .w_full()
        .h_full()
        .fill(p.t.bg)
        .center()
        .gap(8.0)
        .child(text("Distill").key("title").font(p.title()).color(p.ink()))
        .child(
            text("Distill is being rebuilt. Its screens arrive with the tickets that build them.")
                .key("note")
                .font(p.text())
                .color(p.muted())
                .wrap()
                .max_w(520.0)
                .align(Align::Center),
        )
        .child(
            text(s.home.root.display().to_string())
                .key("home")
                .font(p.caption())
                .color(p.muted()),
        )
}

/// A named state for `--snap --scene`.
type Scene = (&'static str, fn(&mut State));

fn shell(s: &mut State) {
    s.home = Home::new("~/distill");
}

const SCENES: &[Scene] = &[("shell", shell)];

pub fn spec() -> AppSpec<State> {
    AppSpec {
        min_size: Some((820.0, 600.0)),
        scenes: SCENES,
        ..AppSpec::new("Distill", (1040.0, 760.0), init, view)
    }
}

fn main() {
    rune::prewarm_gpu();
    match Home::resolve(None) {
        Ok(h) => {
            let _ = HOME.set(h);
        }
        Err(e) => eprintln!("distill-gui: {e:#}"),
    }
    rune::run(spec());
}
