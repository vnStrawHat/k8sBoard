use gpui_kit::TestAppContext;
use gpui_kit::component::{ThemeColor, try_parse_color};
use serde_json::Value;

use super::*;
use crate::settings::ThemePreference;

/// The kit keys the theme file must set, from the colour table of spec 0052.
const REQUIRED_KEYS: [&str; 56] = [
    "background",
    "foreground",
    "muted.background",
    "muted.foreground",
    "border",
    "ring",
    "primary.background",
    "primary.foreground",
    "secondary.background",
    "secondary.hover.background",
    "secondary.active.background",
    "accent.background",
    "popover.background",
    "selection.background",
    "caret",
    "drop_target.background",
    "title_bar.background",
    "sidebar.background",
    "sidebar.accent.background",
    "tab_bar.background",
    "tab.background",
    "tab.active.background",
    "tab.foreground",
    "table.head.background",
    "table.row.border",
    "list.hover.background",
    "list.active.background",
    "list.active.border",
    "scrollbar.thumb.background",
    "success.background",
    "warning.background",
    "danger.background",
    "info.background",
    "success.foreground",
    "warning.foreground",
    "danger.foreground",
    "info.foreground",
    "base.red",
    "base.red.light",
    "base.green",
    "base.green.light",
    "base.yellow",
    "base.yellow.light",
    "base.blue",
    "base.blue.light",
    "base.magenta",
    "base.magenta.light",
    "base.cyan",
    "base.cyan.light",
    "chart.1",
    "chart.2",
    "chart.3",
    "chart.4",
    "chart.5",
    "chart.bullish",
    "chart.bearish",
];

fn raw_themes() -> Vec<Value> {
    let file: Value = serde_json::from_str(ZED_ONE).expect("the embedded file is JSON");
    file["themes"].as_array().expect("themes array").clone()
}

/// A colour of the embedded file, read raw because the kit's base colour fields are private.
fn colour(mode: ThemeMode, key: &str) -> gpui_kit::Hsla {
    let name = match mode {
        ThemeMode::Light => "One Light",
        ThemeMode::Dark => "One Dark",
    };
    let themes = raw_themes();
    let theme = themes
        .iter()
        .find(|theme| theme["name"] == name)
        .expect("theme is in the file");
    let raw = theme["colors"][key].as_str().expect("key is set");
    try_parse_color(raw).expect("colour parses")
}

fn family() -> ThemeFamily {
    zed_one_family().expect("the embedded theme parses")
}

#[test]
fn zed_one_has_one_light_and_one_dark_theme() {
    let family = family();
    assert_eq!(family.light.name, "One Light");
    assert_eq!(family.light.mode, ThemeMode::Light);
    assert_eq!(family.dark.name, "One Dark");
    assert_eq!(family.dark.mode, ThemeMode::Dark);
    assert_eq!(raw_themes().len(), 2);
}

#[test]
fn zed_one_sets_every_required_token() {
    for theme in raw_themes() {
        let colors = theme["colors"].as_object().expect("colors object");
        for key in REQUIRED_KEYS {
            let value = colors
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("{} lacks {key}", theme["name"]));
            assert!(try_parse_color(value).is_ok(), "{key} = {value}");
        }
    }
}

#[test]
fn zed_one_sets_no_font_radius_or_shadow() {
    let family = family();
    for config in [&family.light, &family.dark] {
        assert!(config.font_size.is_none());
        assert!(config.font_family.is_none());
        assert!(config.mono_font_family.is_none());
        assert!(config.mono_font_size.is_none());
        assert!(config.radius.is_none());
        assert!(config.radius_lg.is_none());
        assert!(config.shadow.is_none());
    }
}

#[test]
fn zed_one_has_a_highlight_style() {
    let family = family();
    for config in [&family.light, &family.dark] {
        let highlight = config.highlight.as_ref().expect("highlight is set");
        assert_eq!(
            highlight.editor_background,
            Some(colour(config.mode, "background"))
        );
        assert!(highlight.syntax.string.is_some());
        assert!(highlight.syntax.keyword.is_some());
    }
}

#[gpui_kit::test]
fn zed_one_dark_applies_its_colours(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        ThemePreference::Dark.apply(ColorTheme::ZedOne, cx);
        let theme = Theme::global(cx);
        assert_eq!(theme.theme_name(), "One Dark");
        assert!(theme.is_dark());
        assert_eq!(theme.background, colour(ThemeMode::Dark, "background"));
        assert_eq!(theme.foreground, colour(ThemeMode::Dark, "foreground"));
        assert_eq!(theme.danger, colour(ThemeMode::Dark, "danger.background"));
        assert_eq!(theme.blue_light, colour(ThemeMode::Dark, "base.blue.light"));
    });
}

#[gpui_kit::test]
fn mode_change_keeps_the_family(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        ThemePreference::Dark.apply(ColorTheme::ZedOne, cx);
        Theme::change(ThemeMode::Light, None, cx);
        let theme = Theme::global(cx);
        assert_eq!(theme.theme_name(), "One Light");
        assert_eq!(theme.background, colour(ThemeMode::Light, "background"));
    });
}

#[gpui_kit::test]
fn default_restores_the_kit_theme(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        ThemePreference::Dark.apply(ColorTheme::Default, cx);
        let (font, mono, radius) = {
            let theme = Theme::global(cx);
            (
                theme.font_family.clone(),
                theme.mono_font_family.clone(),
                theme.radius,
            )
        };
        ThemePreference::Dark.apply(ColorTheme::ZedOne, cx);
        ThemePreference::Dark.apply(ColorTheme::Default, cx);
        let theme = Theme::global(cx);
        assert_eq!(theme.theme_name(), "Default Dark");
        assert_eq!(theme.background, ThemeColor::dark().background);
        assert_eq!(theme.font_family, font);
        assert_eq!(theme.mono_font_family, mono);
        assert_eq!(theme.radius, radius);
    });
}

#[gpui_kit::test]
fn table_active_is_the_selection_in_every_combination(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        for colors in [ColorTheme::Default, ColorTheme::ZedOne] {
            for mode in [ThemePreference::Light, ThemePreference::Dark] {
                mode.apply(colors, cx);
                let theme = Theme::global(cx);
                assert_eq!(theme.table_active, theme.selection, "{colors:?} {mode:?}");
            }
        }
    });
}

#[gpui_kit::test]
fn highlight_follows_the_family(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        ThemePreference::Dark.apply(ColorTheme::ZedOne, cx);
        assert_eq!(Theme::global(cx).highlight_theme.name, "One Dark");
        ThemePreference::Light.apply(ColorTheme::Default, cx);
        assert_eq!(Theme::global(cx).highlight_theme.name, "Default Light");
    });
}
