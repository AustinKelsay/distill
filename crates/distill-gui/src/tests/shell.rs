//! The window's frame: one window, navigation between its screens.

use super::*;
use crate::fixture;
use crate::nav::{SCREENS, Screen};

#[test]
fn an_unconfigured_home_opens_on_the_guide_and_a_configured_one_on_the_overview() {
    let s = State::new(distill_core::Home::new("~/distill"), None);
    assert_eq!(s.screen, Screen::Guide);
    let rt = runtime(fixture::running());
    assert_eq!(rt.state.screen, Screen::Overview);
}

#[test]
fn the_navigation_lists_the_screens_and_moves_between_them() {
    let names: Vec<&str> = SCREENS.iter().map(|e| e.label).collect();
    assert_eq!(
        names,
        [
            "Setup",
            "Overview",
            "Sessions",
            "Findings",
            "Approvals",
            "Read log",
            "Guide"
        ]
    );
    let mut rt = runtime(fixture::running());
    for entry in SCREENS {
        press(&mut rt, &format!("nav-{}", entry.id));
        assert_eq!(rt.state.screen, entry.screen);
        assert!(
            rt.find(&format!("screen-{}", entry.id)).is_some(),
            "{} shows its screen",
            entry.label
        );
    }
}

#[test]
fn every_screen_is_built() {
    let mut rt = runtime(fixture::running());
    for entry in SCREENS {
        press(&mut rt, &format!("nav-{}", entry.id));
        assert!(
            !shows(&rt, "Not yet available"),
            "{}: {:?}",
            entry.label,
            texts(&rt)
        );
    }
}

#[test]
fn approvals_in_the_navigation_counts_the_reveal_requests_waiting() {
    let rt = runtime(fixture::running());
    let badge = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("nav-approvals"))
        .and_then(|e| e.label)
        .unwrap();
    assert_eq!(badge, "Approvals, 2 waiting");
}

#[test]
fn the_guide_starts_setup() {
    let mut rt = runtime(fixture::fresh());
    press(&mut rt, "nav-guide");
    assert!(shows(&rt, "How it works") && shows(&rt, "Start setup"));
    press(&mut rt, "screen-guide/start");
    assert_eq!(rt.state.screen, Screen::Setup);

    // Once set up, it offers to open Setup instead.
    let mut rt = runtime(fixture::running());
    press(&mut rt, "nav-guide");
    assert!(shows(&rt, "Open Setup"));
}
