use super::*;
use cluster::{AccessDecision, AccessReport, AccessReview};
use gpui_kit::Task;

use crate::cluster_registry::ClusterRef;
use crate::resource_kind::ResourceKind;

fn place(screen: Screen) -> Place {
    Place {
        screen,
        selection: None,
        is_drawer_open: false,
        tab: DrawerTab::Overview,
        container: None,
        container_tab: ContainerTab::Info,
        filter: None,
    }
}

fn served(_: &Place) -> bool {
    true
}

#[test]
fn back_returns_the_recorded_place_and_forward_redoes_the_current() {
    let mut history = NavigationHistory::default();
    history.record(place(Screen::Pods));
    let back = history.back(place(Screen::Nodes), served);
    assert_eq!(back, Some(place(Screen::Pods)));
    assert!(history.previous().is_none());
    let forward = history.forward(place(Screen::Pods), served);
    assert_eq!(forward, Some(place(Screen::Nodes)));
    assert_eq!(history.previous(), Some(&place(Screen::Pods)));
}

#[test]
fn empty_stacks_return_nothing() {
    let mut history = NavigationHistory::default();
    assert_eq!(history.back(place(Screen::Pods), served), None);
    assert_eq!(history.forward(place(Screen::Pods), served), None);
    assert!(history.previous().is_none());
}

#[test]
fn a_new_record_clears_forward() {
    let mut history = NavigationHistory::default();
    history.record(place(Screen::Pods));
    history.back(place(Screen::Nodes), served);
    history.record(place(Screen::Issues));
    assert_eq!(history.forward(place(Screen::Topology), served), None);
}

#[test]
fn back_keeps_at_most_fifty_places() {
    let mut history = NavigationHistory::default();
    for index in 0..60 {
        let mut from = place(Screen::Pods);
        from.container = Some(index.to_string());
        history.record(from);
    }
    let mut seen = 0;
    let mut oldest = None;
    while let Some(back) = history.back(place(Screen::Nodes), served) {
        seen += 1;
        oldest = back.container;
    }
    assert_eq!(seen, 50);
    assert_eq!(oldest.as_deref(), Some("10"));
}

#[test]
fn forward_is_bounded_by_the_same_cap() {
    let mut history = NavigationHistory::default();
    for _ in 0..50 {
        history.record(place(Screen::Pods));
    }
    for _ in 0..50 {
        history.back(place(Screen::Nodes), served);
    }
    // Walking forward and back again must not grow either stack past the cap.
    for _ in 0..50 {
        history.forward(place(Screen::Nodes), served);
    }
    history.record(place(Screen::Issues));
    let mut seen = 0;
    while history.back(place(Screen::Nodes), served).is_some() {
        seen += 1;
    }
    assert_eq!(seen, 50);
}

#[test]
fn clear_forgets_both_stacks() {
    let mut history = NavigationHistory::default();
    history.record(place(Screen::Pods));
    history.record(place(Screen::Nodes));
    history.back(place(Screen::Issues), served);
    history.clear();
    assert!(history.previous().is_none());
    assert_eq!(history.forward(place(Screen::Pods), served), None);
}

#[test]
fn unserved_places_are_consumed_and_never_reach_forward() {
    let mut history = NavigationHistory::default();
    history.record(place(Screen::Pods));
    history.record(place(Screen::Issues));
    let back = history.back(place(Screen::Nodes), |place| place.screen != Screen::Issues);
    assert_eq!(back, Some(place(Screen::Pods)));
    // Only the current place was pushed forward, not the skipped one.
    assert_eq!(
        history.forward(place(Screen::Pods), served),
        Some(place(Screen::Nodes))
    );
    assert_eq!(history.forward(place(Screen::Pods), served), None);
}

#[test]
fn no_served_place_leaves_the_current_one_off_the_forward_stack() {
    let mut history = NavigationHistory::default();
    history.record(place(Screen::Issues));
    assert_eq!(history.back(place(Screen::Nodes), |_| false), None);
    assert_eq!(history.forward(place(Screen::Pods), served), None);
}

fn place_on(key: ResourceKey) -> Place {
    let cluster = ClusterRef {
        kubeconfig: std::path::PathBuf::from("test.yaml"),
        context: "ctx".to_owned(),
    };
    Place {
        selection: Some(ClusterObject::new(cluster, key.clone())),
        ..place(key.screen())
    }
}

fn service(name: &str) -> ResourceKey {
    ResourceKey::Kind {
        kind: ResourceKind::Services,
        namespace: Some("shop".to_owned()),
        name: name.to_owned(),
    }
}

#[test]
fn row_position_is_one_based_over_the_visible_rows() {
    assert_eq!(row_position(Some(0), 1), Some((1, 1)));
    assert_eq!(row_position(Some(11), 40), Some((12, 40)));
    assert_eq!(row_position(Some(39), 40), Some((40, 40)));
}

#[test]
fn row_position_is_none_without_a_visible_cursor_row() {
    assert_eq!(row_position(None, 40), None);
    assert_eq!(row_position(Some(40), 40), None);
    assert_eq!(row_position(Some(0), 0), None);
}

#[test]
fn back_label_is_the_object_name() {
    assert_eq!(place_on(service("api")).back_label(), "api");
}

#[test]
fn back_label_cuts_a_long_name_to_twenty_characters() {
    let label = place_on(service("a-very-long-service-name-indeed")).back_label();
    assert_eq!(label, "a-very-long-service…");
    assert_eq!(label.chars().count(), 20);
    let exact = "a".repeat(20);
    assert_eq!(place_on(service(&exact)).back_label(), exact);
}

#[test]
fn a_place_without_a_selection_is_named_by_its_screen() {
    assert_eq!(place(Screen::Overview).back_label(), "Overview");
    assert_eq!(
        place(Screen::Kind(ResourceKind::Services)).back_label(),
        "Services"
    );
    assert_eq!(
        place(Screen::Overview).back_tooltip(),
        "Back to Overview (Alt+Left)"
    );
}

#[test]
fn back_tooltip_names_the_kind_and_the_object() {
    assert_eq!(
        place_on(service("api")).back_tooltip(),
        "Back to Service api (Alt+Left)"
    );
    let pod = ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: "api-1".to_owned(),
    };
    assert_eq!(place_on(pod).back_tooltip(), "Back to Pod api-1 (Alt+Left)");
    let node = ResourceKey::Node {
        name: "n1".to_owned(),
    };
    assert_eq!(place_on(node).back_tooltip(), "Back to Node n1 (Alt+Left)");
}

#[test]
fn previous_is_the_place_back_would_restore() {
    let mut history = NavigationHistory::default();
    history.record(place_on(service("api")));
    assert_eq!(
        history.previous().map(Place::back_label).as_deref(),
        Some("api")
    );
}

fn access_denying(denied: &[AccessCheck]) -> AccessState {
    let reviews = AccessCheck::ALL
        .into_iter()
        .map(|check| AccessReview {
            check,
            decision: if denied.contains(&check) {
                AccessDecision::Denied { reason: None }
            } else {
                AccessDecision::Allowed
            },
        })
        .collect();
    AccessState::Known(AccessReport { reviews })
}

fn kind_key(kind: ResourceKind, namespace: Option<&str>, name: &str) -> ResourceKey {
    ResourceKey::Kind {
        kind,
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
    }
}

fn scope_of(names: &[&str]) -> NamespaceScope {
    NamespaceScope::of_namespaces(names.iter().map(|name| (*name).to_owned()))
}

#[test]
fn an_allowed_target_in_scope_is_revealed() {
    let access = access_denying(&[]);
    let target = kind_key(ResourceKind::Secrets, Some("shop"), "credentials");
    assert_eq!(
        link_step(&target, &access, &scope_of(&["shop"])),
        LinkStep::Reveal
    );
}

#[test]
fn a_denied_kind_is_refused_with_the_reason() {
    let access = access_denying(&[AccessCheck::ListSecrets]);
    let target = kind_key(ResourceKind::Secrets, Some("shop"), "credentials");
    assert_eq!(
        link_step(&target, &access, &NamespaceScope::All),
        LinkStep::Denied("Not permitted: list secrets in all namespaces".into())
    );
    assert_eq!(
        link_step(&target, &access, &scope_of(&["shop", "shop-b"])),
        LinkStep::Denied("Not permitted: list secrets in shop, shop-b".into())
    );
}

#[test]
fn denied_pods_and_nodes_are_refused() {
    let access = access_denying(&[AccessCheck::ListPods, AccessCheck::ListNodes]);
    let pod = ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: "api-1".to_owned(),
    };
    let node = ResourceKey::Node {
        name: "n1".to_owned(),
    };
    assert_eq!(
        link_step(&pod, &access, &NamespaceScope::All),
        LinkStep::Denied("Not permitted: list pods".into())
    );
    assert_eq!(
        link_step(&node, &access, &NamespaceScope::All),
        LinkStep::Denied("Not permitted: list nodes".into())
    );
}

#[test]
fn a_link_is_revealed_while_access_is_checking_or_unknown() {
    let checking = AccessState::Checking {
        _task: Task::ready(()),
    };
    let target = kind_key(ResourceKind::Secrets, Some("shop"), "credentials");
    for access in [checking, AccessState::Unknown] {
        assert_eq!(
            link_step(&target, &access, &NamespaceScope::All),
            LinkStep::Reveal
        );
    }
}

#[test]
fn a_cluster_scoped_target_ignores_the_namespace_scope() {
    let access = access_denying(&[]);
    let target = kind_key(ResourceKind::Namespaces, None, "shop");
    let node = ResourceKey::Node {
        name: "n1".to_owned(),
    };
    for scope in [scope_of(&["other"]), scope_of(&["a", "b"])] {
        assert_eq!(link_step(&target, &access, &scope), LinkStep::Reveal);
        assert_eq!(link_step(&node, &access, &scope), LinkStep::Reveal);
    }
}

#[test]
fn a_target_outside_the_scope_is_refused_with_its_namespace() {
    let access = access_denying(&[]);
    let service = kind_key(ResourceKind::Services, Some("team-b"), "payments-api");
    let pod = ResourceKey::Pod {
        namespace: "team-b".to_owned(),
        name: "payments-api-1".to_owned(),
    };
    let text = |name: &str| format!("{name} is in team-b, outside the scope").into();
    for scope in [scope_of(&["team-a"]), scope_of(&["team-a", "team-c"])] {
        assert_eq!(
            link_step(&service, &access, &scope),
            LinkStep::OutOfScope(text("payments-api"))
        );
        assert_eq!(
            link_step(&pod, &access, &scope),
            LinkStep::OutOfScope(text("payments-api-1"))
        );
    }
}

#[test]
fn every_namespace_is_in_the_all_scope() {
    let access = access_denying(&[]);
    let service = kind_key(ResourceKind::Services, Some("team-b"), "payments-api");
    assert_eq!(
        link_step(&service, &access, &NamespaceScope::All),
        LinkStep::Reveal
    );
}

#[test]
fn a_denied_kind_wins_over_an_out_of_scope_namespace() {
    let access = access_denying(&[AccessCheck::ListSecrets]);
    let target = kind_key(ResourceKind::Secrets, Some("team-b"), "credentials");
    assert!(matches!(
        link_step(&target, &access, &scope_of(&["team-a"])),
        LinkStep::Denied(_)
    ));
}
