use gpui_kit::TestAppContext;
use gpui_kit::component::Theme;

use super::*;
use crate::color_theme::ColorTheme;
use crate::settings::ThemePreference;

#[gpui_kit::test]
fn lilex_is_the_monospace_family_and_survives_theme_changes(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init_with_lilex(cx);
        assert_eq!(Theme::global(cx).mono_font_family.as_ref(), FAMILY);
        for colors in [ColorTheme::Default, ColorTheme::ZedOne] {
            for mode in [ThemePreference::Light, ThemePreference::Dark] {
                mode.apply(colors, cx);
                assert_eq!(Theme::global(cx).mono_font_family.as_ref(), FAMILY);
            }
        }
    });
}
