//! The font built into the app: Lilex under the SIL OFL 1.1 (`fonts/OFL.txt`).
//!
//! Lilex is both the interface font and the monospace font, so the whole UI looks the same on
//! every platform. Its `l` and `1` differ, which the platform default fonts do not guarantee.

use std::borrow::Cow;

use gpui_kit::component::Theme;
use gpui_kit::{App, px};

const FAMILY: &str = "Lilex";

/// Registers the font files before `gpui_kit::init`, so the kit's installed-font probe sees the
/// family, then makes Lilex the theme's UI and monospace family. Theme changes keep both, because
/// no theme config names a font family. A family that fails to load leaves the kit's defaults.
pub(crate) fn init_with_bundled_fonts(cx: &mut App) {
    let files = [
        include_bytes!("../fonts/Lilex-Regular.ttf").as_slice(),
        include_bytes!("../fonts/Lilex-Bold.ttf").as_slice(),
        include_bytes!("../fonts/Lilex-Italic.ttf").as_slice(),
        include_bytes!("../fonts/Lilex-BoldItalic.ttf").as_slice(),
    ];
    let result = cx
        .text_system()
        .add_fonts(files.into_iter().map(Cow::Borrowed).collect());
    if let Err(error) = &result {
        tracing::warn!(%error, family = FAMILY, "the built-in font failed to load");
    }
    gpui_kit::init(cx);
    if result.is_ok() {
        Theme::update(cx, |theme| {
            theme.font_family = FAMILY.into();
            theme.mono_font_family = FAMILY.into();
        });
    }
}

/// Sets the base size of the whole interface; the kit root reads `theme.font_size` as the rem
/// size every frame, so every window follows at once. Monospace text (the terminal) keeps its
/// 3 px offset from the UI size (13 px at the default 16 px) and grows with it.
pub(crate) fn apply_font_size(size: u8, cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.font_size = px(f32::from(size));
        theme.mono_font_size = px(f32::from(size) - MONO_SIZE_OFFSET);
    });
}

const MONO_SIZE_OFFSET: f32 = 3.;

#[cfg(test)]
#[path = "bundled_fonts_tests.rs"]
mod bundled_fonts_tests;
