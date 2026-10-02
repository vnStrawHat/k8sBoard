use cluster::NamespaceScope;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Hsla, IntoElement, ParentElement as _, Styled as _, div, px,
};

use crate::app_shell::{AppShell, Screen};
use crate::cluster_session::namespaces_label;
use crate::environment::{environment_badge, environment_color};
use crate::issue_board::IssueSummary;
use crate::namespace_picker::{PickerAnchor, namespace_picker as picker};
use crate::resource_actions::disabled_menu_item;
use crate::settings::AppSettings;
use crate::status_tone::tone_color;

pub(crate) fn title_bar(shell: &AppShell, cx: &Context<AppShell>) -> impl IntoElement {
    // Always 3 px, so the layout does not shift when a session starts. GPUI has one border
    // color per element, so the kit's 1 px bottom border takes the same color.
    let border = match shell.active_profile(cx) {
        Some(profile) => environment_color(profile.environment, cx),
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
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .children(notices_button(shell, cx))
                .child(read_only_badge())
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

fn cluster_switcher(shell: &AppShell, cx: &Context<AppShell>) -> AnyElement {
    let items = shell.switcher_items(cx);
    let trigger = match shell.active_profile(cx) {
        Some(profile) => h_flex()
            .gap_2()
            .items_center()
            .child(environment_badge(profile.environment, cx))
            .child(profile.display_name)
            .into_any_element(),
        None => "No cluster".into_any_element(),
    };
    let is_disabled = items.is_empty();
    let shell_handle = cx.weak_entity();
    Button::new("cluster-switcher")
        .ghost()
        .small()
        .child(trigger)
        .dropdown_caret(true)
        .disabled(is_disabled)
        .dropdown_menu(move |menu, _, _| {
            let menu = items.iter().fold(menu, |menu, item| {
                let target = item.cluster.clone();
                let shell_handle = shell_handle.clone();
                menu.item(
                    PopupMenuItem::new(item.label.clone())
                        .checked(item.is_active)
                        .on_click(move |_, _, cx| {
                            let _ = shell_handle
                                .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
                        }),
                )
            });
            menu.separator().item(disabled_menu_item(
                "Manage clusters…",
                "Settings window comes later".into(),
            ))
        })
        .into_any_element()
}

fn namespace_picker(shell: &AppShell, cx: &Context<AppShell>) -> AnyElement {
    let trigger = Button::new("namespace-picker").ghost().small();
    let Some(live) = shell.session().and_then(|session| session.read(cx).live()) else {
        return trigger.label("ns: —").disabled(true).into_any_element();
    };
    let label = match &live.scope {
        NamespaceScope::All => "ns: all".to_owned(),
        NamespaceScope::Named(namespace) => format!("ns: {namespace}"),
        NamespaceScope::Several(names) => format!("ns: {}", namespaces_label(names)),
    };
    let trigger = trigger.label(label).dropdown_caret(true);
    picker(PickerAnchor::TitleBar, trigger, shell, cx)
}

/// Always shown: this phase of the app never changes anything in a cluster.
fn read_only_badge() -> impl IntoElement {
    Tag::secondary().child(
        h_flex()
            .gap_1()
            .items_center()
            .child(Icon::new(IconName::Lock).size_3())
            .child("Read-only"),
    )
}

fn settings_button() -> impl IntoElement {
    Button::new("settings")
        .ghost()
        .small()
        .icon(Icon::new(IconName::Settings))
        .tooltip("Settings window comes later")
        .disabled(true)
}

/// One line per notice (a corrupt settings file, a skipped kubeconfig). The click dismisses them.
fn notice_lines(shell: &AppShell, cx: &App) -> Vec<String> {
    AppSettings::notice(cx)
        .map(ToString::to_string)
        .into_iter()
        .chain(shell.notices().iter().cloned())
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
