use super::*;
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
