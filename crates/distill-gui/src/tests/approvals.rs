//! Approvals: the owner answers reveal requests. Driven end to end against a temporary home: a
//! trusted model files requests through core, the owner approves or denies them in the window,
//! and the model collects (or is refused) through core again.

use distill_core::detect::Kind;
use distill_core::readlog::{self, ReadResult};
use distill_core::reveal::{self, Collected, State as Asked};

use super::home::*;
use super::*;
use crate::nav::Screen;

fn on_approvals(core: &crate::jobs::Core) -> Runtime<State> {
    let mut rt = window(core);
    press(&mut rt, "open-approvals");
    assert_eq!(rt.state.screen, Screen::Approvals);
    assert_eq!(run_jobs(&mut rt, core), ["reveals"]);
    rt
}

fn state_of(core: &crate::jobs::Core, id: &str) -> Asked {
    reveal::list(&core.home)
        .unwrap()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap()
        .state
}

#[test]
fn a_pending_request_shows_who_asks_for_what_where_why_and_the_time_left() {
    let dir = tempfile::tempdir().unwrap();
    let core = home(dir.path());
    let r = file(
        &core,
        &cloud(),
        &[Kind::AnthropicKey],
        "rotate the deploy key",
    );
    let rt = on_approvals(&core);
    let card = format!("request-{}", r.id);
    assert!(rt.find(&card).is_some(), "{:?}", texts(&rt));
    let all = everything(&rt);
    assert!(all.contains("example-cloud/m"), "{all}");
    assert!(all.contains("anthropic key"), "{all}");
    assert!(
        all.contains(&format!("#{}", short(&core, Kind::AnthropicKey))),
        "{all}"
    );
    assert!(all.contains("claude-code/aaaa"), "{all}");
    assert!(all.contains("rotate the deploy key"), "{all}");
    assert!(
        all.contains("23 h 59 min left") || all.contains("24 h left"),
        "{all}"
    );
    assert_no_value(&rt, "approvals");
}

#[test]
fn approving_releases_only_the_values_that_request_names_once() {
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
    let mut rt = on_approvals(&core);
    assert!(shows(&rt, "Approvals, 2 waiting"), "{:?}", texts(&rt));

    // The first press asks again, naming what will be released; nothing is approved yet.
    press(&mut rt, &format!("approve-{}", key.id));
    assert!(run_jobs(&mut rt, &core).is_empty());
    assert!(
        shows(&rt, "Release 1 secret to example-cloud/m once?"),
        "{:?}",
        texts(&rt)
    );
    assert_eq!(state_of(&core, &key.id), Asked::Pending);
    press(&mut rt, &format!("confirm-{}", key.id));
    assert_eq!(run_jobs(&mut rt, &core), ["approve reveal", "reveals"]);
    assert!(matches!(state_of(&core, &key.id), Asked::Approved { .. }));
    assert_eq!(
        state_of(&core, &pass.id),
        Asked::Pending,
        "only that request"
    );
    assert!(rt.find(&format!("request-{}", key.id)).is_none());
    assert!(rt.find(&format!("request-{}", pass.id)).is_some());
    assert!(shows(&rt, "Approvals, 1 waiting"), "{:?}", texts(&rt));
    assert_no_value(&rt, "approvals after approving");

    // The model collects the key and nothing else, once.
    let Collected::Released(got) =
        reveal::collect(&core.home, &core.runtime, &cloud(), &key.id).unwrap()
    else {
        panic!("not released");
    };
    let values: Vec<&str> = got.values.iter().map(|v| v.value.as_str()).collect();
    assert_eq!(values, [fake_key().as_str()]);
    assert!(matches!(
        reveal::collect(&core.home, &core.runtime, &cloud(), &key.id).unwrap(),
        Collected::Refused(_)
    ));
    assert!(matches!(
        reveal::collect(&core.home, &core.runtime, &cloud(), &pass.id).unwrap(),
        Collected::Refused(_)
    ));
    assert_no_value(&rt, "approvals after collecting");
}

#[test]
fn denying_answers_one_request_and_the_model_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let core = home(dir.path());
    let pass = file(
        &core,
        &cloud(),
        &[Kind::AssignedSecret],
        "check the db password",
    );
    let key = file(
        &core,
        &cloud(),
        &[Kind::AnthropicKey],
        "rotate the deploy key",
    );
    let mut rt = on_approvals(&core);
    press(&mut rt, &format!("deny-{}", pass.id));
    assert_eq!(run_jobs(&mut rt, &core), ["deny reveal", "reveals"]);
    assert!(matches!(state_of(&core, &pass.id), Asked::Denied { .. }));
    assert_eq!(state_of(&core, &key.id), Asked::Pending);
    assert!(rt.find(&format!("request-{}", pass.id)).is_none());
    assert!(shows(&rt, "Denied"), "{:?}", texts(&rt));
    let Collected::Refused(why) =
        reveal::collect(&core.home, &core.runtime, &cloud(), &pass.id).unwrap()
    else {
        panic!("released");
    };
    assert!(why.contains("denied"), "{why}");
    let log = readlog::list(&core.home, None, 50).unwrap();
    assert!(log.iter().any(|e| e.result
        == ReadResult::RevealDenied {
            request: pass.id.clone()
        }));
    assert_no_value(&rt, "approvals after denying");
}

#[test]
fn a_request_answered_elsewhere_says_so_and_leaves_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let core = home(dir.path());
    let r = file(
        &core,
        &cloud(),
        &[Kind::AnthropicKey],
        "rotate the deploy key",
    );
    let mut rt = on_approvals(&core);
    reveal::deny(&core.home, &r.id).unwrap();
    press(&mut rt, &format!("approve-{}", r.id));
    press(&mut rt, &format!("confirm-{}", r.id));
    run_jobs(&mut rt, &core);
    assert!(shows(&rt, "can no longer be approved"), "{:?}", texts(&rt));
    assert!(rt.find(&format!("request-{}", r.id)).is_none());
    assert!(matches!(state_of(&core, &r.id), Asked::Denied { .. }));
}

#[test]
fn with_nothing_waiting_the_screen_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let core = home(dir.path());
    let mut rt = window(&core);
    press(&mut rt, "nav-approvals");
    run_jobs(&mut rt, &core);
    assert!(
        shows(&rt, "No reveal requests are waiting"),
        "{:?}",
        texts(&rt)
    );
}
