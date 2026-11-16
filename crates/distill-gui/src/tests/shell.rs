//! The window's frame: one window, navigation between the spec's six screens.

use super::*;
use crate::fixture;
use crate::nav::{SCREENS, Screen};

#[test]
fn an_unconfigured_home_opens_on_setup_and_a_configured_one_on_the_overview() {
    let rt = runtime(fixture::fresh());
    assert_eq!(rt.state.screen, Screen::Setup);
    let rt = runtime(fixture::running());
    assert_eq!(rt.state.screen, Screen::Overview);
}

#[test]
fn the_navigation_lists_the_six_screens_and_moves_between_them() {
    let names: Vec<&str> = SCREENS.iter().map(|e| e.label).collect();
    assert_eq!(
        names,
        [
            "Setup",
            "Overview",
            "Sessions",
            "Findings",
            "Approvals",
            "Read log"
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
