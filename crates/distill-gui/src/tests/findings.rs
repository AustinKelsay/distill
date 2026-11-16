//! Findings: the rotation list, with the findings command's grouping and filters, and the
//! sessions each fingerprint appears in, opened in the Sessions screen. Driven against a
//! temporary home whose sessions hold fake secrets.

use distill_core::detect::Kind;
use distill_core::read::{Kinds, Order};
use distill_core::readlog::{self, ReadResult};
use distill_core::trust::Reader;

use super::home::*;
use super::*;
use crate::nav::Screen;

fn on_findings(core: &crate::jobs::Core) -> Runtime<State> {
    let mut rt = window(core);
    press(&mut rt, "nav-findings");
    assert_eq!(rt.state.screen, Screen::Findings);
    assert_eq!(run_jobs(&mut rt, core), ["findings"]);
    rt
}

/// The short fingerprints listed, in order.
fn listed(rt: &Runtime<State>) -> Vec<String> {
    rt.state
        .findings
        .list
        .as_ref()
        .map(|l| {
            l.rows
                .iter()
                .map(|r| r.fingerprint.short().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn rerun(rt: &mut Runtime<State>, core: &crate::jobs::Core, f: impl FnOnce(&mut State)) {
    rt.update(|s, _| f(s));
    press(rt, "apply");
    run_jobs(rt, core);
}

#[test]
fn the_rotation_list_groups_secrets_by_fingerprint_most_sessions_first() {
    let dir = tempfile::tempdir().unwrap();
    let core = home(dir.path());
    let rt = on_findings(&core);
    let key = short(&core, Kind::AnthropicKey);
    let pass = short(&core, Kind::AssignedSecret);
    assert_eq!(listed(&rt), [key.clone(), pass.clone()]);
    let all = everything(&rt);
    assert!(all.contains(&format!("#{key}")), "{all}");
    assert!(all.contains("‹anthropic key›"), "{all}");
    assert!(all.contains("2 sessions"), "{all}");
    assert!(all.contains("Claude Code"), "{all}");
    assert!(all.contains("First seen Nov 10, 2026"), "{all}");
    assert!(all.contains("last seen Nov 12, 2026"), "{all}");
    assert!(all.contains("2 fingerprints"), "{all}");
    assert!(!all.contains("‹email›"), "secrets only by default: {all}");
    assert_no_value(&rt, "findings");
    let log = readlog::list(&core.home, Some("owner"), 50).unwrap();
    assert!(
        log.iter().any(|e| e.reader == Reader::Owner
            && e.result
                == ReadResult::Findings {
                    fingerprints: 2,
                    sessions: 0
                }),
        "{log:?}"
    );
}

#[test]
fn the_filters_are_the_findings_commands() {
    let dir = tempfile::tempdir().unwrap();
    let core = home(dir.path());
    let mut rt = on_findings(&core);
    for key in [
        "filter-kind-trigger",
        "filter-harness-trigger",
        "filter-since",
        "filter-until",
        "filter-order-trigger",
    ] {
        assert!(rt.find(key).is_some(), "no {key}: {:?}", texts(&rt));
    }
    let (key, pass, email) = (
        short(&core, Kind::AnthropicKey),
        short(&core, Kind::AssignedSecret),
        short(&core, Kind::Email),
    );
    rerun(&mut rt, &core, |s| {
        s.findings.filters.kinds = Kinds::PersonalDetails
    });
    assert_eq!(listed(&rt), [email]);
    assert!(shows(&rt, "‹email›"));
    rerun(&mut rt, &core, |s| {
        s.findings.filters.kinds = Kinds::Only(Kind::AssignedSecret)
    });
    assert_eq!(listed(&rt), [pass]);
    rerun(&mut rt, &core, |s| s.findings.filters.kinds = Kinds::All);
    assert_eq!(listed(&rt).len(), 3);
    rerun(&mut rt, &core, |s| {
        s.findings.filters.kinds = Kinds::Secrets;
        s.findings.filters.since.replace_text("2026-11-11");
    });
    assert_eq!(listed(&rt), [key], "only the later session");
    rerun(&mut rt, &core, |s| {
        s.findings.filters.since.replace_text("");
        s.findings.filters.until.replace_text("2026-11-10");
        s.findings.filters.order = Order::Recent;
    });
    assert_eq!(listed(&rt).len(), 2);
    rerun(&mut rt, &core, |s| {
        s.findings.filters.until.replace_text("");
        s.findings.filters.harness = Some(distill_core::Harness::Codex);
    });
    assert!(listed(&rt).is_empty());
    assert!(shows(&rt, "No fingerprints match"), "{:?}", texts(&rt));

    rt.update(|s, _| s.findings.filters.since.replace_text("last week"));
    press(&mut rt, "apply");
    assert!(run_jobs(&mut rt, &core).is_empty(), "nothing is asked");
    assert!(shows(&rt, "Write dates as YYYY-MM-DD"), "{:?}", texts(&rt));
    assert_no_value(&rt, "findings filtered");
}

#[test]
fn a_fingerprint_lists_its_sessions_and_opens_one_in_the_sessions_screen() {
    let dir = tempfile::tempdir().unwrap();
    let core = home(dir.path());
    let mut rt = on_findings(&core);
    press(&mut rt, "finding-0");
    assert_eq!(run_jobs(&mut rt, &core), ["where found"]);
    let all = everything(&rt);
    assert!(all.contains("claude-code/aaaa"), "{all}");
    assert!(all.contains("claude-code/bbbb"), "{all}");
    assert!(all.contains("/work/ops"), "{all}");
    assert!(all.contains("event 0"), "{all}");
    assert_no_value(&rt, "where found");

    // Newest first: bbbb, then aaaa.
    press(&mut rt, "open-where-1");
    assert_eq!(run_jobs(&mut rt, &core), ["read session"]);
    assert_eq!(rt.state.screen, Screen::Sessions);
    let open = rt.state.sessions.open.as_ref().expect("a session open");
    assert_eq!(open.key, "claude-code/aaaa");
    let all = everything(&rt);
    assert!(
        all.contains("deploy the app with ‹anthropic key› please"),
        "{all}"
    );
    assert_no_value(&rt, "the session opened from findings");
    assert!(
        readlog::list(&core.home, Some("owner"), 50)
            .unwrap()
            .iter()
            .any(|e| e.session.as_deref() == Some("claude-code/aaaa")
                && matches!(e.result, ReadResult::Served { .. }))
    );

    // Back returns to the findings, still showing where it was found.
    press(&mut rt, "back");
    assert_eq!(rt.state.screen, Screen::Findings);
    run_jobs(&mut rt, &core);
    assert!(shows(&rt, "claude-code/bbbb"), "{:?}", texts(&rt));
    assert!(rt.state.sessions.open.is_none());
}
