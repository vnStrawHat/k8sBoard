use gpui_kit::TestAppContext;
use gpui_kit::component::Theme;

use super::*;
use crate::color_theme::ColorTheme;
use crate::settings::ThemePreference;

#[gpui_kit::test]
fn bundled_families_are_the_theme_fonts_and_survive_theme_changes(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init_with_bundled_fonts(cx);
        assert_eq!(Theme::global(cx).mono_font_family.as_ref(), MONO_FAMILY);
        assert_eq!(Theme::global(cx).font_family.as_ref(), SANS_FAMILY);
        for colors in [ColorTheme::Default, ColorTheme::ZedOne] {
            for mode in [ThemePreference::Light, ThemePreference::Dark] {
                mode.apply(colors, cx);
                assert_eq!(Theme::global(cx).mono_font_family.as_ref(), MONO_FAMILY);
                assert_eq!(Theme::global(cx).font_family.as_ref(), SANS_FAMILY);
            }
        }
    });
}
