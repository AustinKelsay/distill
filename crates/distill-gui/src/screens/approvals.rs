//! Approvals: pending reveal requests, approved or denied only here (#87).
//! Not built yet; this module is where it goes (its state, `open` and `view`).

use rune::El;

use crate::State;
use crate::ui::Paint;

#[derive(Default)]
pub struct ApprovalsState;

pub fn open(_: &mut State) {}

pub fn view(_: &State, p: &Paint, _: &mut Vec<El>) -> El {
    super::placeholder::view(
        p,
        "Approvals",
        "Reveal requests wait here for you to approve or deny: who asks, for which secret, in which session and why.",
    )
}
