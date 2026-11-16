//! Headless runs of the real view: no window, no disk.

use distill_core::Home;
use rune::{Env, Runtime};

use crate::{State, view};

#[test]
fn the_shell_window_shows_distill_and_its_home() {
    let mut rt = Runtime::new(
        State::new(Home::new("/tmp/some-home")),
        view,
        (1040.0, 760.0),
        Env::new(true, 1.0),
    );
    rt.frame(0.0);
    let texts: Vec<String> = rt
        .elements()
        .into_iter()
        .filter_map(|e| e.text.or(e.label))
        .collect();
    assert!(texts.iter().any(|t| t == "Distill"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t.contains("/tmp/some-home")),
        "{texts:?}"
    );
    assert!(rt.find("title").is_some());
    // Nothing of the previous build's screens is left.
    for gone in ["Settings", "Approve", "Needs you"] {
        assert!(
            !texts.iter().any(|t| t.contains(gone)),
            "{gone} in {texts:?}"
        );
    }
}
