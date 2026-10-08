use gpui_kit::TestAppContext;
use gpui_kit::component::Theme;

use super::*;
use crate::color_theme::ColorTheme;
use crate::settings::ThemePreference;

#[gpui_kit::test]
fn bundled_lilex_is_both_theme_fonts_and_survive_theme_changes(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init_with_bundled_fonts(cx);
        assert_eq!(Theme::global(cx).mono_font_family.as_ref(), FAMILY);
        assert_eq!(Theme::global(cx).font_family.as_ref(), FAMILY);
        for colors in [ColorTheme::Default, ColorTheme::ZedOne] {
            for mode in [ThemePreference::Light, ThemePreference::Dark] {
                mode.apply(colors, cx);
                assert_eq!(Theme::global(cx).mono_font_family.as_ref(), FAMILY);
                assert_eq!(Theme::global(cx).font_family.as_ref(), FAMILY);
            }
        }
    });
}

#[gpui_kit::test]
fn the_font_size_survives_theme_changes_and_mono_text_keeps_its_offset(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init_with_bundled_fonts(cx);
        assert_eq!(Theme::global(cx).font_size, px(16.));
        assert_eq!(Theme::global(cx).mono_font_size, px(13.));
        apply_font_size(14, cx);
        for colors in [ColorTheme::Default, ColorTheme::ZedOne] {
            for mode in [ThemePreference::Light, ThemePreference::Dark] {
                mode.apply(colors, cx);
                assert_eq!(Theme::global(cx).font_size, px(14.));
                assert_eq!(Theme::global(cx).mono_font_size, px(11.));
            }
        }
    });
}
