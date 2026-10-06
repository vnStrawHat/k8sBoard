//! The Lilex monospace font, built into the app (SIL OFL 1.1, see `fonts/OFL.txt`). Its `l` and
//! `1` differ, which the platform default monospace fonts do not guarantee.

use std::borrow::Cow;

use gpui_kit::App;
use gpui_kit::component::Theme;

const FAMILY: &str = "Lilex";

/// Registers the font files before `gpui_kit::init`, so the kit's installed-font probe sees the
/// family, then makes Lilex the theme's monospace family. Theme changes keep it, because no theme
/// config names a monospace family. A font that fails to load leaves the kit's default.
pub(crate) fn init_with_lilex(cx: &mut App) {
    let files = [
        include_bytes!("../fonts/Lilex-Regular.ttf").as_slice(),
        include_bytes!("../fonts/Lilex-Bold.ttf").as_slice(),
        include_bytes!("../fonts/Lilex-Italic.ttf").as_slice(),
        include_bytes!("../fonts/Lilex-BoldItalic.ttf").as_slice(),
    ];
    let loaded = cx
        .text_system()
        .add_fonts(files.into_iter().map(Cow::Borrowed).collect());
    gpui_kit::init(cx);
    match loaded {
        Ok(()) => Theme::update(cx, |theme| theme.mono_font_family = FAMILY.into()),
        Err(error) => tracing::warn!(%error, "the built-in Lilex font failed to load"),
    }
}

#[cfg(test)]
#[path = "mono_font_tests.rs"]
mod mono_font_tests;
