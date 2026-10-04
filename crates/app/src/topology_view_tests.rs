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

fn metrics_source() -> MetricsSource {
    MetricsSource::new(&cluster::MetricsSourceFields {
        namespace: "monitoring".to_owned(),
        service: "vmselect-x".to_owned(),
        port: "8481".to_owned(),
        scheme: cluster::MetricsScheme::Http,
        prefix: "/select/0/prometheus".to_owned(),
    })
    .expect("valid source")
}

fn metrics_in(
    source: SourceState,
    traffic_sources: TrafficSources,
) -> crate::cluster_metrics::ClusterMetrics {
    let mut metrics = crate::cluster_metrics::ClusterMetrics::new(
        crate::cluster_metrics::PodReview::Done(Ok(Vec::new())),
    );
    metrics.source = source;
    metrics.traffic_sources = traffic_sources;
    metrics
}

fn ready_source() -> SourceState {
    SourceState::Ready {
        source: metrics_source(),
        check: cluster::SourceCheck {
            latency: Duration::from_millis(30),
            cpu_series: 318,
        },
    }
}

fn button(source: SourceState, traffic_sources: TrafficSources) -> (bool, String) {
    let button = traffic_button(&metrics_in(source, traffic_sources));
    (button.is_enabled, button.tooltip)
}

fn pod_network() -> Vec<TrafficMetricSource> {
    TrafficMetricSource::detect(&BTreeSet::from([
        "container_network_receive_bytes_total".to_owned()
    ]))
}

#[test]
fn traffic_button_state_per_source_state() {
    const SHOWN: &str = "monitoring/vmselect-x:8481 /select/0/prometheus";
    let timeout = "the metrics backend did not answer within 20 s";
    assert_eq!(
        button(SourceState::None, TrafficSources::NotLoaded),
        (
            false,
            "Choose a metrics source in Settings \u{203a} Metrics".to_owned()
        )
    );
    assert_eq!(
        button(SourceState::Invalid, TrafficSources::NotLoaded),
        (
            false,
            "The metrics source in Settings is not valid".to_owned()
        )
    );
    let checking = SourceState::Checking {
        source: metrics_source(),
        _task: Task::ready(()),
    };
    assert_eq!(
        button(checking, TrafficSources::NotLoaded),
        (false, "Checking the metrics source\u{2026}".to_owned())
    );
    let failed = SourceState::Failed {
        source: metrics_source(),
        error: cluster::MetricsError::TimedOut,
    };
    assert_eq!(
        button(failed, TrafficSources::NotLoaded),
        (false, format!("Metrics source unreachable: {timeout}"))
    );
    assert_eq!(
        button(ready_source(), TrafficSources::NotLoaded),
        (true, format!("Show traffic from {SHOWN}"))
    );
    assert_eq!(
        button(ready_source(), TrafficSources::Loaded(Ok(Vec::new()))),
        (false, format!("No traffic metrics in {SHOWN}"))
    );
    assert_eq!(
        button(ready_source(), TrafficSources::Loaded(Ok(pod_network()))),
        (true, format!("Show traffic from {SHOWN}"))
    );
    assert_eq!(
        button(
            ready_source(),
            TrafficSources::Loaded(Err(cluster::MetricsError::TimedOut))
        ),
        (
            true,
            format!("Show traffic (the metric list failed: {timeout}; retrying)")
        )
    );
}

#[test]
fn chip_text_per_sources() {
    let at = "2024-05-01T10:00:05Z".parse().expect("valid time");
    let clock = clock_text(at, &jiff::tz::TimeZone::UTC);
    assert_eq!(clock, "10:00:05");
    let shown = |sources: &'static [TrafficSourceKind]| Some((sources, clock.as_str()));
    assert_eq!(
        traffic_chip_text(None, None),
        "Traffic \u{b7} loading\u{2026}"
    );
    assert_eq!(
        traffic_chip_text(shown(&[TrafficSourceKind::Istio]), None),
        "Traffic \u{b7} Istio \u{b7} last 5 min \u{b7} 10:00:05"
    );
    assert_eq!(
        traffic_chip_text(
            shown(&[TrafficSourceKind::Istio, TrafficSourceKind::PodNetwork]),
            None
        ),
        "Traffic \u{b7} Istio, pod network bytes \u{b7} last 5 min \u{b7} 10:00:05"
    );
    assert_eq!(
        traffic_chip_text(shown(&[TrafficSourceKind::PodNetwork]), None),
        "Traffic \u{b7} pod network bytes (per pod, not per connection) \u{b7} last 5 min \u{b7} 10:00:05"
    );
    assert_eq!(
        traffic_chip_text(shown(&[TrafficSourceKind::Istio]), Some("timed out")),
        "Traffic \u{b7} paused \u{b7} timed out"
    );
}

/// The `traffic_namespace()` graph installed in the view, laid out, in Traffic mode, with no
/// session (so no pod list: only the names that need no pod resolve).
fn traffic_view(cx: &mut gpui_kit::TestAppContext) -> Entity<TopologyView> {
    let view = view_of(cx);
    let graph = crate::topology_fixtures::traffic_namespace().graph();
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.namespace = Some(crate::topology_fixtures::NAMESPACE.to_owned());
            let arranged = lay_out(
                &graph,
                GroupBy::Components,
                1.6,
                &HashMap::new(),
                None,
                view.edge_shape,
            );
            view.layout = Some((structure(&graph), GroupBy::Components, Rc::new(arranged)));
            view.build = Some(Ok(Rc::new(graph)));
            view.set_mode(TopologyMode::Traffic, cx);
        });
    });
    view
}

fn failed_sample() -> TrafficSample {
    TrafficSample {
        at: jiff::Timestamp::now(),
        readings: vec![(
            crate::topology_traffic_fixture::istio_sample().readings[0].0,
            Err(cluster::MetricsError::TimedOut),
        )],
    }
}

#[gpui_kit::test]
fn failed_refresh_keeps_the_last_sample(cx: &mut gpui_kit::TestAppContext) {
    let view = traffic_view(cx);
    notifications(&view, cx, |view, cx| {
        view.apply_traffic_sample(crate::topology_traffic_fixture::istio_sample(), cx);
    });
    let sample = cx.update(|cx| {
        let view = view.read(cx);
        assert!(view.traffic.layer.is_some());
        assert_eq!(view.traffic.paused, None);
        view.traffic.sample.clone().expect("a sample")
    });
    notifications(&view, cx, |view, cx| {
        view.apply_traffic_sample(failed_sample(), cx);
    });
    cx.update(|cx| {
        let view = view.read(cx);
        assert!(view.traffic.layer.is_some(), "the older sample stays drawn");
        assert!(Rc::ptr_eq(
            view.traffic.sample.as_ref().expect("kept"),
            &sample
        ));
        assert_eq!(
            view.traffic.paused.as_deref(),
            Some("the metrics backend did not answer within 20 s")
        );
        let text = traffic_chip_text(None, view.traffic.paused.as_deref());
        assert_eq!(
            text,
            "Traffic \u{b7} paused \u{b7} the metrics backend did not answer within 20 s"
        );
    });
    // The next answer clears the pause.
    notifications(&view, cx, |view, cx| {
        view.apply_traffic_sample(crate::topology_traffic_fixture::istio_sample(), cx);
    });
    cx.update(|cx| assert_eq!(view.read(cx).traffic.paused, None));
}

#[gpui_kit::test]
fn relayout_reroutes_only_the_calls(cx: &mut gpui_kit::TestAppContext) {
    let view = traffic_view(cx);
    notifications(&view, cx, |view, cx| {
        view.apply_traffic_sample(crate::topology_traffic_fixture::istio_sample(), cx);
    });
    let before = cx.update(|cx| view.read(cx).traffic.layer.clone().expect("a layer"));
    assert_eq!(before.calls.len(), 2);
    notifications(&view, cx, |view, cx| {
        // A drag pins the StatefulSet `ledger`, which one of the calls starts from.
        let id = NodeId::Object {
            kind: TopologyKind::StatefulSet,
            name: "ledger".to_owned(),
        };
        let namespace = view.namespace.clone().expect("a namespace");
        view.pins
            .entry(String::new())
            .or_default()
            .entry(namespace)
            .or_default()
            .insert(id, GraphPoint { x: 40., y: 900. });
        view.relayout(cx);
    });
    let after = cx.update(|cx| view.read(cx).traffic.layer.clone().expect("a layer"));
    assert_eq!(after.calls, before.calls, "the same Calls edges");
    assert!(
        Rc::ptr_eq(&after.overlay, &before.overlay),
        "nothing else is computed again"
    );
    assert_ne!(
        after.call_routes, before.call_routes,
        "the routes follow the card"
    );
}

/// A Traffic view with a sample drawn and a request in flight.
fn view_with_fetch(cx: &mut gpui_kit::TestAppContext) -> Entity<TopologyView> {
    let view = traffic_view(cx);
    notifications(&view, cx, |view, cx| {
        view.apply_traffic_sample(crate::topology_traffic_fixture::istio_sample(), cx);
        view.traffic.fetch = Some(Task::ready(()));
    });
    cx.update(|cx| {
        let traffic = &view.read(cx).traffic;
        assert!(traffic.fetch.is_some() && traffic.layer.is_some());
    });
    view
}

fn assert_dropped(view: &Entity<TopologyView>, cx: &mut gpui_kit::TestAppContext) {
    cx.update(|cx| {
        let traffic = &view.read(cx).traffic;
        assert!(traffic.fetch.is_none(), "no request in flight");
        assert!(traffic.sample.is_none() && traffic.layer.is_none());
        assert_eq!(traffic.last_fetch, None, "the next tick fetches at once");
        assert_eq!(traffic.paused, None);
    });
}

#[gpui_kit::test]
fn hide_drops_the_fetch(cx: &mut gpui_kit::TestAppContext) {
    let view = view_with_fetch(cx);
    notifications(&view, cx, |view, cx| view.set_visible(true, cx));
    notifications(&view, cx, |view, cx| view.set_visible(false, cx));
    assert_dropped(&view, cx);
}

#[gpui_kit::test]
fn a_namespace_change_drops_the_fetch(cx: &mut gpui_kit::TestAppContext) {
    let view = view_with_fetch(cx);
    notifications(&view, cx, |view, cx| {
        view.change_namespace(Some("blog".to_owned()), cx);
    });
    assert_dropped(&view, cx);
    // The mode stays: the next tick fetches the other namespace.
    cx.update(|cx| assert_eq!(view.read(cx).mode, TopologyMode::Traffic));
}

#[gpui_kit::test]
fn leaving_traffic_drops_the_fetch(cx: &mut gpui_kit::TestAppContext) {
    let view = view_with_fetch(cx);
    notifications(&view, cx, |view, cx| {
        view.set_mode(TopologyMode::Resources, cx);
    });
    assert_dropped(&view, cx);
}

#[gpui_kit::test]
fn a_mode_switch_clears_the_layer(cx: &mut gpui_kit::TestAppContext) {
    let view = traffic_view(cx);
    notifications(&view, cx, |view, cx| {
        view.apply_traffic_sample(crate::topology_traffic_fixture::istio_sample(), cx);
    });
    notifications(&view, cx, |view, cx| {
        view.set_mode(TopologyMode::Resources, cx);
    });
    cx.update(|cx| {
        let view = view.read(cx);
        assert_eq!(view.mode, TopologyMode::Resources);
        assert!(view.traffic.layer.is_none());
    });
    // The same mode again changes nothing and repaints nothing.
    assert_eq!(
        notifications(&view, cx, |view, cx| {
            view.set_mode(TopologyMode::Resources, cx);
        }),
        0
    );
}

#[test]
fn chip_tooltip_lists_notes_then_outside_peers() {
    let overlay = TrafficOverlay {
        edges: Vec::new(),
        nodes: Vec::new(),
        sources: Vec::new(),
        outside: vec!["web/frontend".to_owned(), "unknown".to_owned()],
        notes: vec!["Istio: more than 2,000 series, the first 2,000 are read".to_owned()],
    };
    assert_eq!(
        traffic_chip_tooltip(&overlay).as_deref(),
        Some(
            "Istio: more than 2,000 series, the first 2,000 are read\n2 peers outside this namespace: web/frontend, unknown"
        )
    );
    let quiet = TrafficOverlay {
        outside: Vec::new(),
        notes: Vec::new(),
        ..overlay
    };
    assert_eq!(traffic_chip_tooltip(&quiet), None);
}
