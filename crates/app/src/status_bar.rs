use std::time::Duration;

use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    App, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
};

use crate::app_shell::{AppShell, Screen};
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
    shell: &AppShell,
    is_kubeconfig_loading: bool,
    handle: WeakEntity<AppShell>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let session = shell.session().map(|session| session.read(cx));
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
    let forwards = shell.running_forward_count(cx);
    if forwards > 0 {
        bar = bar.left(forwards_slot(forwards, handle));
    }
    bar.right(format!("k8sBoard {}", env!("CARGO_PKG_VERSION")))
}

/// `⇄ 3 port-forwards`; a click opens the Port Forwarding page.
fn forwards_slot(count: usize, shell: WeakEntity<AppShell>) -> impl IntoElement {
    div()
        .id("status-port-forwards")
        .cursor_pointer()
        .child(forwards_text(count))
        .on_click(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.show_screen(Screen::PortForwarding, cx);
            });
        })
}

/// `⇄ 1 port-forward`, `⇄ 3 port-forwards`.
fn forwards_text(count: usize) -> String {
    let noun = if count == 1 {
        "port-forward"
    } else {
        "port-forwards"
    };
    format!("⇄ {count} {noun}")
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
    fn the_forwards_item_counts_in_singular_and_plural() {
        assert_eq!(forwards_text(1), "⇄ 1 port-forward");
        assert_eq!(forwards_text(3), "⇄ 3 port-forwards");
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
