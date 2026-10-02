use cluster::NamespaceScope;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex};
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement as _, Styled as _, div};

use crate::app_shell::AppShell;
use crate::cluster_session::namespaces_label;
use crate::namespace_picker::{PickerAnchor, namespace_picker as picker};
use crate::resource_actions::disabled_menu_item;

pub(crate) fn title_bar(shell: &AppShell, cx: &Context<AppShell>) -> impl IntoElement {
    TitleBar::new()
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
                .child(read_only_badge())
                .child(settings_button()),
        )
}

fn cluster_switcher(shell: &AppShell, cx: &Context<AppShell>) -> AnyElement {
    let contexts = shell.context_names();
    let active = shell
        .session()
        .map(|session| session.read(cx).context().to_owned());
    let label = active.clone().unwrap_or_else(|| "No cluster".to_owned());
    let shell_handle = cx.weak_entity();
    Button::new("cluster-switcher")
        .ghost()
        .small()
        .label(label)
        .dropdown_caret(true)
        .disabled(contexts.is_empty())
        .dropdown_menu(move |menu, _, _| {
            let menu = contexts.iter().fold(menu, |menu, name| {
                let target = name.clone();
                let shell_handle = shell_handle.clone();
                menu.item(
                    PopupMenuItem::new(name.clone())
                        .checked(active.as_deref() == Some(name.as_str()))
                        .on_click(move |_, _, cx| {
                            let _ = shell_handle
                                .update(cx, |shell, cx| shell.switch_context(&target, cx));
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
