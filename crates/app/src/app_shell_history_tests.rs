//! Back and forward in a headless shell without a session: the places, the restore through the
//! setters, and what does and does not record.

use gpui_kit::{Entity, TestAppContext};

use super::app_shell_history::is_place_served;
use super::app_shell_tests::{open_shell_on, served_kind};
use super::*;
use crate::cluster_registry::ClusterRef;
use crate::drawer::{ContainerTab, DrawerNavigation, DrawerTab};
use crate::navigation_history::Place;
use crate::table_selection::{ClusterObject, ResourceKey};

fn open_shell(cx: &mut TestAppContext) -> Entity<AppShell> {
    open_shell_on("does-not-exist/kubeconfig.yml", &[], cx).1
}

/// `key` in a cluster of its own: these shells have no session, so the cluster is only a name.
fn object(key: ResourceKey) -> ClusterObject {
    let context = ContextSummary {
        name: "ctx".to_owned(),
        cluster: "cluster".to_owned(),
        user: None,
        namespace: None,
        source: std::path::PathBuf::from("test.yaml"),
    };
    ClusterObject::new(ClusterRef::of(&context), key)
}

fn pod(name: &str) -> ClusterObject {
    object(ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
    })
}

fn kind_object(kind: ResourceKind, name: &str) -> ClusterObject {
    object(ResourceKey::Kind {
        kind,
        namespace: Some("shop".to_owned()),
        name: name.to_owned(),
    })
}

fn filter_text(shell: &AppShell, cx: &App) -> Option<String> {
    shell.toolkit_state(cx).map(|state| state.text)
}

/// Pods screen, filter `api`, `api-0` open on Yaml with container `web` on its Logs sub-tab.
fn show_pod_drawer(shell: &Entity<AppShell>, cx: &mut TestAppContext) {
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            shell.show_screen(Screen::Pods, cx);
            shell.update_view(cx, |view| view.filter.text = "api".to_owned());
        })
    });
    // The `ClearSelection` events of `show_screen` run first.
    cx.run_until_parked();
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            shell.change_selection(Some(pod("api-0")), cx);
            shell.set_drawer_open(true, cx);
            shell.set_drawer_tab(DrawerTab::Yaml, cx);
            shell.select_container("web".to_owned(), cx);
            shell.set_container_tab(ContainerTab::Logs, cx);
        })
    });
}

fn reveal(shell: &Entity<AppShell>, object: ClusterObject, cx: &mut TestAppContext) {
    cx.update(|cx| shell.update(cx, |shell, cx| shell.reveal_object(object, cx)));
    cx.run_until_parked();
}

fn go_back(shell: &Entity<AppShell>, cx: &mut TestAppContext) {
    cx.update(|cx| shell.update(cx, |shell, cx| shell.go_back(cx)));
    cx.run_until_parked();
}

fn go_forward(shell: &Entity<AppShell>, cx: &mut TestAppContext) {
    cx.update(|cx| shell.update(cx, |shell, cx| shell.go_forward(cx)));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn back_restores_screen_selection_tabs_container_and_filter(cx: &mut TestAppContext) {
    let shell = open_shell(cx);
    show_pod_drawer(&shell, cx);
    let secret = kind_object(ResourceKind::Secrets, "credentials");
    reveal(&shell, secret.clone(), cx);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.screen, Screen::Kind(ResourceKind::Secrets));
        assert_eq!(shell.selected, Some(secret.clone()));
    });
    go_back(&shell, cx);
    shell.read_with(cx, |shell, cx| {
        assert_eq!(shell.screen, Screen::Pods);
        assert_eq!(shell.selected, Some(pod("api-0")));
        assert!(shell.drawer.is_open);
        assert_eq!(shell.drawer.tab, DrawerTab::Yaml);
        assert_eq!(shell.drawer.selected_container.as_deref(), Some("web"));
        assert_eq!(shell.drawer.container_tab, ContainerTab::Logs);
        assert_eq!(filter_text(shell, cx).as_deref(), Some("api"));
    });
}

#[gpui_kit::test]
fn forward_goes_to_the_place_back_left(cx: &mut TestAppContext) {
    let shell = open_shell(cx);
    show_pod_drawer(&shell, cx);
    let secret = kind_object(ResourceKind::Secrets, "credentials");
    reveal(&shell, secret.clone(), cx);
    go_back(&shell, cx);
    go_forward(&shell, cx);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.screen, Screen::Kind(ResourceKind::Secrets));
        assert_eq!(shell.selected, Some(secret));
        assert!(shell.drawer.is_open);
    });
}

#[gpui_kit::test]
fn a_place_without_a_selection_restores_the_screen_only(cx: &mut TestAppContext) {
    let shell = open_shell(cx);
    cx.update(|cx| shell.update(cx, |shell, cx| shell.show_screen(Screen::Overview, cx)));
    reveal(&shell, pod("api-0"), cx);
    shell.read_with(cx, |shell, _| assert!(shell.drawer.is_open));
    go_back(&shell, cx);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.screen, Screen::Overview);
        assert_eq!(shell.selected, None);
        assert!(!shell.drawer.is_open);
    });
}

#[gpui_kit::test]
fn revealing_the_shown_object_records_nothing(cx: &mut TestAppContext) {
    let shell = open_shell(cx);
    show_pod_drawer(&shell, cx);
    reveal(&shell, pod("api-0"), cx);
    shell.read_with(cx, |shell, _| {
        assert!(shell.navigation.previous().is_none());
    });
}

#[gpui_kit::test]
fn a_reveal_after_back_clears_forward(cx: &mut TestAppContext) {
    let shell = open_shell(cx);
    show_pod_drawer(&shell, cx);
    reveal(
        &shell,
        kind_object(ResourceKind::Secrets, "credentials"),
        cx,
    );
    go_back(&shell, cx);
    reveal(&shell, kind_object(ResourceKind::Services, "api"), cx);
    go_forward(&shell, cx);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.screen, Screen::Kind(ResourceKind::Services));
    });
}

#[gpui_kit::test]
fn actions_on_another_row_are_not_recorded(cx: &mut TestAppContext) {
    let shell = open_shell(cx);
    show_pod_drawer(&shell, cx);
    // The palette pairs and the row keys reach `reveal_then` through `when_selected`.
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            shell.open_drawer_tab(pod("api-1"), DrawerTab::Events, cx);
        })
    });
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert!(shell.navigation.previous().is_none());
    });
}

#[gpui_kit::test]
fn back_to_topology_sets_no_pending_reveal(cx: &mut TestAppContext) {
    let shell = open_shell(cx);
    let service = kind_object(ResourceKind::Services, "api");
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            shell.show_screen(Screen::Topology, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            shell.change_selection(Some(service.clone()), cx);
            shell.set_drawer_open(true, cx);
        })
    });
    reveal(&shell, pod("api-0"), cx);
    go_back(&shell, cx);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.screen, Screen::Topology);
        assert_eq!(shell.selected, Some(service));
        assert!(shell.drawer.is_open);
        assert_eq!(shell.pending_reveal, None);
    });
}

#[gpui_kit::test]
fn a_cluster_release_clears_the_history(cx: &mut TestAppContext) {
    let shell = open_shell(cx);
    show_pod_drawer(&shell, cx);
    reveal(
        &shell,
        kind_object(ResourceKind::Secrets, "credentials"),
        cx,
    );
    shell.read_with(cx, |shell, _| {
        assert!(shell.navigation.previous().is_some());
    });
    cx.update(|cx| shell.update(cx, |shell, cx| shell.release_all(cx)));
    shell.read_with(cx, |shell, _| {
        assert!(shell.navigation.previous().is_none());
    });
}

#[test]
fn a_removed_custom_kind_is_not_served() {
    let gone = Screen::Kind(ResourceKind::Custom(served_kind("widgets.x.io", false)));
    let place = |screen| Place {
        screen,
        selection: None,
        is_drawer_open: false,
        tab: DrawerTab::Overview,
        container: None,
        container_tab: ContainerTab::Info,
        filter: None,
    };
    let other = [served_kind("gadgets.x.io", false)];
    assert!(!is_place_served(&place(gone), Some(&other)));
    let same = [served_kind("widgets.x.io", false)];
    assert!(is_place_served(&place(gone), Some(&same)));
    // An unknown list proves nothing, and built-in screens are always served.
    assert!(is_place_served(&place(gone), None));
    assert!(is_place_served(&place(Screen::Pods), Some(&[])));
}

fn drawer_navigation(shell: &Entity<AppShell>, cx: &mut TestAppContext) -> DrawerNavigation {
    cx.update(|cx| shell.update(cx, |shell, cx| shell.drawer_navigation(cx)))
}

#[gpui_kit::test]
fn the_back_button_names_the_place_a_link_left(cx: &mut TestAppContext) {
    let shell = open_shell(cx);
    assert!(drawer_navigation(&shell, cx).back.is_none());
    show_pod_drawer(&shell, cx);
    reveal(
        &shell,
        kind_object(ResourceKind::Secrets, "credentials"),
        cx,
    );
    let back = drawer_navigation(&shell, cx).back.expect("a back target");
    assert_eq!(back.label, "api-0");
    assert_eq!(back.tooltip, "Back to Pod api-0 (Alt+Left)");
    go_back(&shell, cx);
    assert!(drawer_navigation(&shell, cx).back.is_none());
}

#[gpui_kit::test]
fn row_controls_show_only_on_screens_with_a_table_cursor(cx: &mut TestAppContext) {
    let shell = open_shell(cx);
    for (screen, is_shown) in [
        (Screen::Pods, true),
        (Screen::Nodes, true),
        (Screen::Kind(ResourceKind::Services), true),
        (Screen::Overview, false),
        (Screen::Topology, false),
        (Screen::PortForwarding, false),
        (Screen::Issues, false),
    ] {
        cx.update(|cx| shell.update(cx, |shell, cx| shell.show_screen(screen, cx)));
        cx.run_until_parked();
        let rows = drawer_navigation(&shell, cx).rows;
        assert_eq!(rows.is_some(), is_shown, "{screen:?}");
    }
}

#[gpui_kit::test]
fn row_controls_are_disabled_without_a_visible_subject(cx: &mut TestAppContext) {
    let shell = open_shell(cx);
    cx.update(|cx| shell.update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx)));
    cx.run_until_parked();
    let rows = drawer_navigation(&shell, cx).rows.expect("row controls");
    assert_eq!(rows.position, None);
}
