//! The fonts built into the app, both under the SIL OFL 1.1 (`fonts/OFL.txt` for Lilex,
//! `fonts/OFL-Inter.txt` for Inter).
//!
//! Lilex is the monospace font: its `l` and `1` differ, which the platform default monospace fonts
//! do not guarantee. Inter is the interface font: its ascent plus descent is 1.21 em, so the
//! descenders fit the 1.25 line boxes the kit clips labels to. Segoe UI measures 1.33 em on
//! Windows, which cuts about 1 px off every descender.

use std::borrow::Cow;

use gpui_kit::App;
use gpui_kit::component::Theme;

const MONO_FAMILY: &str = "Lilex";
const SANS_FAMILY: &str = "Inter";

/// Registers the font files before `gpui_kit::init`, so the kit's installed-font probe sees the
/// families, then makes Inter the theme's UI family and Lilex its monospace family. Theme changes
/// keep both, because no theme config names a font family. A family that fails to load leaves the
/// kit's default for that family only.
pub(crate) fn init_with_bundled_fonts(cx: &mut App) {
    let mono_loaded = add_family(
        cx,
        MONO_FAMILY,
        vec![
            include_bytes!("../fonts/Lilex-Regular.ttf").as_slice(),
            include_bytes!("../fonts/Lilex-Bold.ttf").as_slice(),
            include_bytes!("../fonts/Lilex-Italic.ttf").as_slice(),
            include_bytes!("../fonts/Lilex-BoldItalic.ttf").as_slice(),
        ],
    );
    let sans_loaded = add_family(
        cx,
        SANS_FAMILY,
        vec![
            include_bytes!("../fonts/Inter-Regular.ttf").as_slice(),
            include_bytes!("../fonts/Inter-Medium.ttf").as_slice(),
            include_bytes!("../fonts/Inter-SemiBold.ttf").as_slice(),
            include_bytes!("../fonts/Inter-Bold.ttf").as_slice(),
        ],
    );
    gpui_kit::init(cx);
    Theme::update(cx, |theme| {
        if mono_loaded {
            theme.mono_font_family = MONO_FAMILY.into();
        }
        if sans_loaded {
            theme.font_family = SANS_FAMILY.into();
        }
    });
}

fn add_family(cx: &mut App, family: &str, files: Vec<&'static [u8]>) -> bool {
    let result = cx
        .text_system()
        .add_fonts(files.into_iter().map(Cow::Borrowed).collect());
    if let Err(error) = &result {
        tracing::warn!(%error, family, "a built-in font failed to load");
    }
    result.is_ok()
}

#[cfg(test)]
#[path = "bundled_fonts_tests.rs"]
mod bundled_fonts_tests;
