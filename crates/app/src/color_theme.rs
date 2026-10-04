//! The colour theme families (Default and Zed One) and how one is put into the kit's theme slots.

use std::rc::Rc;

use gpui_kit::App;
use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry, ThemeSet};
use serde::{Deserialize, Serialize};

/// One Light and One Dark, colours from Zed's `one.json` (MIT, see the notice in the file).
const ZED_ONE: &str = include_str!("../themes/zed-one.json");

/// The colour family, orthogonal to the light/dark mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ColorTheme {
    /// One Light / One Dark, picked by the mode.
    ZedOne,
    /// The kit's own theme: today's look. Last because serde requires `other` on the last variant.
    #[default]
    #[serde(other)] // any unknown string loads as Default instead of resetting settings.json
    Default,
}

/// The light and dark configs of one family.
struct ThemeFamily {
    light: ThemeConfig,
    dark: ThemeConfig,
}

impl ColorTheme {
    /// Puts this family's light and dark configs in the theme slots; the next `Theme::change`
    /// loads one of them.
    pub(crate) fn install(self, cx: &mut App) {
        let installed = match self {
            Self::ZedOne => zed_one_family().or_else(|| {
                tracing::warn!("the embedded Zed One theme does not parse");
                None
            }),
            Self::Default => None,
        };
        let (light, dark) = match installed {
            Some(family) => (Rc::new(family.light), Rc::new(family.dark)),
            None => {
                let registry = ThemeRegistry::global(cx);
                (
                    registry.default_light_theme().clone(),
                    registry.default_dark_theme().clone(),
                )
            }
        };
        let theme = Theme::global_mut(cx);
        theme.light_theme = light;
        theme.dark_theme = dark;
    }
}

/// One Light and One Dark; `None` only if the embedded file is broken (a unit test guards it).
fn zed_one_family() -> Option<ThemeFamily> {
    let set: ThemeSet = serde_json::from_str(ZED_ONE).ok()?;
    let mut light = None;
    let mut dark = None;
    for config in set.themes {
        match config.mode {
            ThemeMode::Light => light = Some(config),
            ThemeMode::Dark => dark = Some(config),
        }
    }
    Some(ThemeFamily {
        light: light?,
        dark: dark?,
    })
}

#[cfg(test)]
#[path = "color_theme_tests.rs"]
mod color_theme_tests;
