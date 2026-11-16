//! Sessions: search and read sessions, findings covered until clicked (#86).
//! Not built yet; this module is where it goes (its state, `open` and `view`).

use rune::El;

use crate::State;
use crate::ui::Paint;

#[derive(Default)]
pub struct SessionsState;

pub fn open(_: &mut State) {}

pub fn view(_: &State, p: &Paint, _: &mut Vec<El>) -> El {
    super::placeholder::view(
        p,
        "Sessions",
        "Search and open your sessions here, with every finding covered until you click it.",
    )
}
