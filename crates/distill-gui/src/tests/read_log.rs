//! Read log: every read, refusal and step of a reveal, newest first, filtered by reader and day.
//! Driven against a temporary home where a trusted model reads and asks for reveals, an unknown
//! one is refused, and the owner answers in the window.

use distill_core::detect::Kind;
use distill_core::read::{self, Range};
use distill_core::reveal;
use distill_core::trust::Reader;

use super::home::*;
use super::*;
use crate::nav::Screen;

/// A home with a little of everything in its read log.
fn busy_home(dir: &std::path::Path) -> crate::jobs::Core {
    let core = home(dir);
    read::read(
        &core.home,
        &core.runtime,
        &cloud(),
        "claude-code/aaaa",
        Range::default(),
    )
    .unwrap();
    let stranger = Reader::model(Some("unknown-cloud"), Some("x"));
    read::read(
        &core.home,
        &core.runtime,
        &stranger,
        "claude-code/aaaa",
        Range::default(),
    )
    .unwrap();
    let a = file(
        &core,
        &cloud(),
        &[Kind::AnthropicKey],
        "rotate the deploy key",
    );
    file(
        &core,
        &cloud(),
        &[Kind::AssignedSecret],
        "check the db password",
    );
    reveal::approve(&core.home, &a.id).unwrap();
    reveal::collect(&core.home, &core.runtime, &cloud(), &a.id).unwrap();
    core
}

fn on_read_log(core: &crate::jobs::Core) -> Runtime<State> {
    let mut rt = window(core);
    press(&mut rt, "nav-read-log");
    assert_eq!(rt.state.screen, Screen::ReadLog);
    assert_eq!(run_jobs(&mut rt, core), ["read log"]);
    rt
}

fn rows(rt: &Runtime<State>) -> usize {
    rt.state.read_log.entries.as_ref().map_or(0, Vec::len)
}

fn rerun(rt: &mut Runtime<State>, core: &crate::jobs::Core, f: impl FnOnce(&mut State)) {
    rt.update(|s, _| f(s));
    press(rt, "apply");
    run_jobs(rt, core);
}

#[test]
fn the_read_log_lists_reads_refusals_and_reveal_activity() {
    let dir = tempfile::tempdir().unwrap();
    let core = busy_home(dir.path());
    let rt = on_read_log(&core);
    let all = everything(&rt);
    for want in [
        "example-cloud/m",
        "unknown-cloud/x",
        "claude-code/aaaa",
        "served 3 events",
        "refused: ",
        "requested: #",
        "approved by the owner",
        "collected: 1 value",
        "Reveal",
        "Refused",
        "Read",
    ] {
        assert!(all.contains(want), "{want:?} in {all}");
    }
    assert!(rt.find("entry-0").is_some());
    assert!(rt.find("edited").is_none(), "a whole chain says nothing");
    assert_no_value(&rt, "read log");
}

#[test]
fn an_edited_read_log_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let core = busy_home(dir.path());
    let path = core.home.read_log_path();
    let log = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<&str> = log.lines().collect();
    lines.remove(1);
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    let rt = on_read_log(&core);
    let all = everything(&rt);
    assert!(
        rt.find("edited").is_some() && all.contains("The read log has been edited: line 2"),
        "{all}"
    );
}

#[test]
fn the_read_log_is_filtered_by_reader_and_day() {
    let dir = tempfile::tempdir().unwrap();
    let core = busy_home(dir.path());
    let mut rt = on_read_log(&core);
    let everyone = rows(&rt);
    assert!(everyone >= 6, "{everyone}");

    rerun(&mut rt, &core, |s| {
        s.read_log.reader.replace_text("unknown-cloud")
    });
    assert_eq!(rows(&rt), 1);
    assert!(!shows(&rt, "example-cloud/m"), "{:?}", texts(&rt));

    let today = chrono::Utc::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    rerun(&mut rt, &core, |s| {
        s.read_log.reader.replace_text("");
        s.read_log.since.replace_text(&today);
        s.read_log.until.replace_text(&today);
    });
    assert_eq!(rows(&rt), everyone);
    rerun(&mut rt, &core, |s| {
        s.read_log.since.replace_text("");
        s.read_log.until.replace_text("2026-01-01");
    });
    assert_eq!(rows(&rt), 0);
    assert!(shows(&rt, "No reads match"), "{:?}", texts(&rt));

    rt.update(|s, _| s.read_log.until.replace_text("yesterday"));
    press(&mut rt, "apply");
    assert!(run_jobs(&mut rt, &core).is_empty(), "nothing is asked");
    assert!(shows(&rt, "Write dates as YYYY-MM-DD"), "{:?}", texts(&rt));
}

#[test]
fn approving_and_denying_in_the_window_show_in_the_read_log_and_no_screen_shows_a_value() {
    let dir = tempfile::tempdir().unwrap();
    let core = home(dir.path());
    let key = file(
        &core,
        &cloud(),
        &[Kind::AnthropicKey],
        "rotate the deploy key",
    );
    let pass = file(
        &core,
        &cloud(),
        &[Kind::AssignedSecret],
        "check the db password",
    );
    let mut rt = window(&core);
    press(&mut rt, "nav-approvals");
    run_jobs(&mut rt, &core);
    press(&mut rt, &format!("approve-{}", key.id));
    press(&mut rt, &format!("confirm-{}", key.id));
    run_jobs(&mut rt, &core);
    press(&mut rt, &format!("deny-{}", pass.id));
    run_jobs(&mut rt, &core);
    reveal::collect(&core.home, &core.runtime, &cloud(), &key.id).unwrap();
    assert_no_value(&rt, "approvals");

    press(&mut rt, "nav-read-log");
    run_jobs(&mut rt, &core);
    let all = everything(&rt);
    assert!(
        all.contains(&format!("reveal {} approved by the owner", key.id)),
        "{all}"
    );
    assert!(
        all.contains(&format!("reveal {} denied by the owner", pass.id)),
        "{all}"
    );
    assert!(
        all.contains(&format!("reveal {} collected: 1 value", key.id)),
        "{all}"
    );
    assert_no_value(&rt, "read log");

    press(&mut rt, "nav-findings");
    run_jobs(&mut rt, &core);
    press(&mut rt, "finding-0");
    run_jobs(&mut rt, &core);
    assert_no_value(&rt, "findings");
    press(&mut rt, "nav-approvals");
    run_jobs(&mut rt, &core);
    assert_no_value(&rt, "approvals again");
}
