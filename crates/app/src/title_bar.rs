use cluster::NamespaceScope;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, IntoElement, ParentElement as _, SharedString, Styled as _,
    WeakEntity, div, px,
};

use crate::app_shell::AppShell;
use crate::cluster_session::{ClusterSession, LiveList};
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
    let picker = Button::new("namespace-picker").ghost().small();
    let Some(session) = shell.session().cloned() else {
        return picker.label("ns: —").disabled(true).into_any_element();
    };
    let Some(live) = session.read(cx).live() else {
        return picker.label("ns: —").disabled(true).into_any_element();
    };
    let label = match &live.scope {
        NamespaceScope::All => "ns: all".to_owned(),
        NamespaceScope::Named(namespace) => format!("ns: {namespace}"),
    };
    let shell_handle = cx.weak_entity();
    picker
        .label(label)
        .dropdown_caret(true)
        .dropdown_menu(move |menu, _, cx| namespace_menu(menu, &session, &shell_handle, cx))
        .into_any_element()
}

/// Built when the picker opens, from the namespaces known at that moment.
fn namespace_menu(
    menu: PopupMenu,
    session: &Entity<ClusterSession>,
    shell: &WeakEntity<AppShell>,
    cx: &App,
) -> PopupMenu {
    let menu = menu.scrollable(true).max_h(px(360.));
    let Some(live) = session.read(cx).live() else {
        return menu;
    };
    let item = |name: &str, scope: NamespaceScope| scope_item(name, scope, &live.scope, shell);
    let menu = menu
        .item(item("All namespaces", NamespaceScope::All))
        .separator();
    match &live.namespaces {
        LiveList::Loading => menu.item(PopupMenuItem::new("Loading namespaces…").disabled(true)),
        LiveList::Failed { message } => menu
            .item(disabled_menu_item(
                "Could not list namespaces",
                message.clone().into(),
            ))
            .item(item(
                live.default_namespace(),
                NamespaceScope::Named(live.default_namespace().to_owned()),
            )),
        LiveList::Ready { items, .. } => items.iter().fold(menu, |menu, namespace| {
            menu.item(item(
                &namespace.name,
                NamespaceScope::Named(namespace.name.clone()),
            ))
        }),
    }
}

fn scope_item(
    label: &str,
    scope: NamespaceScope,
    active: &NamespaceScope,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let shell = shell.clone();
    PopupMenuItem::new(SharedString::from(label.to_owned()))
        .checked(*active == scope)
        .on_click(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| shell.set_namespace(scope.clone(), cx));
        })
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
