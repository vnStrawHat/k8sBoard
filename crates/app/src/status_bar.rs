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

/// The state of several viewed clusters as one line. A cluster that is interrupted or failed is a
/// banner of its own; here only the resource types that are watched count.
fn multi_watch_state(sessions: &[&ClusterSession]) -> WatchState {
    let mut watched = None;
    for session in sessions {
        if let SessionPhase::Live(live) = session.phase() {
            watched = Some(watched.unwrap_or(0) + live.watch_count());
        }
    }
    match watched {
        Some(count) => WatchState::Watching(count),
        None if sessions
            .iter()
            .any(|session| matches!(session.phase(), SessionPhase::Connecting { .. })) =>
        {
            WatchState::Connecting
        }
        None => WatchState::Disconnected,
    }
}

/// `Watching 7 resource types · 2 clusters`.
fn multi_watching_text(count: usize, clusters: usize) -> String {
    format!("Watching {count} resource types · {clusters} clusters")
}

pub(crate) fn status_bar(
    shell: &AppShell,
    is_kubeconfig_loading: bool,
    handle: WeakEntity<AppShell>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let sessions: Vec<&ClusterSession> = shell
        .view()
        .slots()
        .iter()
        .map(|slot| slot.session.read(cx))
        .collect();
    let is_multi = sessions.len() >= 2;
    let session = shell.session().map(|session| session.read(cx));
    let state = if is_multi {
        multi_watch_state(&sessions)
    } else {
        watch_state(session, is_kubeconfig_loading)
    };
    let context = session.map_or("", ClusterSession::context);
    let (text, dot): (String, Option<Hsla>) = match state {
        WatchState::LoadingKubeconfig => ("Loading kubeconfig…".to_owned(), None),
        WatchState::Connecting => (format!("Connecting to {context}…"), None),
        WatchState::Watching(count) if is_multi => (
            multi_watching_text(count, sessions.len()),
            Some(theme.success),
        ),
        WatchState::Watching(count) => (
            format!("Watching {count} resource types"),
            Some(theme.success),
        ),
        WatchState::Interrupted => ("Live updates interrupted".to_owned(), Some(theme.warning)),
        WatchState::Disconnected => ("Disconnected".to_owned(), Some(theme.danger)),
    };
    let version = if is_multi {
        api_range_text(
            sessions
                .iter()
                .filter_map(|session| session.live().map(|live| live.api_latency)),
        )
    } else {
        session
            .and_then(ClusterSession::live)
            .map(|live| api_text(&live.server_version.git_version, live.api_latency))
    };
    let user = session
        .filter(|_| !is_multi)
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

/// `API 38–61 ms` over the live clusters; one value when they all answered alike, and nothing
/// while none is live.
fn api_range_text(latencies: impl Iterator<Item = Duration>) -> Option<String> {
    let millis: Vec<u128> = latencies.map(latency_millis).map(u128::from).collect();
    let (min, max) = (millis.iter().min()?, millis.iter().max()?);
    Some(if min == max {
        format!("API {min} ms")
    } else {
        format!("API {min}–{max} ms")
    })
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
    fn status_bar_multi_line() {
        assert_eq!(
            multi_watching_text(7, 2),
            "Watching 7 resource types · 2 clusters"
        );
    }

    #[test]
    fn api_range_shows_the_spread_of_the_live_clusters() {
        let ms = Duration::from_millis;
        assert_eq!(
            api_range_text([ms(38), ms(61)].into_iter()),
            Some("API 38–61 ms".to_owned())
        );
    }

    #[test]
    fn api_range_is_one_value_when_the_clusters_answer_alike() {
        let ms = Duration::from_millis;
        assert_eq!(
            api_range_text([ms(38), ms(38)].into_iter()),
            Some("API 38 ms".to_owned())
        );
    }

    #[test]
    fn api_range_is_absent_while_no_cluster_is_live() {
        assert_eq!(api_range_text(std::iter::empty()), None);
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
