use std::time::Duration;

use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
};

use crate::app_shell::{AppShell, Screen};
use crate::cluster_registry::ClusterProfile;
use crate::cluster_session::{ClusterSession, SessionPhase, latency_millis};
use crate::environment::environment_badge;
use crate::status_tooltip::{Section, table_tooltip};
use crate::watched_kinds::{WatchedKind, watched_row};

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
    let watched = match state {
        WatchState::Watching(_) => session
            .and_then(ClusterSession::live)
            .map(|live| live.watched_kinds()),
        _ => None,
    };
    let version = session
        .and_then(ClusterSession::live)
        .map(|live| api_text(&live.server_version.git_version, live.api_latency));
    let user = session
        .and_then(ClusterSession::user)
        .map(|user| format!("user: {user}"));
    // First, so a Production cluster is named even when the title bar is scrolled or covered.
    let mut items: Vec<AnyElement> = shell
        .active_profile(cx)
        .map(|profile| environment_slot(&profile, cx))
        .into_iter()
        .collect();
    items.push(watch_slot(text, dot, watched));
    items.extend(version.map(IntoElement::into_any_element));
    items.extend(user.map(IntoElement::into_any_element));
    let forwards = shell.running_forward_count(cx);
    if forwards > 0 {
        items.push(forwards_slot(forwards, handle).into_any_element());
    }
    let mut bar = StatusBar::new();
    for item in separated(items, separator_color(cx)) {
        bar = bar.left(item);
    }
    bar.right(shell.usage().clone())
}

/// The environment badge of the title bar, then the cluster's display name.
fn environment_slot(profile: &ClusterProfile, cx: &App) -> AnyElement {
    h_flex()
        .id("status-environment")
        .gap_1p5()
        .items_center()
        .child(environment_badge(&profile.environment, cx))
        .child(profile.display_name.clone())
        .into_any_element()
}

/// `items` with a vertical rule between each pair.
fn separated(items: Vec<AnyElement>, color: Hsla) -> Vec<AnyElement> {
    let mut separated = Vec::with_capacity(items.len() * 2);
    for (index, item) in items.into_iter().enumerate() {
        if index > 0 {
            separated.push(status_separator(color).into_any_element());
        }
        separated.push(item);
    }
    separated
}

/// The color of the rules between status bar items: muted text, softened.
pub(crate) fn separator_color(cx: &App) -> Hsla {
    cx.theme().muted_foreground.opacity(0.4)
}

/// The rule between two status bar items; the CPU and network items use the same one.
pub(crate) fn status_separator(color: Hsla) -> impl IntoElement {
    div().w_px().h_3().flex_none().bg(color)
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

/// The dot and text of the live state; while watching, hovering lists the watched kinds.
fn watch_slot(text: String, dot: Option<Hsla>, watched: Option<Vec<WatchedKind>>) -> AnyElement {
    let slot = h_flex()
        .id("status-watching")
        .gap_1()
        .items_center()
        .children(dot.map(|color| div().size_2().rounded_full().bg(color)))
        .child(text);
    match watched {
        Some(kinds) => slot
            .tooltip(table_tooltip(watched_sections(&kinds)))
            .into_any_element(),
        None => slot.into_any_element(),
    }
}

/// The most kinds the watching tooltip lists; a tall list would run off a short window.
const WATCHED_ROWS_SHOWN: usize = 20;

/// The tooltip table of the watching slot: one row per kind, with `×N` for several watches, and
/// a `+N` row for the kinds past `WATCHED_ROWS_SHOWN`. The note says why the number changes.
fn watched_sections(kinds: &[WatchedKind]) -> Vec<Section> {
    let mut rows: Vec<_> = kinds
        .iter()
        .take(WATCHED_ROWS_SHOWN)
        .map(watched_row)
        .collect();
    if kinds.len() > WATCHED_ROWS_SHOWN {
        rows.push(("and more", format!("+{}", kinds.len() - WATCHED_ROWS_SHOWN)));
    }
    vec![
        Section {
            title: "Watched resources",
            rows,
        },
        Section {
            title: "Follows the screens you have opened",
            rows: Vec::new(),
        },
    ]
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

    #[test]
    fn the_watching_tooltip_lists_each_kind_with_its_multiplier() {
        let kinds = [
            WatchedKind {
                name: "Nodes",
                count: 1,
            },
            WatchedKind {
                name: "Pods",
                count: 3,
            },
        ];
        let sections = watched_sections(&kinds);
        assert_eq!(
            sections[0],
            Section {
                title: "Watched resources",
                rows: vec![("Nodes", String::new()), ("Pods", "×3".to_owned())],
            }
        );
    }

    #[test]
    fn a_long_watching_tooltip_folds_the_rest_into_one_row() {
        let kinds = vec![
            WatchedKind {
                name: "Pods",
                count: 1
            };
            WATCHED_ROWS_SHOWN + 4
        ];
        let rows = &watched_sections(&kinds)[0].rows;
        assert_eq!(rows.len(), WATCHED_ROWS_SHOWN + 1);
        assert_eq!(rows.last(), Some(&("and more", "+4".to_owned())));
    }
}
