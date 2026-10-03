use gpui_kit::AppContext as _;

use super::*;
use crate::topology_fixtures::Fixture;

fn named(name: &str) -> NamespaceScope {
    NamespaceScope::Named(name.to_owned())
}

#[test]
fn default_namespace_follows_the_scope() {
    assert_eq!(default_namespace(&named("shop")), Some("shop".to_owned()));
    let several = NamespaceScope::Several(vec!["a".to_owned(), "b".to_owned()]);
    assert_eq!(default_namespace(&several), Some("a".to_owned()));
    assert_eq!(default_namespace(&NamespaceScope::All), None);
}

#[test]
fn a_namespace_that_left_the_scope_resets_to_the_default() {
    assert_eq!(
        resolve_namespace(Some("shop"), &named("blog")),
        Some("blog".to_owned())
    );
    assert_eq!(
        resolve_namespace(Some("shop"), &NamespaceScope::All),
        Some("shop".to_owned())
    );
    assert_eq!(resolve_namespace(None, &NamespaceScope::All), None);
}

#[test]
fn namespace_choices_list_the_scope_or_every_namespace() {
    assert_eq!(namespace_choices(&named("shop"), None), ["shop"]);
    let summary = |name: &str| NamespaceSummary {
        name: name.to_owned(),
        phase: cluster::NamespacePhase::Active,
        created_at: None,
        labels: Vec::new(),
        deleting_since: None,
        deletion_conditions: Vec::new(),
    };
    let loaded = [summary("zeta"), summary("alpha")];
    assert_eq!(
        namespace_choices(&NamespaceScope::All, Some(&loaded)),
        ["alpha", "zeta"]
    );
    assert!(namespace_choices(&NamespaceScope::All, None).is_empty());
}

#[test]
fn a_cut_png_says_its_percentage() {
    assert_eq!(saved_detail("a.png", None), "Saved to a.png");
    assert_eq!(saved_detail("a.png", Some(1.)), "Saved to a.png");
    assert_eq!(
        saved_detail("a.png", Some(0.5)),
        "Saved to a.png \u{b7} exported at 50%"
    );
    // An SVG is not scaled.
    assert_eq!(saved_detail("a.SVG", Some(0.5)), "Saved to a.SVG");
}

#[test]
fn the_header_says_loading_too_large_or_the_count() {
    assert_eq!(count_text("shop", None), "ns: shop \u{b7} loading\u{2026}");
    assert_eq!(
        count_text("shop", Some(&Err(TooLarge::Nodes(900)))),
        "ns: shop \u{b7} too large"
    );
    let graph = Fixture::default().with_deployment("api", 1, 1).graph();
    assert_eq!(
        count_text("shop", Some(&Ok(Rc::new(graph)))),
        "ns: shop \u{b7} 1 resources"
    );
}

fn service_key(name: &str) -> ResourceKey {
    ResourceKey::Kind {
        kind: crate::resource_kind::ResourceKind::Services,
        namespace: Some("shop".to_owned()),
        name: name.to_owned(),
    }
}

#[test]
fn a_selection_is_gone_only_from_a_feed_that_has_loaded() {
    let fixture = Fixture::default().with_service("web", &[]);
    let pods: Vec<PodSummary> = Vec::new();
    fixture.with_rows(|rows| {
        assert!(!is_selection_gone(&service_key("web"), "shop", &pods, rows));
        assert!(is_selection_gone(&service_key("api"), "shop", &pods, rows));
        // Another namespace is not this graph's business.
        assert!(!is_selection_gone(&service_key("api"), "blog", &pods, rows));
    });
}

#[test]
fn a_selection_waits_for_a_feed_that_is_not_ready() {
    let fixture = Fixture::default().loading(TopologyKind::Service);
    fixture.with_rows(|rows| {
        assert!(!is_selection_gone(&service_key("web"), "shop", &[], rows));
    });
}

#[test]
fn a_pod_is_gone_when_the_loaded_pods_lack_it() {
    let pods = vec![crate::topology_fixtures::pod("web-1", &[], None)];
    let key = |name: &str| ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
    };
    assert!(!is_selection_gone(&key("web-1"), "shop", &pods, &[]));
    assert!(is_selection_gone(&key("web-2"), "shop", &pods, &[]));
}

#[test]
fn too_large_states_name_their_limit() {
    assert!(too_large_text(TooLarge::Objects(6_000), "shop", false).contains("5000"));
    assert!(too_large_text(TooLarge::Nodes(900), "shop", false).contains("500"));
}

#[test]
fn topology_selected_picks_the_first_deployment() {
    let graph = Fixture::default()
        .with_service("web", &[])
        .with_deployment("zeta", 1, 1)
        .with_deployment("alpha", 1, 1)
        .graph();
    // The nodes are sorted by id, so the first Deployment is the one named alpha.
    let id = first_deployment(&graph).expect("a deployment");
    assert!(
        matches!(id, NodeId::Object { kind: TopologyKind::Deployment, ref name } if name == "alpha")
    );
    let none = Fixture::default().with_service("web", &[]).graph();
    assert_eq!(first_deployment(&none), None);
}

fn view_of(cx: &mut gpui_kit::TestAppContext) -> Entity<TopologyView> {
    cx.new(|cx| TopologyView::new(WeakEntity::new_invalid(), cx))
}

/// How many times the view notified while `act` ran.
fn notifications(
    view: &Entity<TopologyView>,
    cx: &mut gpui_kit::TestAppContext,
    act: impl FnOnce(&mut TopologyView, &mut Context<TopologyView>),
) -> usize {
    let count = Rc::new(std::cell::Cell::new(0));
    let seen = Rc::clone(&count);
    let _subscription = cx.update(|cx| cx.observe(view, move |_, _| seen.set(seen.get() + 1)));
    cx.update(|cx| view.update(cx, act));
    cx.run_until_parked();
    count.get()
}

fn node_id(name: &str) -> NodeId {
    NodeId::Object {
        kind: TopologyKind::Service,
        name: name.to_owned(),
    }
}

#[gpui_kit::test]
fn hover_notifies_only_on_change(cx: &mut gpui_kit::TestAppContext) {
    let view = view_of(cx);
    let enter = |view: &mut TopologyView, cx: &mut Context<TopologyView>| {
        view.set_hover(node_id("web"), true, cx);
    };
    assert_eq!(notifications(&view, cx, enter), 1);
    // The same card again: nothing changed.
    assert_eq!(notifications(&view, cx, enter), 0);
    // The leave of another card does not clear this one.
    let other = |view: &mut TopologyView, cx: &mut Context<TopologyView>| {
        view.set_hover(node_id("api"), false, cx);
    };
    assert_eq!(notifications(&view, cx, other), 0);
    let leave = |view: &mut TopologyView, cx: &mut Context<TopologyView>| {
        view.set_hover(node_id("web"), false, cx);
    };
    assert_eq!(notifications(&view, cx, leave), 1);
    assert_eq!(notifications(&view, cx, leave), 0);
}

#[gpui_kit::test]
fn hover_during_a_drag_is_recorded_and_painted_when_it_ends(cx: &mut gpui_kit::TestAppContext) {
    let view = view_of(cx);
    let count = notifications(&view, cx, |view, cx| {
        view.drag = Drag::Minimap;
        view.set_hover(node_id("web"), true, cx);
    });
    // The cards move under the pointer during a drag: no repaint for the hover.
    assert_eq!(count, 0);
    cx.update(|cx| assert_eq!(view.read(cx).hovered, Some(node_id("web"))));
    // The end of the drag repaints once, with the hover as it is.
    let count = notifications(&view, cx, |view, cx| {
        view.finish_drag(Point::default(), 1, cx);
    });
    assert_eq!(count, 1);
    cx.update(|cx| assert_eq!(view.read(cx).hovered, Some(node_id("web"))));
}

#[gpui_kit::test]
fn a_flow_timer_runs_only_while_edges_flow(cx: &mut gpui_kit::TestAppContext) {
    let view = view_of(cx);
    let has_timer =
        |cx: &mut gpui_kit::TestAppContext| cx.update(|cx| view.read(cx).flow_timer.is_some());
    assert!(!has_timer(cx));
    notifications(&view, cx, |view, cx| view.sync_flow(true, cx));
    assert!(has_timer(cx));
    // Painting again while edges flow keeps the one timer.
    notifications(&view, cx, |view, cx| view.sync_flow(true, cx));
    assert!(has_timer(cx));
    notifications(&view, cx, |view, cx| view.sync_flow(false, cx));
    assert!(!has_timer(cx));
}

#[gpui_kit::test]
fn a_namespace_change_clears_the_hover(cx: &mut gpui_kit::TestAppContext) {
    let view = view_of(cx);
    notifications(&view, cx, |view, cx| {
        view.set_hover(node_id("web"), true, cx);
    });
    notifications(&view, cx, |view, cx| {
        view.change_namespace(Some("blog".to_owned()), cx);
    });
    cx.update(|cx| assert!(view.read(cx).hovered.is_none()));
}

#[test]
fn the_minimap_shrinks_while_the_drawer_is_open() {
    let full = minimap_size(false);
    let compact = minimap_size(true);
    assert_eq!(full, (MINIMAP_WIDTH, MINIMAP_HEIGHT));
    assert!(compact.0 < full.0 && compact.1 < full.1);
    // The compact minimap keeps the shape of the full one.
    assert!((compact.0 / compact.1 - full.0 / full.1).abs() < 1e-3);
}

#[gpui_kit::test]
fn rbac_chip_is_enabled_and_toggles(cx: &mut gpui_kit::TestAppContext) {
    let view = view_of(cx);
    let has_rbac = |cx: &mut gpui_kit::TestAppContext| {
        cx.update(|cx| view.read(cx).filter.kinds.contains(&KindFilter::Rbac))
    };
    // Off by default, and a chip like the others: the toolbar draws one for every kind filter.
    assert!(!has_rbac(cx));
    assert!(KindFilter::ALL.contains(&KindFilter::Rbac));
    assert_eq!(chip_id(KindFilter::Rbac), "topology-chip-rbac");
    assert_eq!(
        KindFilter::Rbac.tooltip(),
        "Show service accounts, bindings, and roles"
    );
    notifications(&view, cx, |view, cx| view.toggle_kind(KindFilter::Rbac, cx));
    assert!(has_rbac(cx));
    notifications(&view, cx, |view, cx| view.toggle_kind(KindFilter::Rbac, cx));
    assert!(!has_rbac(cx));
}

#[gpui_kit::test]
fn the_rbac_screen_turns_the_chip_on(cx: &mut gpui_kit::TestAppContext) {
    let view = view_of(cx);
    notifications(&view, cx, |view, cx| view.set_rbac(true, cx));
    cx.update(|cx| assert!(view.read(cx).filter.kinds.contains(&KindFilter::Rbac)));
}

#[test]
fn click_on_feedless_row_reveals_instead_of_drawer() {
    let cluster_role = ResourceKey::Kind {
        kind: crate::resource_kind::ResourceKind::ClusterRoles,
        namespace: None,
        name: "cluster-admin".to_owned(),
    };
    // No row in any feed: the object is shown on its screen, whatever the click count.
    assert_eq!(
        card_click(Some(cluster_role.clone()), false, 1),
        CardClick::Reveal(cluster_role.clone())
    );
    // An object with a row opens the drawer on a click and is revealed by a double click.
    let service = service_key("web");
    assert_eq!(
        card_click(Some(service.clone()), true, 1),
        CardClick::Select(service.clone())
    );
    assert_eq!(
        card_click(Some(service.clone()), true, 2),
        CardClick::Reveal(service)
    );
    assert_eq!(card_click(None, true, 1), CardClick::Highlight);
}

#[test]
fn too_large_with_rbac_on_says_so() {
    let with = too_large_text(TooLarge::Nodes(900), "shop", true);
    assert!(
        with.ends_with(
            "Too many nodes with the RBAC layer on; turn RBAC off or pick a smaller namespace."
        ),
        "{with}"
    );
    assert!(with.contains("900") && with.contains("500"));
    // Without the layer, the hint stays the kind chips and Problems only.
    let without = too_large_text(TooLarge::Nodes(900), "shop", false);
    assert!(!without.contains("RBAC"));
    assert!(without.contains("Problems only"));
    // The object limit has its own text, whatever the chip.
    assert!(!too_large_text(TooLarge::Objects(6_000), "shop", true).contains("RBAC"));
}
