use cluster::NamespaceScope;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Hsla, IntoElement, ParentElement as _, Styled as _, div, px,
};

use crate::app_shell::{AppShell, Screen};
use crate::cluster_registry::ClusterRef;
use crate::cluster_session::namespaces_label;
use crate::cluster_switcher::cluster_switcher as switcher_popover;
use crate::environment::{Environment, environment_badge, environment_color};
use crate::issue_board::IssueSummary;
use crate::keymap::{OpenPalette, ToggleReadOnly};
use crate::namespace_picker::{PickerAnchor, namespace_picker as picker};
use crate::settings::AppSettings;
use crate::settings_window::OpenSettings;
use crate::shortcut_sheet::row_keys;
use crate::status_tone::tone_color;
use crate::write_guard::WriteLock;

pub(crate) fn title_bar(shell: &AppShell, cx: &Context<AppShell>) -> impl IntoElement {
    // Always 3 px, so the layout does not shift when a session starts. GPUI has one border
    // color per element, so the kit's 1 px bottom border takes the same color.
    let border = match border_environment(shell, cx) {
        Some(environment) => environment_color(environment, cx),
        None => cx.theme().title_bar_border,
    };
    TitleBar::new()
        .border_t(px(3.))
        .border_color(border)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().font_semibold().child("k8sBoard"))
                .child(cluster_switcher(shell, cx))
                .child(namespace_picker(shell, cx)),
        )
        .child(search_box(cx))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .children(notices_button(shell, cx))
                .children(write_lock_badge(shell, cx))
                .child(issues_button(shell, cx))
                .child(settings_button()),
        )
}

/// The flag with the issue count, in the tone of the worst issue. Without a count it is a muted
/// icon: nothing to show yet, or no issues.
fn issues_button(shell: &AppShell, cx: &Context<AppShell>) -> AnyElement {
    let button = Button::new("issues")
        .ghost()
        .small()
        .on_click(cx.listener(|shell, _, _, cx| shell.show_screen(Screen::Issues, cx)));
    let muted = cx.theme().muted_foreground;
    let live_session = shell
        .session()
        .map(|session| session.read(cx))
        .filter(|session| session.live().is_some());
    let Some(session) = live_session else {
        return button
            .child(flag(muted, None))
            .tooltip("Not connected")
            .disabled(true)
            .into_any_element();
    };
    let Some(summary) = session.issues().summary() else {
        return button
            .child(flag(muted, None))
            .tooltip("Checking for issues…")
            .into_any_element();
    };
    let shown = match summary.total {
        0 => flag(muted, None),
        total => flag(tone_color(summary.worst().tone(), cx), Some(total)),
    };
    button
        .child(shown)
        .tooltip(issues_tooltip(summary, session.issues().coverage().note()))
        .into_any_element()
}

fn flag(color: Hsla, total: Option<usize>) -> impl IntoElement {
    h_flex()
        .gap_1()
        .items_center()
        .text_color(color)
        .child(Icon::new(IconName::Flag))
        .children(total.map(|total| total.to_string()))
}

/// `4 issues (1 critical)`; a rule that could not run is named after `partial coverage`.
fn issues_tooltip(summary: IssueSummary, coverage_note: Option<String>) -> String {
    let unit = if summary.total == 1 {
        "issue"
    } else {
        "issues"
    };
    let mut text = format!("{} {unit} ({} critical)", summary.total, summary.critical);
    if summary.is_partial {
        text.push_str(" · partial coverage");
        if let Some(note) = coverage_note {
            text.push_str(": ");
            text.push_str(&note);
        }
    }
    text
}

/// The environment of the top border: the riskiest of the viewed clusters while several are viewed
/// (0024 decision 26), else the one cluster's own.
fn border_environment(shell: &AppShell, cx: &App) -> Option<Environment> {
    if shell.view().is_multi() {
        return shell.view().riskiest();
    }
    shell.active_profile(cx).map(|profile| profile.environment)
}

/// What the cluster trigger shows: the primary label, and `+{n−1}` for the other viewed clusters.
#[derive(Debug, PartialEq, Eq)]
struct TriggerText {
    label: String,
    plus: Option<String>,
    /// `Viewing: {label}, {label}, …`, while several clusters are viewed.
    tooltip: Option<String>,
}

/// `labels` are the viewed clusters in display order; `primary_label` is the primary's.
fn trigger_text(primary_label: &str, labels: &[&str]) -> TriggerText {
    let is_multi = labels.len() >= 2;
    TriggerText {
        label: primary_label.to_owned(),
        plus: is_multi.then(|| format!("+{}", labels.len() - 1)),
        tooltip: is_multi.then(|| format!("Viewing: {}", labels.join(", "))),
    }
}

fn cluster_switcher(shell: &AppShell, cx: &Context<AppShell>) -> AnyElement {
    let view = shell.view();
    let mut tooltip = None;
    let trigger = match shell.active_profile(cx) {
        Some(profile) => {
            let labels: Vec<&str> = view
                .slots()
                .iter()
                .map(|slot| slot.label.as_str())
                .collect();
            // The primary decides the badge and the label of several clusters; one cluster keeps
            // its display name.
            let primary = view.primary().filter(|_| view.is_multi());
            let (environment, label) = match primary {
                Some(slot) => (slot.profile.environment, slot.label.as_str()),
                None => (profile.environment, profile.display_name.as_str()),
            };
            let text = trigger_text(label, &labels);
            tooltip = text.tooltip;
            h_flex()
                .gap_2()
                .items_center()
                .child(environment_badge(environment, cx))
                .child(text.label)
                .children(text.plus.map(|plus| {
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(plus)
                }))
                .into_any_element()
        }
        None => "No cluster".into_any_element(),
    };
    let trigger = Button::new("cluster-switcher")
        .ghost()
        .small()
        .child(trigger)
        .dropdown_caret(true);
    let trigger = match tooltip {
        Some(tooltip) => trigger.tooltip(tooltip),
        None => trigger,
    };
    switcher_popover(trigger, shell, cx)
}

/// The search box of the middle slot (inventory T6): a click opens the command palette. It
/// shrinks before the groups on either side do.
fn search_box(cx: &Context<AppShell>) -> AnyElement {
    let key = row_keys(&OpenPalette, cx).into_iter().next();
    Button::new("palette-search")
        .ghost()
        .small()
        .flex_1()
        .min_w_0()
        .max_w(px(280.))
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::Search).size_4())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_left()
                        .child("Search resources or run a command…"),
                )
                .children(key.map(Kbd::new)),
        )
        .on_click(cx.listener(|shell, _, window, cx| shell.open_palette("", window, cx)))
        .into_any_element()
}

/// `ns: all`, `ns: kube-system`, or the shortened list of several: the title-bar label of a scope.
pub(crate) fn scope_label(scope: &NamespaceScope) -> String {
    match scope {
        NamespaceScope::All => "ns: all".to_owned(),
        NamespaceScope::Named(namespace) => format!("ns: {namespace}"),
        NamespaceScope::Several(names) => format!("ns: {}", namespaces_label(names)),
    }
}

fn namespace_picker(shell: &AppShell, cx: &Context<AppShell>) -> AnyElement {
    let trigger = Button::new("namespace-picker").ghost().small();
    let Some(live) = shell.scope_live(cx) else {
        return trigger.label("ns: —").disabled(true).into_any_element();
    };
    let label = scope_label(&live.scope);
    let trigger = trigger.label(label).dropdown_caret(true);
    picker(PickerAnchor::TitleBar, trigger, shell, cx)
}

/// One viewed cluster as the lock badge reads it.
struct LockedCluster {
    cluster: ClusterRef,
    label: String,
    environment: Environment,
    lock: WriteLock,
}

/// What the badge shows: the lock icon and its text.
#[derive(Debug, PartialEq, Eq)]
struct BadgeFace {
    lock: WriteLock,
    text: String,
}

/// `Read-only` only when every viewed cluster is locked. Any open cluster wins, because it is the
/// state that can change something: `Unlocked` for one viewed cluster, `Unlocked: stg-b` for one
/// open cluster among several, and `2 of 3 unlocked` for more.
fn badge_face(clusters: &[LockedCluster]) -> BadgeFace {
    let open: Vec<&LockedCluster> = clusters
        .iter()
        .filter(|cluster| cluster.lock == WriteLock::Unlocked)
        .collect();
    let text = match (clusters.len(), open.as_slice()) {
        (_, []) => {
            return BadgeFace {
                lock: WriteLock::Locked,
                text: "Read-only".to_owned(),
            };
        }
        (1, _) => "Unlocked".to_owned(),
        (_, [only]) => format!("Unlocked: {}", only.label),
        (total, several) => format!("{} of {total} unlocked", several.len()),
    };
    BadgeFace {
        lock: WriteLock::Unlocked,
        text,
    }
}

/// The tooltip of the badge: one cluster says what the lock means; several list each one's state.
fn badge_tooltip(clusters: &[LockedCluster]) -> String {
    let state = |lock: WriteLock| match lock {
        WriteLock::Locked => "Read-only",
        WriteLock::Unlocked => "Unlocked",
    };
    match clusters {
        [only] => format!("{}: {}", only.label, state(only.lock)),
        several => several
            .iter()
            .map(|cluster| format!("{}: {}", cluster.label, state(cluster.lock)))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// The lock of the viewed clusters: a lock and `Read-only`, or an open lock and `Unlocked`, with a
/// dashed border in the environment color. One cluster: a click toggles its lock. Several: a click
/// opens a menu with one item per cluster, ticked while it is read-only. Hidden without a session.
fn write_lock_badge(shell: &AppShell, cx: &Context<AppShell>) -> Option<AnyElement> {
    let view = shell.view();
    let clusters: Vec<LockedCluster> = view
        .slots()
        .iter()
        .map(|slot| LockedCluster {
            cluster: slot.cluster.clone(),
            label: slot.label.clone(),
            environment: slot.profile.environment,
            lock: slot.session.read(cx).lock(),
        })
        .collect();
    let first = clusters.first()?;
    let environment = if view.is_multi() {
        view.riskiest().unwrap_or(first.environment)
    } else {
        first.environment
    };
    let face = badge_face(&clusters);
    let icon = match face.lock {
        WriteLock::Locked => IconName::Lock,
        WriteLock::Unlocked => IconName::LockOpen,
    };
    let button = Button::new("write-lock")
        .ghost()
        .small()
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(Icon::new(icon).size_3())
                .child(face.text),
        )
        .tooltip_with_action(badge_tooltip(&clusters), &ToggleReadOnly, None);
    // The kit button draws its own border, so the dashed environment border is a frame around it.
    let frame = |inner: AnyElement| {
        div()
            .border_1()
            .border_dashed()
            .border_color(environment_color(environment, cx))
            .rounded(cx.theme().radius)
            .child(inner)
            .into_any_element()
    };
    if view.is_multi() {
        let shell = cx.weak_entity();
        return Some(frame(
            button
                .dropdown_menu(move |menu, _, _| {
                    clusters.iter().fold(menu, |menu, entry| {
                        let (target, shell) = (entry.cluster.clone(), shell.clone());
                        let label = format!("{} · {}", entry.environment.badge(), entry.label);
                        menu.item(
                            PopupMenuItem::new(label)
                                .checked(entry.lock == WriteLock::Locked)
                                .on_click(move |_, window, cx| {
                                    let _ = shell.update(cx, |shell, cx| {
                                        shell.toggle_write_lock(&target, window, cx);
                                    });
                                }),
                        )
                    })
                })
                .into_any_element(),
        ));
    }
    let target = first.cluster.clone();
    Some(frame(
        button
            .on_click(cx.listener(move |shell, _, window, cx| {
                shell.toggle_write_lock(&target, window, cx);
            }))
            .into_any_element(),
    ))
}

fn settings_button() -> impl IntoElement {
    Button::new("settings")
        .ghost()
        .small()
        .icon(Icon::new(IconName::Settings))
        .tooltip_with_action("Settings", &OpenSettings, None)
        .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx))
}

/// One line per notice (a corrupt settings file, a skipped kubeconfig). The click dismisses them.
fn notice_lines(shell: &AppShell, cx: &App) -> Vec<String> {
    AppSettings::notice(cx)
        .map(ToString::to_string)
        .into_iter()
        .chain(shell.notices(cx))
        .collect()
}

/// The warning icon with the notices as its tooltip; absent while there is nothing to read.
fn notices_button(shell: &AppShell, cx: &Context<AppShell>) -> Option<AnyElement> {
    let lines = notice_lines(shell, cx);
    if lines.is_empty() {
        return None;
    }
    let warning = cx.theme().warning;
    Some(
        Button::new("notices")
            .ghost()
            .small()
            .child(
                div()
                    .text_color(warning)
                    .child(Icon::new(IconName::TriangleAlert)),
            )
            .tooltip(lines.join("\n"))
            .on_click(cx.listener(|shell, _, _, cx| shell.dismiss_notices(cx)))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_shows_plus_n_and_primary_label() {
        let text = trigger_text("prod-eu", &["dev-a", "prod-eu", "stg-b"]);
        assert_eq!(text.label, "prod-eu");
        assert_eq!(text.plus.as_deref(), Some("+2"));
    }

    #[test]
    fn trigger_tooltip_lists_the_viewed_clusters() {
        let text = trigger_text("prod-eu", &["dev-a", "prod-eu", "stg-b"]);
        assert_eq!(
            text.tooltip.as_deref(),
            Some("Viewing: dev-a, prod-eu, stg-b")
        );
    }

    #[test]
    fn trigger_has_no_plus_or_tooltip_for_one_cluster() {
        let one = trigger_text("prod-eu", &["prod-eu"]);
        assert_eq!((one.plus, one.tooltip), (None, None));
    }
}

#[cfg(test)]
mod lock_badge_tests {
    use super::*;

    fn entry(label: &str, lock: WriteLock) -> LockedCluster {
        LockedCluster {
            cluster: ClusterRef {
                kubeconfig: std::path::PathBuf::from("test.yaml"),
                context: label.to_owned(),
            },
            label: label.to_owned(),
            environment: Environment::Staging,
            lock,
        }
    }

    fn face(locks: &[(&str, WriteLock)]) -> BadgeFace {
        let clusters: Vec<LockedCluster> = locks
            .iter()
            .map(|(label, lock)| entry(label, *lock))
            .collect();
        badge_face(&clusters)
    }

    #[test]
    fn badge_shows_the_lock_state() {
        use WriteLock::{Locked, Unlocked};
        let read_only = BadgeFace {
            lock: Locked,
            text: "Read-only".to_owned(),
        };
        assert_eq!(face(&[("stg-b", Locked)]), read_only);
        assert_eq!(face(&[("a", Locked), ("b", Locked)]), read_only);
        let unlocked = face(&[("stg-b", Unlocked)]);
        assert_eq!(
            (unlocked.lock, unlocked.text.as_str()),
            (Unlocked, "Unlocked")
        );
    }

    #[test]
    fn an_open_cluster_wins_over_locked_ones() {
        use WriteLock::{Locked, Unlocked};
        let one = face(&[("prod-a", Locked), ("stg-b", Unlocked)]);
        assert_eq!((one.lock, one.text.as_str()), (Unlocked, "Unlocked: stg-b"));
        let several = face(&[("prod-a", Locked), ("stg-b", Unlocked), ("dev-c", Unlocked)]);
        assert_eq!(
            (several.lock, several.text.as_str()),
            (Unlocked, "2 of 3 unlocked")
        );
        let all = face(&[("a", Unlocked), ("b", Unlocked)]);
        assert_eq!(all.text, "2 of 2 unlocked");
    }

    #[test]
    fn the_tooltip_lists_every_viewed_cluster() {
        let clusters = [
            entry("prod-a", WriteLock::Locked),
            entry("stg-b", WriteLock::Unlocked),
        ];
        assert_eq!(
            badge_tooltip(&clusters),
            "prod-a: Read-only\nstg-b: Unlocked"
        );
        assert_eq!(
            badge_tooltip(&clusters[1..]),
            "stg-b: Unlocked",
            "one cluster names only itself"
        );
    }
}
