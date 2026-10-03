use cluster::{SecretKey, ServiceSummary};

use super::*;
use crate::topology_fixtures::{
    Fixture, NAMESPACE, Ref, claim, crashing_pod, has_edge, hpa, ids, index_of, ingress, now,
    object, pod, pod_in, pod_with, secret, service,
};
use crate::topology_layout::structure;

fn node<'a>(graph: &'a TopologyGraph, id: &NodeId) -> &'a TopologyNode {
    &graph.nodes[index_of(graph, id)]
}

fn pods_of(owner_kind: &str, owner: &str, count: usize, labels: &[&str]) -> Vec<PodSummary> {
    (0..count)
        .map(|n| {
            pod(
                &format!("{owner}-{n:02}"),
                labels,
                Some((owner_kind, owner)),
            )
        })
        .collect()
}

#[test]
fn deployment_owns_replica_set_owns_pods() {
    let graph = Fixture::default()
        .with_deployment("api", 2, 2)
        .with_replica_set("api-7d9f", Some("api"), 2, 2)
        .with_pods(pods_of("ReplicaSet", "api-7d9f", 2, &[]))
        .graph();
    let api = object(TopologyKind::Deployment, "api");
    let set = object(TopologyKind::ReplicaSet, "api-7d9f");
    assert!(has_edge(&graph, &api, &set, Relation::Owns));
    for pod_name in ["api-7d9f-00", "api-7d9f-01"] {
        let pod = object(TopologyKind::Pod, pod_name);
        assert!(has_edge(&graph, &set, &pod, Relation::Owns));
    }
    assert_eq!(graph.edges.len(), 3);
}

#[test]
fn inactive_replica_set_is_hidden() {
    let graph = Fixture::default()
        .with_deployment("api", 2, 2)
        .with_replica_set("api-old", Some("api"), 0, 0)
        .with_replica_set("api-new", Some("api"), 2, 2)
        .graph();
    let names: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == TopologyKind::ReplicaSet)
        .map(|node| node.name.as_ref())
        .collect();
    assert_eq!(names, ["api-new"]);
}

#[test]
fn stateful_and_daemon_sets_own_pods() {
    let graph = Fixture::default()
        .with_stateful_set("db", 1, 1)
        .with_daemon_set("agent", 1, 1)
        .with_pod(pod("db-0", &[], Some(("StatefulSet", "db"))))
        .with_pod(pod("agent-x", &[], Some(("DaemonSet", "agent"))))
        .graph();
    assert!(has_edge(
        &graph,
        &object(TopologyKind::StatefulSet, "db"),
        &object(TopologyKind::Pod, "db-0"),
        Relation::Owns
    ));
    assert!(has_edge(
        &graph,
        &object(TopologyKind::DaemonSet, "agent"),
        &object(TopologyKind::Pod, "agent-x"),
        Relation::Owns
    ));
}

#[test]
fn service_routes_to_matching_pods() {
    let graph = Fixture::default()
        .with_service("web", &["app=web"])
        .with_pod(pod("web-1", &["app=web", "tier=front"], None))
        .with_pod(pod("web-2", &["app=web"], None))
        .with_pod(pod("db-1", &["app=db"], None))
        .with_pod(pod_in("other", "web-3", &["app=web"]))
        .graph();
    let web = object(TopologyKind::Service, "web");
    for matching in ["web-1", "web-2"] {
        let pod = object(TopologyKind::Pod, matching);
        assert!(has_edge(&graph, &web, &pod, Relation::RoutesTo));
    }
    let routed = graph
        .edges
        .iter()
        .filter(|edge| edge.relation == Relation::RoutesTo)
        .count();
    assert_eq!(routed, 2);
}

#[test]
fn selectorless_and_external_name_services_route_nowhere() {
    let external = ServiceSummary {
        service_type: "ExternalName".to_owned(),
        ..service("outside", &["app=web"])
    };
    let graph = Fixture::default()
        .with_service("manual", &[])
        .with_service_summary(external)
        .with_pod(pod("web-1", &["app=web"], None))
        .graph();
    assert!(graph.edges.is_empty());
    assert!(graph.checks.is_empty());
    assert!(
        !graph
            .nodes
            .iter()
            .any(|node| matches!(node.id, NodeId::NoPods { .. }))
    );
}

#[test]
fn ingress_routes_to_backend_and_default_services() {
    let graph = Fixture::default()
        .with_service("web", &[])
        .with_service("fallback", &[])
        .with_ingress(ingress("shop", &[("/", "web")], Some("fallback"), None))
        .graph();
    let from = object(TopologyKind::Ingress, "shop");
    for backend in ["web", "fallback"] {
        let to = object(TopologyKind::Service, backend);
        assert!(has_edge(&graph, &from, &to, Relation::RoutesTo));
    }
}

#[test]
fn ingress_tls_secret_is_a_mount() {
    let graph = Fixture::default()
        .with_ingress(ingress("shop", &[], None, Some("shop-tls")))
        .with_secret("shop-tls", "kubernetes.io/tls")
        .graph();
    assert!(has_edge(
        &graph,
        &object(TopologyKind::Ingress, "shop"),
        &object(TopologyKind::Secret, "shop-tls"),
        Relation::Mounts
    ));
}

#[test]
fn hpa_scales_its_target() {
    let graph = Fixture::default()
        .with_deployment("api", 2, 2)
        .with_hpa(hpa("api-hpa", "Deployment", "api"))
        .graph();
    assert!(has_edge(
        &graph,
        &object(TopologyKind::HorizontalPodAutoscaler, "api-hpa"),
        &object(TopologyKind::Deployment, "api"),
        Relation::Owns
    ));
}

#[test]
fn config_refs_aggregate_to_top_workload() {
    let owned = |name: &str| {
        pod_with(
            pod(name, &[], Some(("ReplicaSet", "api-7d9f"))),
            &[Ref::EnvConfigMap("settings")],
        )
    };
    let graph = Fixture::default()
        .with_deployment("api", 2, 2)
        .with_replica_set("api-7d9f", Some("api"), 2, 2)
        .with_config_map("settings")
        .with_pod(owned("api-7d9f-a"))
        .with_pod(owned("api-7d9f-b"))
        .graph();
    let settings = object(TopologyKind::ConfigMap, "settings");
    let mounts: Vec<&TopologyEdge> = graph
        .edges
        .iter()
        .filter(|edge| edge.relation == Relation::Mounts)
        .collect();
    assert_eq!(mounts.len(), 1);
    assert!(has_edge(
        &graph,
        &object(TopologyKind::Deployment, "api"),
        &settings,
        Relation::Mounts
    ));
}

#[test]
fn standalone_pod_keeps_its_refs() {
    let graph = Fixture::default()
        .with_config_map("settings")
        .with_pod(pod_with(
            pod("tool", &[], None),
            &[Ref::EnvFromConfigMap("settings")],
        ))
        .graph();
    assert!(has_edge(
        &graph,
        &object(TopologyKind::Pod, "tool"),
        &object(TopologyKind::ConfigMap, "settings"),
        Relation::Mounts
    ));
}

#[test]
fn env_env_from_projected_and_claim_refs_are_read() {
    let graph = Fixture::default()
        .with_config_map("from-env")
        .with_config_map("from-env-from")
        .with_config_map("mounted")
        .with_config_map("projected")
        .with_secret("env-secret", "Opaque")
        .with_claim(claim("data", "Bound", None))
        .with_pod(pod_with(
            pod("tool", &[], None),
            &[
                Ref::EnvConfigMap("from-env"),
                Ref::EnvFromConfigMap("from-env-from"),
                Ref::MountConfigMap("mounted"),
                Ref::ProjectedConfigMap("projected"),
                Ref::EnvSecret("env-secret"),
                Ref::MountClaim("data"),
            ],
        ))
        .graph();
    let tool = object(TopologyKind::Pod, "tool");
    for (kind, name) in [
        (TopologyKind::ConfigMap, "from-env"),
        (TopologyKind::ConfigMap, "from-env-from"),
        (TopologyKind::ConfigMap, "mounted"),
        (TopologyKind::ConfigMap, "projected"),
        (TopologyKind::Secret, "env-secret"),
        (TopologyKind::PersistentVolumeClaim, "data"),
    ] {
        assert!(
            has_edge(&graph, &tool, &object(kind, name), Relation::Mounts),
            "{name}"
        );
    }
}

#[test]
fn projected_and_pull_secrets_are_refs() {
    let graph = Fixture::default()
        .with_secret("token", "Opaque")
        .with_secret("registry", "kubernetes.io/dockerconfigjson")
        .with_secret("mounted", "Opaque")
        .with_pod(pod_with(
            pod("tool", &[], None),
            &[
                Ref::ProjectedSecret("token"),
                Ref::PullSecret("registry"),
                Ref::MountSecret("mounted"),
                Ref::EnvFromSecret("mounted"),
            ],
        ))
        .graph();
    let tool = object(TopologyKind::Pod, "tool");
    for name in ["token", "registry", "mounted"] {
        let target = object(TopologyKind::Secret, name);
        assert!(has_edge(&graph, &tool, &target, Relation::Mounts), "{name}");
    }
}

#[test]
fn kube_root_ca_is_ignored() {
    let graph = Fixture::default()
        .with_config_map("kube-root-ca.crt")
        .with_pod(pod_with(
            pod("tool", &[], None),
            &[Ref::ProjectedConfigMap("kube-root-ca.crt")],
        ))
        .graph();
    assert!(graph.edges.is_empty());
    assert!(graph.checks.is_empty());
    assert!(
        !graph
            .nodes
            .iter()
            .any(|node| node.kind == TopologyKind::ConfigMap)
    );
}

#[test]
fn unreferenced_config_is_hidden() {
    let graph = Fixture::default()
        .with_config_map("unused")
        .with_secret("helm-release", "helm.sh/release.v1")
        .with_claim(claim("bound-unused", "Bound", None))
        .graph();
    assert!(graph.nodes.is_empty());
}

#[test]
fn pending_claim_is_shown_without_refs() {
    let created = now() - std::time::Duration::from_secs(600);
    let graph = Fixture::default()
        .with_claim(claim("stuck", "Pending", Some(created)))
        .graph();
    let stuck = node(
        &graph,
        &object(TopologyKind::PersistentVolumeClaim, "stuck"),
    );
    assert_eq!(stuck.look, NodeLook::Plain);
    assert_eq!(stuck.caption, "PVC \u{b7} Pending");
}

#[test]
fn ref_with_loading_feed_is_not_checked() {
    let graph = Fixture::default()
        .loading(TopologyKind::ConfigMap)
        .with_pod(pod_with(
            pod("tool", &[], None),
            &[Ref::EnvConfigMap("settings")],
        ))
        .graph();
    let target = node(&graph, &object(TopologyKind::ConfigMap, "settings"));
    assert_eq!(target.look, NodeLook::Unchecked);
    assert_eq!(target.caption, "ConfigMap \u{b7} not checked");
    assert!(target.key.is_none());
    assert!(graph.checks.is_empty());
}

#[test]
fn ref_without_a_feed_is_not_checked() {
    // A kind whose chip is off has no feed at all; its targets are not looked up.
    let graph = Fixture::default()
        .without_feed(TopologyKind::ConfigMap)
        .with_pod(pod_with(
            pod("tool", &[], None),
            &[Ref::EnvConfigMap("settings")],
        ))
        .graph();
    let target = node(&graph, &object(TopologyKind::ConfigMap, "settings"));
    assert_eq!(target.look, NodeLook::Unchecked);
    assert!(graph.checks.is_empty());
}

#[test]
fn ref_with_off_feed_is_not_checked() {
    let graph = Fixture::default()
        .off(TopologyKind::Secret)
        .with_pod(pod_with(pod("tool", &[], None), &[Ref::EnvSecret("token")]))
        .graph();
    let target = node(&graph, &object(TopologyKind::Secret, "token"));
    assert_eq!(target.look, NodeLook::Unchecked);
    assert_eq!(target.caption, "Secret \u{b7} not checked");
    assert!(graph.checks.is_empty());
}

#[test]
fn pod_set_above_limit_becomes_group() {
    let labels = ["app=api"];
    let graph = Fixture::default()
        .with_service("api", &["app=api"])
        .with_replica_set("api-7d9f", None, 7, 7)
        .with_pods(pods_of(
            "ReplicaSet",
            "api-7d9f",
            POD_GROUP_LIMIT + 1,
            &labels,
        ))
        .graph();
    let group = NodeId::PodGroup {
        owner_kind: TopologyKind::ReplicaSet,
        owner: "api-7d9f".to_owned(),
    };
    let node = node(&graph, &group);
    assert_eq!(node.name, "7 pods");
    assert_eq!(node.caption, "7 running");
    assert!(has_edge(
        &graph,
        &object(TopologyKind::ReplicaSet, "api-7d9f"),
        &group,
        Relation::Owns
    ));
    assert!(has_edge(
        &graph,
        &object(TopologyKind::Service, "api"),
        &group,
        Relation::RoutesTo
    ));
    assert!(
        !graph
            .nodes
            .iter()
            .any(|node| node.kind == TopologyKind::Pod && matches!(node.id, NodeId::Object { .. }))
    );
}

#[test]
fn a_set_at_the_limit_stays_single() {
    let graph = Fixture::default()
        .with_pods(pods_of("ReplicaSet", "api-7d9f", POD_GROUP_LIMIT, &[]))
        .graph();
    assert_eq!(graph.nodes.len(), POD_GROUP_LIMIT);
    assert!(
        !graph
            .nodes
            .iter()
            .any(|node| matches!(node.id, NodeId::PodGroup { .. }))
    );
}

#[test]
fn bad_pods_stay_out_of_group() {
    let mut pods = pods_of("ReplicaSet", "api-7d9f", POD_GROUP_LIMIT + 1, &[]);
    pods.push(crashing_pod(
        "api-7d9f-bad",
        &[],
        Some(("ReplicaSet", "api-7d9f")),
    ));
    let graph = Fixture::default().with_pods(pods).graph();
    let bad = node(&graph, &object(TopologyKind::Pod, "api-7d9f-bad"));
    assert_eq!(bad.tone, Some(StatusTone::Bad));
    let group = graph
        .nodes
        .iter()
        .find(|node| matches!(node.id, NodeId::PodGroup { .. }))
        .expect("the healthy pods group");
    assert_eq!(group.objects, POD_GROUP_LIMIT + 1);
}

#[test]
fn pod_captions_name_status_and_container() {
    let graph = Fixture::default()
        .with_pod(crashing_pod("bad", &[], None))
        .with_pod(pod("good", &[], None))
        .graph();
    assert_eq!(
        node(&graph, &object(TopologyKind::Pod, "bad")).caption,
        "CrashLoopBackOff \u{b7} api"
    );
    assert_eq!(
        node(&graph, &object(TopologyKind::Pod, "good")).caption,
        "Running \u{b7} 1/1"
    );
}

#[test]
fn workload_captions_show_ready_counts() {
    let graph = Fixture::default()
        .with_deployment("api", 3, 2)
        .with_stateful_set("db", 1, 1)
        .with_replica_set("api-7d9f", Some("api"), 3, 3)
        .graph();
    assert_eq!(
        node(&graph, &object(TopologyKind::Deployment, "api")).caption,
        "Deployment \u{b7} 2/3"
    );
    assert_eq!(
        node(&graph, &object(TopologyKind::StatefulSet, "db")).caption,
        "StatefulSet \u{b7} 1/1"
    );
    assert_eq!(
        node(&graph, &object(TopologyKind::ReplicaSet, "api-7d9f")).caption,
        "ReplicaSet \u{b7} rev 3"
    );
}

#[test]
fn routed_service_caption_is_ingress_path() {
    let graph = Fixture::default()
        .with_service("web", &[])
        .with_service("db", &[])
        .with_ingress(ingress("shop", &[("/v2", "web")], None, None))
        .graph();
    assert_eq!(
        node(&graph, &object(TopologyKind::Service, "web")).caption,
        "Service \u{b7} /v2"
    );
    assert_eq!(
        node(&graph, &object(TopologyKind::Service, "db")).caption,
        "Service \u{b7} ClusterIP"
    );
}

#[test]
fn ids_and_edges_are_sorted_and_deduped() {
    let graph = Fixture::default()
        .with_deployment("b", 1, 1)
        .with_deployment("a", 1, 1)
        .with_service("web", &["app=web"])
        .with_pods(pods_of("ReplicaSet", "x", 2, &["app=web"]))
        .graph();
    let nodes = ids(&graph);
    let mut sorted = nodes.clone();
    sorted.sort();
    assert_eq!(nodes, sorted);
    let mut edges = graph.edges.clone();
    edges.sort();
    edges.dedup();
    assert_eq!(graph.edges, edges);
}

#[test]
fn input_order_does_not_change_graph() {
    let forward = Fixture::default()
        .with_deployment("a", 1, 1)
        .with_deployment("b", 1, 1)
        .with_service("web", &["app=web"])
        .with_pod(pod("p1", &["app=web"], None))
        .with_pod(pod("p2", &["app=web"], None))
        .graph();
    let backward = Fixture::default()
        .with_pod(pod("p2", &["app=web"], None))
        .with_pod(pod("p1", &["app=web"], None))
        .with_service("web", &["app=web"])
        .with_deployment("b", 1, 1)
        .with_deployment("a", 1, 1)
        .graph();
    assert_eq!(structure(&forward), structure(&backward));
}

#[test]
fn resources_count_group_members() {
    let graph = Fixture::default()
        .with_deployment("api", 7, 7)
        .with_pods(pods_of("ReplicaSet", "api-7d9f", POD_GROUP_LIMIT + 1, &[]))
        .graph();
    // One Deployment and a group of seven pods.
    assert_eq!(graph.resources, 1 + POD_GROUP_LIMIT + 1);
}

#[test]
fn raw_input_above_limit_is_too_large_without_building() {
    let pods = (0..=RAW_LIMIT).map(|n| pod(&format!("p{n}"), &[], None));
    let fixture = Fixture::default().with_pods(pods);
    let TopologyBuild::TooLarge(too_large) = fixture.build() else {
        panic!("expected too large");
    };
    assert_eq!(too_large, TooLarge::Objects(RAW_LIMIT + 1));
}

#[test]
fn too_large_above_node_limit() {
    let mut fixture = Fixture::default();
    for n in 0..=NODE_LIMIT {
        fixture = fixture.with_deployment(&format!("d{n:03}"), 1, 1);
    }
    let TopologyBuild::TooLarge(too_large) = fixture.build() else {
        panic!("expected too large");
    };
    assert_eq!(too_large, TooLarge::Nodes(NODE_LIMIT + 1));
}

#[test]
fn equal_inputs_build_equal_graphs() {
    let build = |ready: u32| {
        Fixture::default()
            .with_deployment("api", 2, ready)
            .with_pod(pod("p1", &[], None))
            .graph()
    };
    // The view repaints only when the graph differs: a changed count must show.
    assert_eq!(build(2), build(2));
    assert_ne!(build(2), build(1));
}

#[test]
fn secret_nodes_hold_names_only() {
    let mut keyed = secret("db-credentials", "Opaque");
    keyed.keys = vec![SecretKey {
        name: "password".to_owned(),
        size_bytes: 12,
        is_binary: false,
    }];
    let graph = Fixture::default()
        .with_secret_summary(keyed)
        .with_pod(pod_with(
            pod("tool", &[], None),
            &[Ref::EnvSecret("db-credentials")],
        ))
        .graph();
    let node = node(&graph, &object(TopologyKind::Secret, "db-credentials"));
    assert_eq!(node.name, "db-credentials");
    assert_eq!(node.caption, "Secret \u{b7} Opaque");
    let shown = format!("{node:?}");
    assert!(!shown.contains("password"));
}

#[test]
fn group_by_defaults_to_app_with_labels() {
    let pods = [pod("a", &["app.kubernetes.io/name=api"], None)];
    assert_eq!(resolve_group_by(None, NAMESPACE, &pods), GroupBy::App);
    let by_app = [pod("a", &["app=api"], None)];
    assert_eq!(resolve_group_by(None, NAMESPACE, &by_app), GroupBy::App);
    let by_k8s_app = [pod("a", &["k8s-app=api"], None)];
    assert_eq!(resolve_group_by(None, NAMESPACE, &by_k8s_app), GroupBy::App);
    // The user's choice wins.
    assert_eq!(
        resolve_group_by(Some(GroupBy::Components), NAMESPACE, &pods),
        GroupBy::Components
    );
}

#[test]
fn group_by_falls_back_to_components() {
    let unlabelled = [pod("a", &["tier=front"], None)];
    assert_eq!(
        resolve_group_by(None, NAMESPACE, &unlabelled),
        GroupBy::Components
    );
    // A pod of another namespace does not count.
    let elsewhere = [pod_in("other", "a", &["app=api"])];
    assert_eq!(
        resolve_group_by(None, NAMESPACE, &elsewhere),
        GroupBy::Components
    );
    assert_eq!(resolve_group_by(None, NAMESPACE, &[]), GroupBy::Components);
}

#[test]
fn service_matching_agrees_with_the_health_core() {
    use crate::kind_join::service_health;
    let pods = vec![
        pod("web-1", &["app=web"], None),
        pod("web-2", &["app=web", "tier=front"], None),
        pod("db-1", &["app=db"], None),
    ];
    for selector in [
        &["app=web"][..],
        &["app=db"],
        &["app=none"],
        &["app=web", "tier=front"],
    ] {
        let summary = service("svc", selector);
        let graph = Fixture::default()
            .with_service_summary(summary.clone())
            .with_pods(pods.clone())
            .graph();
        let routed = graph
            .edges
            .iter()
            .filter(|edge| edge.relation == Relation::RoutesTo)
            .count();
        let health = service_health(&summary, &pods, None);
        let expected = health.matching_pods.unwrap_or(0);
        // A Service that matches nothing is drawn with one ghost edge.
        assert_eq!(routed, expected.max(1), "{selector:?}");
    }
}

// ---- step 2: filters, groups, expansion ----

fn without(kind: KindFilter) -> TopologyFilter {
    let mut filter = TopologyFilter::everything();
    filter.kinds.remove(&kind);
    filter
}

#[test]
fn hidden_kind_filter_drops_nodes_and_edges() {
    let mut fixture = Fixture::default()
        .with_service("web", &["app=web"])
        .with_deployment("api", 1, 1)
        .with_pod(pod("web-1", &["app=web"], None))
        .with_ingress(ingress("shop", &[("/", "web")], None, None));
    fixture.filter = without(KindFilter::Service);
    let graph = fixture.graph();
    assert!(
        !graph
            .nodes
            .iter()
            .any(|node| node.kind == TopologyKind::Service)
    );
    // The edges of the Service went with it.
    assert!(graph.edges.is_empty());
    assert_eq!(graph.nodes.len(), 3);
}

#[test]
fn problems_only_keeps_neighbours() {
    let mut fixture = Fixture::default()
        .with_service("web", &["app=web"])
        .with_service("calm", &["app=calm"])
        .with_pod(crashing_pod("web-1", &["app=web"], None))
        .with_pod(pod("calm-1", &["app=calm"], None));
    fixture.filter.problems_only = true;
    let graph = fixture.graph();
    let kept: Vec<NodeId> = ids(&graph);
    assert!(kept.contains(&object(TopologyKind::Pod, "web-1")));
    assert!(kept.contains(&object(TopologyKind::Service, "web")));
    assert!(!kept.contains(&object(TopologyKind::Pod, "calm-1")));
    assert!(!kept.contains(&object(TopologyKind::Service, "calm")));
}

#[test]
fn expanded_group_shows_pods() {
    let mut fixture =
        Fixture::default().with_pods(pods_of("ReplicaSet", "api-7d9f", POD_GROUP_LIMIT + 2, &[]));
    fixture.expanded.insert(NodeId::PodGroup {
        owner_kind: TopologyKind::ReplicaSet,
        owner: "api-7d9f".to_owned(),
    });
    let graph = fixture.graph();
    assert_eq!(graph.nodes.len(), POD_GROUP_LIMIT + 2);
    assert!(
        !graph
            .nodes
            .iter()
            .any(|node| matches!(node.id, NodeId::PodGroup { .. }))
    );
}

#[test]
fn checks_follow_visible_nodes() {
    let mut fixture = Fixture::default()
        .with_service("web", &["app=missing"])
        .with_pod(pod("other", &["app=other"], None));
    let with_check = fixture.graph();
    assert_eq!(with_check.checks.len(), 1);
    fixture.filter = without(KindFilter::Service);
    let hidden = fixture.graph();
    assert!(hidden.checks.is_empty());
}

#[test]
fn app_group_from_labels() {
    let mut labelled = deployment_labelled("api", "shop-api");
    labelled.labels = vec!["app.kubernetes.io/name=shop-api".to_owned()];
    let graph = Fixture::default()
        .with_pod(pod("p", &["app=billing"], None))
        .graph();
    assert_eq!(
        node(&graph, &object(TopologyKind::Pod, "p"))
            .group
            .as_deref(),
        Some("billing")
    );
    let _ = labelled;
}

fn deployment_labelled(name: &str, _app: &str) -> cluster::DeploymentSummary {
    crate::topology_fixtures::deployment(name, 1, 1)
}

#[test]
fn app_group_from_neighbour() {
    let graph = Fixture::default()
        .with_service("web", &["app=web"])
        .with_pod(pod("web-1", &["app=web"], None))
        .graph();
    // The Service has no label of its own: it takes the group of its pod.
    assert_eq!(
        node(&graph, &object(TopologyKind::Service, "web"))
            .group
            .as_deref(),
        Some("web")
    );
}

#[test]
fn unlabelled_component_is_ungrouped() {
    let graph = Fixture::default()
        .with_deployment("api", 1, 1)
        .with_pod(pod("lonely", &[], None))
        .graph();
    assert!(graph.nodes.iter().all(|node| node.group.is_none()));
}
