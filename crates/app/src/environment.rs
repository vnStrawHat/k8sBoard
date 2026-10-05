//! The environment a cluster belongs to (production, staging, ...): guessed from names and shown
//! as a colored badge.

use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _};
use gpui_kit::{App, Hsla, IntoElement, ParentElement as _, Styled as _};
use serde::{Deserialize, Serialize};

/// Ordered by risk, lowest first, so the riskiest of several name matches is `max()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Environment {
    Local,
    Development,
    Staging,
    Production,
}

impl Environment {
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
}

/// Guesses from the context name and the kubeconfig cluster entry name. The riskiest match
/// wins, so a mixed name never looks safer than it is; no match is Staging, which asks for a
/// confirm click rather than a typed name.
pub(crate) fn guess_environment(context: &str, cluster: &str) -> Environment {
    [
        Environment::Production,
        Environment::Staging,
        Environment::Development,
        Environment::Local,
    ]
    .into_iter()
    .find(|environment| {
        [context, cluster]
            .into_iter()
            .any(|name| matches_environment(*environment, name))
    })
    .unwrap_or(Environment::Staging)
}

fn matches_environment(environment: Environment, name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    let tokens = tokens(&lowered);
    let any_token = |test: &dyn Fn(&str) -> bool| tokens.iter().any(|token| test(token));
    match environment {
        Environment::Production => any_token(&|token| token.starts_with("prod") || token == "prd"),
        Environment::Staging => {
            any_token(&|token| matches!(token, "stg" | "uat") || token.starts_with("stag"))
        }
        Environment::Development => {
            any_token(&|token| token.starts_with("dev") || token.starts_with("test"))
        }
        Environment::Local => {
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

/// The colors an environment can wear. Each one is a theme token, so no color literal lives here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EnvironmentColor {
    Red,
    Amber,
    Blue,
    Purple,
    Teal,
    Gray,
}

impl EnvironmentColor {
    /// The color an environment wears.
    pub(crate) fn of(environment: Environment) -> Self {
        match environment {
            Environment::Production => Self::Red,
            Environment::Staging => Self::Amber,
            Environment::Development => Self::Blue,
            Environment::Local => Self::Gray,
        }
    }
}

/// The only place an environment color touches the theme.
pub(crate) fn palette_color(color: EnvironmentColor, cx: &App) -> Hsla {
    let theme = cx.theme();
    match color {
        EnvironmentColor::Red => theme.danger,
        EnvironmentColor::Amber => theme.warning,
        EnvironmentColor::Blue => theme.info,
        EnvironmentColor::Purple => theme.magenta,
        EnvironmentColor::Teal => theme.cyan,
        EnvironmentColor::Gray => theme.muted_foreground,
    }
}

/// The color of the environment badge.
pub(crate) fn environment_color(environment: Environment, cx: &App) -> Hsla {
    palette_color(EnvironmentColor::of(environment), cx)
}

/// A filled badge with light text on the environment color.
pub(crate) fn environment_badge(environment: Environment, cx: &App) -> impl IntoElement {
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
