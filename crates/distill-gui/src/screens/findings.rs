//! Findings: the rotation list (#86).
//! Not built yet; this module is where it goes (its state, `open` and `view`).

use rune::El;

use crate::State;
use crate::ui::Paint;

#[derive(Default)]
pub struct FindingsState;

pub fn open(_: &mut State) {}

pub fn view(_: &State, p: &Paint, _: &mut Vec<El>) -> El {
    super::placeholder::view(
        p,
        "Findings",
        "The rotation list goes here: each secret by kind, how many sessions hold it, and when it was first and last seen.",
    )
}
