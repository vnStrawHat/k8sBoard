//! The environment a cluster belongs to (production, staging, ...): guessed from names or chosen
//! by the user, and shown as a colored badge. Every environment behaves as one of the four
//! built-ins, its tier, so no custom environment is ever weaker than the built-in it names.

use std::borrow::Cow;

use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _};
use gpui_kit::{App, Hsla, IntoElement, ParentElement as _, Styled as _};
use serde::{Deserialize, Serialize};

/// The four built-ins. Ordered by risk, lowest first, so the riskiest of several name matches is
/// `max()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EnvironmentTier {
    Local,
    Development,
    Staging,
    Production,
}

impl EnvironmentTier {
    /// Display order: the riskiest first.
    pub(crate) const ALL: [Self; 4] = [
        Self::Production,
        Self::Staging,
        Self::Development,
        Self::Local,
    ];

    /// The full name, as the Environment control of Settings lists it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::Development => "Development",
            Self::Staging => "Staging",
            Self::Production => "Production",
        }
    }

    pub(crate) fn badge(self) -> &'static str {
        match self {
            Self::Local => "LOCAL",
            Self::Development => "DEV",
            Self::Staging => "STG",
            Self::Production => "PROD",
        }
    }

    pub(crate) fn color(self) -> EnvironmentColor {
        match self {
            Self::Production => EnvironmentColor::Red,
            Self::Staging => EnvironmentColor::Amber,
            Self::Development => EnvironmentColor::Blue,
            Self::Local => EnvironmentColor::Gray,
        }
    }
}

/// The colors an environment can wear. Each one is a theme token, so no color literal lives here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EnvironmentColor {
    Red,
    Amber,
    Green,
    Blue,
    Teal,
    Purple,
    Gray,
}

/// The only place an environment color touches the theme.
pub(crate) fn palette_color(color: EnvironmentColor, cx: &App) -> Hsla {
    let theme = cx.theme();
    match color {
        EnvironmentColor::Red => theme.danger,
        EnvironmentColor::Amber => theme.warning,
        EnvironmentColor::Green => theme.success,
        EnvironmentColor::Blue => theme.info,
        EnvironmentColor::Teal => theme.cyan,
        EnvironmentColor::Purple => theme.magenta,
        EnvironmentColor::Gray => theme.muted_foreground,
    }
}

/// An environment the user added in Settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CustomEnvironment {
    pub(crate) name: String,
    pub(crate) color: EnvironmentColor,
    pub(crate) tier: EnvironmentTier,
}

/// What a registry entry stores. Untagged: `"production"` is a built-in, any other string a
/// custom name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum EnvironmentKey {
    BuiltIn(EnvironmentTier),
    Custom(String),
}

/// An environment as a cluster wears it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Environment {
    BuiltIn(EnvironmentTier),
    Custom(CustomEnvironment),
}

impl Environment {
    pub(crate) const PRODUCTION: Self = Self::BuiltIn(EnvironmentTier::Production);
    pub(crate) const STAGING: Self = Self::BuiltIn(EnvironmentTier::Staging);
    #[cfg(test)]
    pub(crate) const DEVELOPMENT: Self = Self::BuiltIn(EnvironmentTier::Development);
    #[cfg(test)]
    pub(crate) const LOCAL: Self = Self::BuiltIn(EnvironmentTier::Local);

    /// The built-in whose rules this environment follows.
    pub(crate) fn tier(&self) -> EnvironmentTier {
        match self {
            Self::BuiltIn(tier) => *tier,
            Self::Custom(custom) => custom.tier,
        }
    }

    pub(crate) fn name(&self) -> &str {
        match self {
            Self::BuiltIn(tier) => tier.name(),
            Self::Custom(custom) => &custom.name,
        }
    }

    pub(crate) fn badge(&self) -> Cow<'static, str> {
        match self {
            Self::BuiltIn(tier) => Cow::Borrowed(tier.badge()),
            Self::Custom(custom) => Cow::Owned(custom.name.to_uppercase()),
        }
    }

    pub(crate) fn color(&self) -> EnvironmentColor {
        match self {
            Self::BuiltIn(tier) => tier.color(),
            Self::Custom(custom) => custom.color,
        }
    }
}

/// Production, Staging, then Development and Local together (W2).
pub(crate) const BUILT_IN_GROUP_TITLES: [&str; 3] =
    ["Production", "Staging", "Development · Local"];

fn folded(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Built-in names and badges, the built-in group titles, and `auto`, compared trimmed and
/// lower-case. A custom environment with such a name would pass for a built-in with other rules.
pub(crate) fn is_reserved(name: &str) -> bool {
    let name = folded(name);
    name == "auto"
        || BUILT_IN_GROUP_TITLES
            .iter()
            .any(|title| folded(title) == name)
        || EnvironmentTier::ALL
            .iter()
            .any(|tier| folded(tier.name()) == name || folded(tier.badge()) == name)
}

/// The custom environments resolution may use, in list order: a reserved name, or one repeating an
/// earlier custom name ignoring case, is skipped (only a hand edit can produce either).
pub(crate) fn usable_environments(
    custom: &[CustomEnvironment],
) -> impl Iterator<Item = &CustomEnvironment> {
    custom
        .iter()
        .enumerate()
        .filter_map(|(index, environment)| {
            let name = folded(&environment.name);
            let repeats = custom[..index]
                .iter()
                .any(|earlier| folded(&earlier.name) == name);
            (!is_reserved(&name) && !repeats).then_some(environment)
        })
}

/// Matches a `Custom` key exactly (case-sensitive) among the usable environments. A missing one
/// resolves to Production, so a dangling reference is locked and asks for the name, never weaker.
pub(crate) fn resolve_environment(
    key: &EnvironmentKey,
    custom: &[CustomEnvironment],
) -> Environment {
    match key {
        EnvironmentKey::BuiltIn(tier) => Environment::BuiltIn(*tier),
        EnvironmentKey::Custom(name) => usable_environments(custom)
            .find(|environment| environment.name == *name)
            .map_or(Environment::PRODUCTION, |environment| {
                Environment::Custom(environment.clone())
            }),
    }
}

/// Guesses from the context name and the kubeconfig cluster entry name. The riskiest match
/// wins, so a mixed name never looks safer than it is; no match is Staging, which asks for a
/// confirm click rather than a typed name.
pub(crate) fn guess_environment(context: &str, cluster: &str) -> EnvironmentTier {
    EnvironmentTier::ALL
        .into_iter()
        .find(|tier| {
            [context, cluster]
                .into_iter()
                .any(|name| matches_environment(*tier, name))
        })
        .unwrap_or(EnvironmentTier::Staging)
}

fn matches_environment(tier: EnvironmentTier, name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    let tokens = tokens(&lowered);
    let any_token = |test: &dyn Fn(&str) -> bool| tokens.iter().any(|token| test(token));
    match tier {
        EnvironmentTier::Production => {
            any_token(&|token| token.starts_with("prod") || token == "prd")
        }
        EnvironmentTier::Staging => {
            any_token(&|token| matches!(token, "stg" | "uat") || token.starts_with("stag"))
        }
        EnvironmentTier::Development => {
            any_token(&|token| token.starts_with("dev") || token.starts_with("test"))
        }
        EnvironmentTier::Local => {
            lowered.starts_with("kind-")
                || lowered.starts_with("k3d-")
                || lowered.contains("docker-desktop")
                || any_token(&|token| matches!(token, "minikube" | "localhost"))
        }
    }
}

/// Splits on every char that is not ASCII alphanumeric and trims trailing digits, so `prod1`
/// reads as `prod` and `latest` stays one token (no substring matches).
fn tokens(lowered: &str) -> Vec<&str> {
    lowered
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(|token| token.trim_end_matches(|c: char| c.is_ascii_digit()))
        .filter(|token| !token.is_empty())
        .collect()
}

/// The color of the environment badge.
pub(crate) fn environment_color(environment: &Environment, cx: &App) -> Hsla {
    palette_color(environment.color(), cx)
}

/// A filled badge with light text on the environment color.
pub(crate) fn environment_badge(environment: &Environment, cx: &App) -> impl IntoElement {
    let color = environment_color(environment, cx);
    Tag::custom(color, cx.theme().background, color)
        .small()
        .font_semibold()
        .font_family(cx.theme().mono_font_family.clone())
        .child(environment.badge())
}

#[cfg(test)]
#[path = "environment_tests.rs"]
mod environment_tests;
