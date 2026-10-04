use cluster::NamespaceScope;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Hsla, IntoElement, ParentElement as _, Styled as _, div, px,
};

use crate::app_shell::{AppShell, Screen};
use crate::cluster_registry::ClusterProfile;
use crate::cluster_session::namespaces_label;
use crate::cluster_switcher::cluster_switcher as switcher_popover;
use crate::environment::{cluster_color, environment_badge, environment_color};
use crate::issue_board::IssueSummary;
use crate::keymap::{OpenPalette, ToggleReadOnly};
use crate::namespace_picker::{PickerAnchor, namespace_picker as picker};
use crate::settings::AppSettings;
use crate::settings_window::OpenSettings;
use crate::shortcut_sheet::row_keys;
use crate::status_tone::tone_color;
use crate::write_guard::WriteLock;

/// The top border: the cluster's own color (its environment's unless the user picked one), while
/// the environment badge keeps the environment color.
fn border_color(profile: Option<&ClusterProfile>, cx: &App) -> Hsla {
    match profile {
        Some(profile) => cluster_color(profile.color, cx),
        None => cx.theme().title_bar_border,
    }
}

pub(crate) fn title_bar(shell: &AppShell, cx: &Context<AppShell>) -> impl IntoElement {
    // Always 3 px, so the layout does not shift when a session starts. GPUI has one border
    // color per element, so the kit's 1 px bottom border takes the same color.
    let border = border_color(shell.active_profile(cx).as_ref(), cx);
    // Linux draws its own X, which closes without asking the window: it asks the shell first, so a
    // node shell pod is deleted before the window goes (the hook of the platform window covers the
    // other platforms).
    let shell_handle = cx.weak_entity();
    TitleBar::new()
        .on_close_window(move |_, window, cx| {
            let _ = shell_handle.update(cx, |shell, cx| shell.close_main_window(window, cx));
        })
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

/// The open cluster as the trigger shows it: its environment badge and its display name.
fn cluster_switcher(shell: &AppShell, cx: &Context<AppShell>) -> AnyElement {
    let trigger = match shell.active_profile(cx) {
        Some(profile) => h_flex()
            .gap_2()
            .items_center()
            .child(environment_badge(profile.environment, cx))
            .child(profile.display_name)
            .into_any_element(),
        None => "No cluster".into_any_element(),
    };
    let trigger = Button::new("cluster-switcher")
        .ghost()
        .small()
        .child(trigger)
        .dropdown_caret(true);
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
    let Some(live) = shell.live(cx) else {
        return trigger.label("ns: —").disabled(true).into_any_element();
    };
    let label = scope_label(&live.scope);
    let trigger = trigger.label(label).dropdown_caret(true);
    picker(PickerAnchor::TitleBar, trigger, shell, cx)
}

/// The icon and the text of the badge for `lock`.
fn badge_face(lock: WriteLock) -> (IconName, &'static str) {
    match lock {
        WriteLock::Locked => (IconName::Lock, "Read-only"),
        WriteLock::Unlocked => (IconName::LockOpen, "Unlocked"),
    }
}

/// `{label}: Read-only` or `{label}: Unlocked`.
fn badge_tooltip(label: &str, lock: WriteLock) -> String {
    format!("{label}: {}", badge_face(lock).1)
}

/// The lock of the open cluster: a lock and `Read-only`, or an open lock and `Unlocked`, with a
/// dashed border in its environment color. A click toggles it. Hidden without a session.
fn write_lock_badge(shell: &AppShell, cx: &Context<AppShell>) -> Option<AnyElement> {
    let open = shell.active_session()?;
    let lock = open.session.read(cx).lock();
    let (icon, text) = badge_face(lock);
    let target = open.cluster.clone();
    let button = Button::new("write-lock")
        .ghost()
        .small()
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(Icon::new(icon).size_3())
                .child(text),
        )
        .tooltip_with_action(badge_tooltip(&open.label, lock), &ToggleReadOnly, None)
        .on_click(cx.listener(move |shell, _, window, cx| {
            shell.toggle_write_lock(&target, window, cx);
        }));
    // The kit button draws its own border, so the dashed environment border is a frame around it.
    Some(
        div()
            .border_1()
            .border_dashed()
            .border_color(environment_color(open.profile.environment, cx))
            .rounded(cx.theme().radius)
            .child(button)
            .into_any_element(),
    )
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

    #[gpui_kit::test]
    fn title_bar_border_uses_the_cluster_color(cx: &mut gpui_kit::TestAppContext) {
        use crate::cluster_registry::ClusterRegistry;
        use crate::environment::ClusterColor;
        cx.update(|cx| {
            gpui_kit::init(cx);
            let summary = cluster::ContextSummary {
                name: "prod-a".to_owned(),
                cluster: "prod-a".to_owned(),
                user: None,
                namespace: None,
                source: std::path::PathBuf::from("a.yaml"),
            };
            let mut registry = ClusterRegistry::default();
            let plain = registry.profile(&summary);
            assert_eq!(border_color(Some(&plain), cx), cx.theme().danger);
            registry
                .entry_mut(&crate::cluster_registry::ClusterRef::of(&summary))
                .color = Some(ClusterColor::Teal);
            let teal = registry.profile(&summary);
            assert_eq!(border_color(Some(&teal), cx), cx.theme().cyan);
            // The badge stays the environment color.
            assert_eq!(environment_color(teal.environment, cx), cx.theme().danger);
            assert_eq!(border_color(None, cx), cx.theme().title_bar_border);
        });
    }

    #[test]
    fn the_badge_shows_the_lock_state() {
        assert_eq!(badge_face(WriteLock::Locked), (IconName::Lock, "Read-only"));
        assert_eq!(
            badge_face(WriteLock::Unlocked),
            (IconName::LockOpen, "Unlocked")
        );
    }

    #[test]
    fn the_tooltip_names_the_cluster_and_its_state() {
        assert_eq!(
            badge_tooltip("prod-a", WriteLock::Locked),
            "prod-a: Read-only"
        );
        assert_eq!(
            badge_tooltip("stg-b", WriteLock::Unlocked),
            "stg-b: Unlocked"
        );
    }
}
