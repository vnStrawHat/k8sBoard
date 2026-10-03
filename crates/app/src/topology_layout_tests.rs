use std::time::Instant;

use cluster::PodSummary;

use super::*;
use crate::topology_fixtures::{
    Fixture, Ref, crashing_pod, deployment, hpa, ingress, object, pod, pod_with,
};
use crate::topology_graph::{TopologyBuild, TopologyEdge, TopologyKind};

/// Canvas shapes: a narrow one keeps the bands in one band-column; a wide one spreads them.
const TALL: f32 = 0.1;
const WIDE: f32 = 1.7;

fn no_pins() -> HashMap<NodeId, GraphPoint> {
    HashMap::new()
}

fn components(graph: &TopologyGraph) -> TopologyLayout {
    layout(graph, GroupBy::Components, TALL, &no_pins(), None)
}

fn rect(graph: &TopologyGraph, layout: &TopologyLayout, id: &NodeId) -> GraphRect {
    let index = graph
        .nodes
        .iter()
        .position(|node| node.id == *id)
        .unwrap_or_else(|| panic!("no node {id:?}"));
    layout.rects[index]
}

fn column_x(column: usize) -> f32 {
    MARGIN + column as f32 * COLUMN_PITCH
}

fn rs_pods(owner: &str, count: usize) -> Vec<PodSummary> {
    (0..count)
        .map(|n| pod(&format!("{owner}-{n}"), &[], Some(("ReplicaSet", owner))))
        .collect()
}

#[test]
fn columns_follow_kind_table() {
    let graph = Fixture::default()
        .with_ingress(ingress("shop", &[("/", "web")], None, None))
        .with_service("web", &["app=web"])
        .with_deployment("api", 1, 1)
        .with_hpa(hpa("api-hpa", "Deployment", "api"))
        .with_replica_set("api-7d9f", Some("api"), 1, 1)
        .with_pod(pod(
            "api-7d9f-0",
            &["app=web"],
            Some(("ReplicaSet", "api-7d9f")),
        ))
        .graph();
    let layout = components(&graph);
    let x_of = |kind, name: &str| rect(&graph, &layout, &object(kind, name)).origin.x;
    assert_eq!(x_of(TopologyKind::Ingress, "shop"), column_x(0));
    assert_eq!(
        x_of(TopologyKind::HorizontalPodAutoscaler, "api-hpa"),
        column_x(0)
    );
    assert_eq!(x_of(TopologyKind::Service, "web"), column_x(1));
    assert_eq!(x_of(TopologyKind::Deployment, "api"), column_x(1));
    assert_eq!(x_of(TopologyKind::ReplicaSet, "api-7d9f"), column_x(2));
    assert_eq!(x_of(TopologyKind::Pod, "api-7d9f-0"), column_x(3));
}

#[test]
fn config_row_sits_under_band() {
    let graph = Fixture::default()
        .with_deployment("api", 1, 1)
        .with_replica_set("api-7d9f", Some("api"), 1, 1)
        .with_config_map("settings")
        .with_pods(
            rs_pods("api-7d9f", 3)
                .into_iter()
                .map(|pod| pod_with(pod, &[Ref::EnvConfigMap("settings")])),
        )
        .graph();
    let layout = components(&graph);
    let settings = rect(
        &graph,
        &layout,
        &object(TopologyKind::ConfigMap, "settings"),
    );
    let lowest_column = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.kind.placement() != Placement::ConfigRow)
        .map(|(index, _)| layout.rects[index].bottom())
        .fold(0., f32::max);
    assert_eq!(settings.origin.y, lowest_column + CONFIG_GAP);
}

#[test]
fn config_node_aligns_under_first_source() {
    let graph = Fixture::default()
        .with_deployment("api", 1, 1)
        .with_replica_set("api-7d9f", Some("api"), 1, 1)
        .with_config_map("settings")
        .with_secret("token", "Opaque")
        .with_pod(pod_with(
            pod("api-7d9f-0", &[], Some(("ReplicaSet", "api-7d9f"))),
            &[Ref::EnvConfigMap("settings"), Ref::EnvSecret("token")],
        ))
        .graph();
    let layout = components(&graph);
    // The pod's refs aggregate to the Deployment, which sits in column 1.
    let at = |kind, name: &str| rect(&graph, &layout, &object(kind, name)).origin.x;
    assert_eq!(at(TopologyKind::ConfigMap, "settings"), column_x(1));
    // The Secret finds that slot taken and takes the next one.
    assert_eq!(at(TopologyKind::Secret, "token"), column_x(2));
}

#[test]
fn config_row_collision_moves_right() {
    let graph = Fixture::default()
        .with_deployment("api", 1, 1)
        .with_config_map("a")
        .with_config_map("b")
        .with_config_map("c")
        .with_pod(pod_with(
            pod("api-0", &[], Some(("ReplicaSet", "api-1"))),
            &[
                Ref::EnvConfigMap("a"),
                Ref::EnvConfigMap("b"),
                Ref::EnvConfigMap("c"),
            ],
        ))
        .with_replica_set("api-1", Some("api"), 1, 1)
        .graph();
    let layout = components(&graph);
    let xs: Vec<f32> = ["a", "b", "c"]
        .iter()
        .map(|name| {
            rect(&graph, &layout, &object(TopologyKind::ConfigMap, name))
                .origin
                .x
        })
        .collect();
    assert_eq!(xs, [column_x(1), column_x(2), column_x(3)]);
}

#[test]
fn missing_ghost_takes_its_kind_placement() {
    let graph = Fixture::default()
        .with_ingress(ingress("shop", &[("/", "gone")], None, None))
        .with_pod(pod_with(
            pod("tool", &[], None),
            &[Ref::EnvConfigMap("nope")],
        ))
        .graph();
    let layout = components(&graph);
    let service_ghost = NodeId::Missing {
        kind: TopologyKind::Service,
        name: "gone".to_owned(),
    };
    let config_ghost = NodeId::Missing {
        kind: TopologyKind::ConfigMap,
        name: "nope".to_owned(),
    };
    let pod_rect = rect(&graph, &layout, &object(TopologyKind::Pod, "tool"));
    assert_eq!(rect(&graph, &layout, &service_ghost).origin.x, column_x(1));
    // The config ghost sits in the config row, under the pod that refers to it.
    let config = rect(&graph, &layout, &config_ghost);
    assert!(config.origin.y >= pod_rect.bottom() + CONFIG_GAP);
    assert_eq!(config.origin.x, column_x(3));
}

#[test]
fn components_become_bands_largest_first() {
    let graph = Fixture::default()
        // A component of two nodes, then one of three: the larger comes first.
        .with_deployment("small", 1, 1)
        .with_replica_set("small-1", Some("small"), 1, 1)
        .with_service("big", &["app=big"])
        .with_pods([
            pod("big-1", &["app=big"], None),
            pod("big-2", &["app=big"], None),
        ])
        .graph();
    let layout = components(&graph);
    let big = rect(&graph, &layout, &object(TopologyKind::Service, "big"));
    let small = rect(&graph, &layout, &object(TopologyKind::Deployment, "small"));
    assert!(big.origin.y < small.origin.y);
    assert_eq!(layout.bands.len(), 2);
}

#[test]
fn singletons_share_last_band() {
    let graph = Fixture::default()
        .with_service("web", &["app=web"])
        .with_pod(pod("web-1", &["app=web"], None))
        .with_deployment("lone-a", 1, 1)
        .with_deployment("lone-b", 1, 1)
        .graph();
    let layout = components(&graph);
    assert_eq!(layout.bands.len(), 2);
    let a = rect(&graph, &layout, &object(TopologyKind::Deployment, "lone-a"));
    let b = rect(&graph, &layout, &object(TopologyKind::Deployment, "lone-b"));
    // Both singles stand in one column of the last band.
    assert_eq!(a.origin.x, b.origin.x);
    assert_eq!(b.origin.y - a.origin.y, ROW_PITCH);
    let last = &layout.bands[1].rect;
    assert!(a.origin.y >= last.origin.y);
}

/// Edges that go between the same two columns cross when their ends are in opposite orders.
fn crossings(graph: &TopologyGraph, layout: &TopologyLayout) -> usize {
    let column =
        |index: usize| ((layout.rects[index].origin.x - MARGIN) / COLUMN_PITCH).round() as i32;
    let y = |index: usize| layout.rects[index].origin.y;
    let edges: Vec<&TopologyEdge> = graph
        .edges
        .iter()
        .filter(|edge| edge.relation != Relation::Mounts)
        .collect();
    let mut count = 0;
    for (n, a) in edges.iter().enumerate() {
        for b in &edges[n + 1..] {
            if (column(a.from), column(a.to)) != (column(b.from), column(b.to)) {
                continue;
            }
            if (y(a.from) - y(b.from)) * (y(a.to) - y(b.to)) < 0. {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn barycenter_untangles_shared_service_fixture() {
    // `a` selects the pods of `y` and `b` those of `x`: by name the lines cross.
    let graph = Fixture::default()
        .with_service("a", &["team=y"])
        .with_service("b", &["team=x"])
        .with_deployment("x", 1, 1)
        .with_deployment("y", 1, 1)
        .with_replica_set("rs-x", Some("x"), 1, 1)
        .with_replica_set("rs-y", Some("y"), 1, 1)
        .with_pod(pod("px", &["team=x"], Some(("ReplicaSet", "rs-x"))))
        .with_pod(pod("py", &["team=y"], Some(("ReplicaSet", "rs-y"))))
        .graph();
    let layout = layout(&graph, GroupBy::App, TALL, &no_pins(), None);
    assert_eq!(crossings(&graph, &layout), 0);
}

#[test]
fn layout_is_deterministic_under_shuffle() {
    let forward = Fixture::default()
        .with_service("web", &["app=web"])
        .with_deployment("api", 1, 1)
        .with_replica_set("api-1", Some("api"), 1, 1)
        .with_pods([
            pod("p1", &["app=web"], Some(("ReplicaSet", "api-1"))),
            pod("p2", &["app=web"], Some(("ReplicaSet", "api-1"))),
        ])
        .graph();
    let shuffled = Fixture::default()
        .with_pods([
            pod("p2", &["app=web"], Some(("ReplicaSet", "api-1"))),
            pod("p1", &["app=web"], Some(("ReplicaSet", "api-1"))),
        ])
        .with_replica_set("api-1", Some("api"), 1, 1)
        .with_deployment("api", 1, 1)
        .with_service("web", &["app=web"])
        .graph();
    assert_eq!(components(&forward).rects, components(&shuffled).rects);
}

#[test]
fn short_columns_are_centered() {
    let graph = Fixture::default()
        .with_deployment("api", 1, 1)
        .with_replica_set("api-1", Some("api"), 1, 1)
        .with_pods(rs_pods("api-1", 3))
        .graph();
    let layout = components(&graph);
    let first_pod = rect(&graph, &layout, &object(TopologyKind::Pod, "api-1-0"));
    let deployment = rect(&graph, &layout, &object(TopologyKind::Deployment, "api"));
    assert_eq!(deployment.origin.y - first_pod.origin.y, ROW_PITCH);
}

#[test]
fn extent_covers_nodes_and_margin() {
    let graph = Fixture::default()
        .with_deployment("api", 1, 1)
        .with_pods(rs_pods("api-1", 2))
        .graph();
    let layout = components(&graph);
    for rect in &layout.rects {
        assert!(rect.origin.x >= layout.extent.origin.x + MARGIN);
        assert!(rect.origin.y >= layout.extent.origin.y + MARGIN);
        assert!(rect.right() <= layout.extent.right() - MARGIN);
        assert!(rect.bottom() <= layout.extent.bottom() - MARGIN);
    }
    let empty = components(&Fixture::default().graph());
    assert_eq!(empty.extent.width, 2. * MARGIN);
}

#[test]
fn structure_ignores_status_changes() {
    let healthy = Fixture::default().with_pod(pod("p1", &[], None)).graph();
    let failing = Fixture::default()
        .with_pod(crashing_pod("p1", &[], None))
        .graph();
    assert_ne!(healthy.nodes[0].caption, failing.nodes[0].caption);
    assert_eq!(structure(&healthy), structure(&failing));
}

#[test]
fn structure_changes_with_new_pod() {
    let before = Fixture::default().with_pod(pod("p1", &[], None)).graph();
    let after = Fixture::default()
        .with_pods([pod("p1", &[], None), pod("p2", &[], None)])
        .graph();
    assert_ne!(structure(&before), structure(&after));
}

fn api_with(pods: usize) -> TopologyGraph {
    Fixture::default()
        .with_deployment("api", pods as u32, pods as u32)
        .with_replica_set("api-1", Some("api"), pods as u32, pods as u32)
        .with_pods(rs_pods("api-1", pods))
        .graph()
}

#[test]
fn new_pod_keeps_siblings_in_place() {
    let before = api_with(2);
    let first = components(&before);
    let after = api_with(3);
    let second = layout(&after, GroupBy::Components, TALL, &no_pins(), Some(&first));
    for node in &before.nodes {
        assert_eq!(
            rect(&before, &first, &node.id),
            rect(&after, &second, &node.id),
            "{:?}",
            node.id
        );
    }
    // The new pod is last in its column.
    let newest = rect(&after, &second, &object(TopologyKind::Pod, "api-1-2"));
    for sibling in ["api-1-0", "api-1-1"] {
        let sibling = rect(&after, &second, &object(TopologyKind::Pod, sibling));
        assert!(newest.origin.y > sibling.origin.y);
    }
}

#[test]
fn previous_drops_removed_ids() {
    let before = api_with(3);
    let first = components(&before);
    let after = Fixture::default()
        .with_deployment("api", 2, 2)
        .with_replica_set("api-1", Some("api"), 2, 2)
        .with_pods([
            pod("api-1-0", &[], Some(("ReplicaSet", "api-1"))),
            pod("api-1-2", &[], Some(("ReplicaSet", "api-1"))),
        ])
        .graph();
    let second = layout(&after, GroupBy::Components, TALL, &no_pins(), Some(&first));
    assert_eq!(second.rects.len(), after.nodes.len());
    let kept_first = rect(&after, &second, &object(TopologyKind::Pod, "api-1-0"));
    let kept_last = rect(&after, &second, &object(TopologyKind::Pod, "api-1-2"));
    assert!(kept_first.origin.y < kept_last.origin.y);
}

#[test]
fn app_bands_are_titled_and_sorted() {
    let graph = Fixture::default()
        .with_pods([
            pod("b-1", &["app=beta"], None),
            pod("a-1", &["app=alpha"], None),
        ])
        .graph();
    let layout = layout(&graph, GroupBy::App, TALL, &no_pins(), None);
    let titles: Vec<&str> = layout
        .bands
        .iter()
        .filter_map(|band| band.title.as_deref())
        .collect();
    assert_eq!(titles, ["alpha", "beta"]);
    let alpha = rect(&graph, &layout, &object(TopologyKind::Pod, "a-1"));
    let beta = rect(&graph, &layout, &object(TopologyKind::Pod, "b-1"));
    assert!(alpha.origin.y < beta.origin.y);
}

#[test]
fn ungrouped_band_is_last() {
    let graph = Fixture::default()
        .with_pods([pod("loose", &[], None), pod("z-1", &["app=zeta"], None)])
        .graph();
    let layout = layout(&graph, GroupBy::App, TALL, &no_pins(), None);
    let titles: Vec<&str> = layout
        .bands
        .iter()
        .filter_map(|band| band.title.as_deref())
        .collect();
    assert_eq!(titles, ["zeta", "Ungrouped"]);
}

#[test]
fn pinned_node_keeps_its_origin() {
    let graph = api_with(2);
    let free = components(&graph);
    let pinned_id = object(TopologyKind::Pod, "api-1-0");
    let pins = HashMap::from([(pinned_id.clone(), GraphPoint { x: 900., y: 500. })]);
    let pinned = layout(&graph, GroupBy::Components, TALL, &pins, None);
    assert_eq!(
        rect(&graph, &pinned, &pinned_id).origin,
        GraphPoint { x: 900., y: 500. }
    );
    // Nothing moves out of its way.
    let other = object(TopologyKind::Pod, "api-1-1");
    assert_eq!(rect(&graph, &pinned, &other), rect(&graph, &free, &other));
}

#[test]
fn topology_budget() {
    // 40 Services, 100 ReplicaSets (and their Deployments), 3,000 pods: 30 per ReplicaSet.
    let mut fixture = Fixture::default();
    for n in 0..40 {
        fixture = fixture.with_service(&format!("svc-{n:02}"), &[&format!("app=svc-{n:02}")]);
    }
    let mut pods = Vec::new();
    for n in 0..100 {
        let deployment_name = format!("dep-{n:03}");
        let set = format!("{deployment_name}-7d9f");
        fixture = fixture
            .with_deployment_summary(deployment(&deployment_name, 30, 30))
            .with_replica_set(&set, Some(&deployment_name), 30, 30);
        let app = format!("app=svc-{:02}", n % 40);
        for p in 0..30 {
            pods.push(pod(
                &format!("{set}-{p:02}"),
                &[&app],
                Some(("ReplicaSet", &set)),
            ));
        }
    }
    fixture = fixture.with_pods(pods);
    let started = Instant::now();
    let TopologyBuild::Graph(graph) = fixture.build() else {
        panic!("the realistic input is within the limits");
    };
    let built = layout(&graph, GroupBy::App, WIDE, &no_pins(), None);
    let elapsed = started.elapsed();
    eprintln!(
        "topology_budget: {elapsed:?} for {} nodes",
        graph.nodes.len()
    );
    assert_eq!(built.rects.len(), graph.nodes.len());
    assert!(
        elapsed.as_millis() <= 80,
        "build and layout took {elapsed:?}"
    );
}

#[test]
fn services_and_deployments_share_a_column_in_name_order() {
    let graph = Fixture::default()
        .with_service("zeta", &[])
        .with_deployment("alpha", 1, 1)
        .graph();
    let layout = components(&graph);
    // Kind order first: the Service sorts before the Deployment.
    let service = rect(&graph, &layout, &object(TopologyKind::Service, "zeta"));
    let deployment = rect(&graph, &layout, &object(TopologyKind::Deployment, "alpha"));
    assert!(service.origin.y < deployment.origin.y);
}

/// A graph of `bands` apps, each a Deployment with a ReplicaSet and `pods` pods.
fn apps(bands: usize, pods: usize) -> Fixture {
    let mut fixture = Fixture::default();
    for band in 0..bands {
        let app = format!("app-{band:02}");
        let set = format!("{app}-7d9f");
        let label = format!("app={app}");
        let mut owner = deployment(&app, pods as u32, pods as u32);
        owner.labels = vec![label.clone()];
        fixture = fixture
            .with_deployment_summary(owner)
            .with_replica_set(&set, Some(&app), pods as u32, pods as u32)
            .with_pods(
                (0..pods)
                    .map(|n| pod(&format!("{set}-{n}"), &[&label], Some(("ReplicaSet", &set)))),
            );
    }
    fixture
}

fn columns_of(layout: &TopologyLayout) -> usize {
    let lefts: std::collections::BTreeSet<i64> = layout
        .bands
        .iter()
        .map(|band| band.rect.origin.x.round() as i64)
        .collect();
    lefts.len()
}

#[test]
fn bands_flow_into_band_columns_to_match_a_wide_canvas() {
    let graph = apps(10, 2).graph();
    let tall = layout(&graph, GroupBy::App, TALL, &no_pins(), None);
    let wide = layout(&graph, GroupBy::App, WIDE, &no_pins(), None);
    assert_eq!(columns_of(&tall), 1);
    assert!(columns_of(&wide) > 1);
    // The wide layout is closer to the canvas shape than the stack.
    let aspect = |layout: &TopologyLayout| layout.extent.width / layout.extent.height;
    assert!((aspect(&wide) / WIDE).ln().abs() < (aspect(&tall) / WIDE).ln().abs());
}

#[test]
fn band_columns_do_not_overlap() {
    let graph = apps(10, 2).graph();
    let wide = layout(&graph, GroupBy::App, WIDE, &no_pins(), None);
    for (n, a) in wide.bands.iter().enumerate() {
        for b in &wide.bands[n + 1..] {
            let apart = a.rect.right() <= b.rect.origin.x
                || b.rect.right() <= a.rect.origin.x
                || a.rect.bottom() <= b.rect.origin.y
                || b.rect.bottom() <= a.rect.origin.y;
            assert!(apart);
        }
    }
}

#[test]
fn band_packing_is_deterministic() {
    let graph = apps(10, 2).graph();
    let first = layout(&graph, GroupBy::App, WIDE, &no_pins(), None);
    let second = layout(&graph, GroupBy::App, WIDE, &no_pins(), None);
    assert_eq!(first.rects, second.rects);
}

#[test]
fn adding_a_pod_moves_no_other_card() {
    // Three more Deployments make the workload column of `app-03` the tallest, so a new pod
    // does not make that band grow, and no band below it moves.
    let busy = || {
        let mut fixture = apps(10, 2);
        for name in ["app-03-x", "app-03-y", "app-03-z"] {
            let mut extra = deployment(name, 1, 1);
            extra.labels = vec!["app=app-03".to_owned()];
            fixture = fixture.with_deployment_summary(extra);
        }
        fixture
    };
    let before = busy().graph();
    let first = layout(&before, GroupBy::App, WIDE, &no_pins(), None);
    let after = busy()
        .with_pod(pod(
            "app-03-7d9f-9",
            &["app=app-03"],
            Some(("ReplicaSet", "app-03-7d9f")),
        ))
        .graph();
    let second = layout(&after, GroupBy::App, WIDE, &no_pins(), Some(&first));
    assert_eq!(columns_of(&second), columns_of(&first));
    for (n, (was, is)) in first.bands.iter().zip(&second.bands).enumerate() {
        assert_eq!(was.rect, is.rect, "band {n}");
    }
    for node in &before.nodes {
        assert_eq!(
            rect(&before, &first, &node.id),
            rect(&after, &second, &node.id),
            "{:?}",
            node.id
        );
    }
}

#[test]
fn a_new_band_joins_the_shortest_column_and_keeps_the_others() {
    let before = apps(6, 2).graph();
    let first = layout(&before, GroupBy::App, WIDE, &no_pins(), None);
    let after = apps(7, 2).graph();
    let second = layout(&after, GroupBy::App, WIDE, &no_pins(), Some(&first));
    for node in &before.nodes {
        let was = rect(&before, &first, &node.id);
        let is = rect(&after, &second, &node.id);
        assert_eq!(was.origin.x, is.origin.x, "{:?}", node.id);
    }
}

#[test]
fn config_row_wraps_after_the_last_slot() {
    let mut fixture = Fixture::default().with_deployment("api", 1, 1);
    let names: Vec<String> = (0..6).map(|n| format!("cm-{n}")).collect();
    for name in &names {
        fixture = fixture.with_config_map(name);
    }
    let refs: Vec<Ref> = names.iter().map(|name| Ref::EnvConfigMap(name)).collect();
    let graph = fixture
        .with_replica_set("api-1", Some("api"), 1, 1)
        .with_pod(pod_with(
            pod("api-1-0", &[], Some(("ReplicaSet", "api-1"))),
            &refs,
        ))
        .graph();
    let layout = components(&graph);
    let rects: Vec<GraphRect> = names
        .iter()
        .map(|name| rect(&graph, &layout, &object(TopologyKind::ConfigMap, name)))
        .collect();
    // Wanted slot 1: slots 1 to 3 of the first row, then slots 0 to 2 of the next.
    let xs: Vec<f32> = rects.iter().map(|rect| rect.origin.x).collect();
    let expected = [1, 2, 3, 0, 1, 2].map(|slot| MARGIN + slot as f32 * COLUMN_PITCH);
    assert_eq!(xs, expected);
    assert_eq!(rects[3].origin.y - rects[0].origin.y, ROW_PITCH);
    // No two cards overlap.
    for (n, a) in rects.iter().enumerate() {
        for b in &rects[n + 1..] {
            assert!(a.origin != b.origin);
        }
    }
    let band = &layout.bands[0];
    assert!(rects.iter().all(|rect| rect.bottom() <= band.rect.bottom()));
}
