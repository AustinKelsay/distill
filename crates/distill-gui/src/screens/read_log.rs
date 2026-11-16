//! Read log: every read, refusal and reveal in the read log (#87).
//! Not built yet; this module is where it goes (its state, `open` and `view`).

use rune::El;

use crate::State;
use crate::ui::Paint;

#[derive(Default)]
pub struct ReadLogState;

pub fn open(_: &mut State) {}

pub fn view(_: &State, p: &Paint, _: &mut Vec<El>) -> El {
    super::placeholder::view(
        p,
        "Read log",
        "Every read of session text goes here: who read, which sessions and when.",
    )
}
