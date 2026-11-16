//! Shared look: the kit and core theme for the current appearance.

use rune::tok::{self, Theme};
use rune::{Color, TextStyle};
use rune_kit::Kit;

/// Everything a view function needs to paint itself.
#[derive(Clone)]
pub struct Paint {
    pub k: Kit,
    pub t: Theme,
}

impl Paint {
    pub fn new(dark: bool, _scale: f32) -> Paint {
        Paint {
            k: Kit::neutral(dark),
            t: tok::theme(dark),
        }
    }

    pub fn title(&self) -> TextStyle {
        TextStyle::new(20.0, 650.0).lh(26.0)
    }
    pub fn text(&self) -> TextStyle {
        self.k.ty.ui
    }
    pub fn caption(&self) -> TextStyle {
        self.k.ty.caption()
    }

    pub fn ink(&self) -> Color {
        self.t.text
    }
    pub fn muted(&self) -> Color {
        self.t.text_secondary
    }
}
