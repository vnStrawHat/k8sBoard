use std::time::Duration;

use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{App, Hsla, IntoElement, ParentElement as _, Styled as _, div};

use crate::cluster_session::{ClusterSession, SessionPhase, latency_millis};

/// What the first status bar slot says about the live updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WatchState {
    LoadingKubeconfig,
    Connecting,
    Watching(usize),
    Interrupted,
    Disconnected,
}

fn watch_state(session: Option<&ClusterSession>, is_kubeconfig_loading: bool) -> WatchState {
    let Some(session) = session else {
        return if is_kubeconfig_loading {
            WatchState::LoadingKubeconfig
        } else {
            WatchState::Disconnected
        };
    };
    match session.phase() {
        SessionPhase::Connecting { .. } => WatchState::Connecting,
        SessionPhase::Failed { .. } => WatchState::Disconnected,
        SessionPhase::Live(live) => {
            if live.has_problem() {
                WatchState::Interrupted
            } else {
                WatchState::Watching(live.watch_count())
            }
        }
    }
}

pub(crate) fn status_bar(
    session: Option<&ClusterSession>,
    is_kubeconfig_loading: bool,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let state = watch_state(session, is_kubeconfig_loading);
    let context = session.map_or("", ClusterSession::context);
    let (text, dot): (String, Option<Hsla>) = match state {
        WatchState::LoadingKubeconfig => ("Loading kubeconfig…".to_owned(), None),
        WatchState::Connecting => (format!("Connecting to {context}…"), None),
        WatchState::Watching(count) => (
            format!("Watching {count} resource types"),
            Some(theme.success),
        ),
        WatchState::Interrupted => ("Live updates interrupted".to_owned(), Some(theme.warning)),
        WatchState::Disconnected => ("Disconnected".to_owned(), Some(theme.danger)),
    };
    let version = session
        .and_then(ClusterSession::live)
        .map(|live| api_text(&live.server_version.git_version, live.api_latency));
    let user = session
        .and_then(ClusterSession::user)
        .map(|user| format!("user: {user}"));
    let mut bar = StatusBar::new().left(watch_slot(text, dot));
    if let Some(version) = version {
        bar = bar.left(version);
    }
    if let Some(user) = user {
        bar = bar.left(user);
    }
    bar.right(format!("k8sBoard {}", env!("CARGO_PKG_VERSION")))
}

/// `API v1.29.5 · 38 ms`: the server version and the round trip of its request.
fn api_text(git_version: &str, latency: Duration) -> String {
    format!("API {git_version} · {} ms", latency_millis(latency))
}

fn watch_slot(text: String, dot: Option<Hsla>) -> impl IntoElement {
    h_flex()
        .gap_1()
        .items_center()
        .children(dot.map(|color| div().size_2().rounded_full().bg(color)))
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_bar_shows_version_and_latency() {
        assert_eq!(
            api_text("v1.29.5", Duration::from_millis(38)),
            "API v1.29.5 · 38 ms"
        );
    }

    #[test]
    fn latency_below_one_ms_shows_one() {
        assert_eq!(
            api_text("v1.29.5", Duration::from_micros(400)),
            "API v1.29.5 · 1 ms"
        );
        assert_eq!(latency_millis(Duration::ZERO), 1);
    }
}
