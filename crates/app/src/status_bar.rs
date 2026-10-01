use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{App, Hsla, IntoElement, ParentElement as _, Styled as _, div};

use crate::cluster_session::{ClusterSession, SessionPhase};

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
        .map(|live| format!("API {}", live.server_version.git_version));
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

fn watch_slot(text: String, dot: Option<Hsla>) -> impl IntoElement {
    h_flex()
        .gap_1()
        .items_center()
        .children(dot.map(|color| div().size_2().rounded_full().bg(color)))
        .child(text)
}
