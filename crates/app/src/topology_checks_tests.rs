use cluster::WorkloadCondition;

use super::*;
use crate::topology_fixtures::{
    Fixture, Ref, claim, crashing_pod, has_edge, hpa, ingress, now, object, pod, pod_with,
};

fn rules(graph: &crate::topology_graph::TopologyGraph) -> Vec<CheckRule> {
    graph.checks.iter().map(|check| check.rule).collect()
}

fn only_check(graph: &crate::topology_graph::TopologyGraph) -> &ConfigCheck {
    assert_eq!(graph.checks.len(), 1, "{:?}", graph.checks);
    &graph.checks[0]
}

fn tool_with(refs: &[Ref]) -> PodSummary {
    pod_with(pod("tool", &[], None), refs)
}

#[test]
fn service_without_matching_pods_gets_ghost() {
    let graph = Fixture::default()
        .with_service("web", &["app=web"])
        .with_pod(pod("other", &["app=other"], None))
        .graph();
    let ghost = NodeId::NoPods {
        service: "web".to_owned(),
    };
    let node = graph
        .nodes
        .iter()
        .find(|node| node.id == ghost)
        .expect("the ghost");
    assert_eq!(node.look, NodeLook::Ghost);
    assert_eq!(node.caption, "0 pods match");
    assert_eq!(node.tone, Some(StatusTone::Bad));
    assert!(has_edge(
        &graph,
        &object(TopologyKind::Service, "web"),
        &ghost,
        Relation::RoutesTo
    ));
    let check = only_check(&graph);
    assert_eq!(check.rule, CheckRule::ServiceNoPods);
    assert_eq!(check.tone, StatusTone::Bad);
    assert_eq!(
        check.text,
        "Service web matches no pods (selector app=web)."
    );
}

#[test]
fn no_pods_check_waits_for_pods() {
    let graph = Fixture::default()
        .with_service("web", &["app=web"])
        .without_pods()
        .graph();
    assert!(graph.checks.is_empty());
    assert!(
        !graph
            .nodes
            .iter()
            .any(|node| matches!(node.id, NodeId::NoPods { .. }))
    );
}

#[test]
fn ingress_to_missing_service_is_bad() {
    let graph = Fixture::default()
        .with_ingress(ingress("shop", &[("/", "gone")], None, None))
        .graph();
    let ghost = NodeId::Missing {
        kind: TopologyKind::Service,
        name: "gone".to_owned(),
    };
    assert!(has_edge(
        &graph,
        &object(TopologyKind::Ingress, "shop"),
        &ghost,
        Relation::RoutesTo
    ));
    let check = only_check(&graph);
    assert_eq!(check.rule, CheckRule::IngressMissingService);
    assert_eq!(check.tone, StatusTone::Bad);
    assert_eq!(check.text, "Ingress shop routes to missing Service gone.");
}

#[test]
fn missing_config_map_and_secret_are_warn() {
    let graph = Fixture::default()
        .with_pod(tool_with(&[
            Ref::EnvConfigMap("nope"),
            Ref::EnvSecret("nada"),
        ]))
        .graph();
    assert_eq!(
        rules(&graph),
        [CheckRule::MissingConfigMap, CheckRule::MissingSecret]
    );
    assert!(
        graph
            .checks
            .iter()
            .all(|check| check.tone == StatusTone::Warn)
    );
    assert_eq!(
        graph.checks[0].text,
        "Pod tool references missing ConfigMap nope."
    );
    assert_eq!(
        graph.checks[1].text,
        "Pod tool references missing Secret nada."
    );
}

#[test]
fn missing_pull_secret_is_warn() {
    let graph = Fixture::default()
        .with_pod(tool_with(&[Ref::PullSecret("registry")]))
        .graph();
    let check = only_check(&graph);
    assert_eq!(check.rule, CheckRule::MissingSecret);
    assert_eq!(check.tone, StatusTone::Warn);
}

#[test]
fn missing_claim_is_bad() {
    let graph = Fixture::default()
        .with_pod(tool_with(&[Ref::MountClaim("gone")]))
        .graph();
    let check = only_check(&graph);
    assert_eq!(check.rule, CheckRule::MissingClaim);
    assert_eq!(check.tone, StatusTone::Bad);
    assert_eq!(check.text, "Pod tool mounts missing PVC gone.");
}

#[test]
fn pending_claim_after_grace() {
    let created = now() - std::time::Duration::from_secs(600);
    let graph = Fixture::default()
        .with_claim(claim("stuck", "Pending", Some(created)))
        .graph();
    let check = only_check(&graph);
    assert_eq!(check.rule, CheckRule::ClaimNotBound);
    assert_eq!(check.tone, StatusTone::Warn);
    assert_eq!(check.text, "PVC stuck is Pending for 10m.");
}

#[test]
fn pending_claim_within_grace_is_quiet() {
    let created = now() - std::time::Duration::from_secs(120);
    let graph = Fixture::default()
        .with_claim(claim("young", "Pending", Some(created)))
        .with_pod(tool_with(&[Ref::MountClaim("young")]))
        .graph();
    assert!(graph.checks.is_empty());
}

#[test]
fn lost_claim_is_bad() {
    let graph = Fixture::default()
        .with_claim(claim("data", "Lost", None))
        .with_pod(tool_with(&[Ref::MountClaim("data")]))
        .graph();
    // The WHY box of a lost claim says the same, so it adds no second check.
    let check = only_check(&graph);
    assert_eq!(check.rule, CheckRule::ClaimNotBound);
    assert_eq!(check.tone, StatusTone::Bad);
    assert_eq!(check.text, "PVC data lost its volume pv-1.");
}

#[test]
fn hpa_missing_target() {
    let graph = Fixture::default()
        .with_hpa(hpa("api-hpa", "Deployment", "gone"))
        .graph();
    let check = only_check(&graph);
    assert_eq!(check.rule, CheckRule::HpaMissingTarget);
    assert_eq!(check.tone, StatusTone::Warn);
    assert_eq!(check.text, "HPA api-hpa scales missing Deployment gone.");
}

#[test]
fn off_feed_skips_missing_checks() {
    let graph = Fixture::default()
        .off(TopologyKind::Service)
        .off(TopologyKind::ConfigMap)
        .with_ingress(ingress("shop", &[("/", "gone")], None, None))
        .with_pod(tool_with(&[Ref::EnvConfigMap("nope")]))
        .graph();
    assert!(graph.checks.is_empty());
    assert!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind != TopologyKind::Pod && node.kind != TopologyKind::Ingress)
            .all(|node| node.look == NodeLook::Unchecked)
    );
}

#[test]
fn why_box_tones_node_and_adds_check() {
    let mut scaler = hpa("api-hpa", "Deployment", "api");
    scaler.conditions = vec![WorkloadCondition {
        name: "ScalingActive".to_owned(),
        is_true: false,
        reason: Some("FailedGetResourceMetric".to_owned()),
        message: None,
    }];
    let graph = Fixture::default()
        .with_deployment("api", 1, 1)
        .with_hpa(scaler)
        .graph();
    let check = only_check(&graph);
    assert_eq!(check.rule, CheckRule::ObjectDiagnosis);
    assert_eq!(check.tone, StatusTone::Bad);
    assert_eq!(check.text, "HPA api-hpa: Metrics unavailable.");
    let node = graph
        .nodes
        .iter()
        .find(|node| node.id == check.node)
        .expect("the checked node");
    assert_eq!(node.tone, Some(StatusTone::Bad));
}

#[test]
fn ingress_missing_tls_box_is_not_duplicated() {
    let graph = Fixture::default()
        .with_ingress(ingress("shop", &[], None, Some("shop-tls")))
        .with_secret("other", "kubernetes.io/tls")
        .graph();
    // MissingSecret covers the absent secret; the certificate box adds nothing.
    let check = only_check(&graph);
    assert_eq!(check.rule, CheckRule::MissingSecret);
    assert_eq!(
        check.text,
        "Ingress shop references missing Secret shop-tls."
    );
}

#[test]
fn pod_failure_tones_without_check() {
    let graph = Fixture::default()
        .with_pod(crashing_pod("bad", &[], None))
        .graph();
    assert!(graph.checks.is_empty());
    assert_eq!(graph.nodes[0].tone, Some(StatusTone::Bad));
}

#[test]
fn checks_sort_bad_first() {
    let graph = Fixture::default()
        .with_pod(tool_with(&[
            Ref::EnvConfigMap("nope"),
            Ref::MountClaim("gone"),
        ]))
        .graph();
    // The missing ConfigMap sorts before the claim by rule, but the Bad tone leads.
    assert_eq!(
        rules(&graph),
        [CheckRule::MissingClaim, CheckRule::MissingConfigMap]
    );
}

#[test]
fn chip_labels_singular_and_plural() {
    let cases = [
        (
            CheckRule::ServiceNoPods,
            "1 Service matches no pods",
            "2 Services match no pods",
        ),
        (
            CheckRule::IngressMissingService,
            "1 Ingress to a missing Service",
            "2 Ingress routes to missing Services",
        ),
        (
            CheckRule::MissingConfigMap,
            "1 missing ConfigMap",
            "2 missing ConfigMaps",
        ),
        (
            CheckRule::MissingSecret,
            "1 missing Secret",
            "2 missing Secrets",
        ),
        (CheckRule::MissingClaim, "1 missing PVC", "2 missing PVCs"),
        (
            CheckRule::ClaimNotBound,
            "1 PVC not bound",
            "2 PVCs not bound",
        ),
        (
            CheckRule::HpaMissingTarget,
            "1 HPA without a target",
            "2 HPAs without targets",
        ),
        (
            CheckRule::ObjectDiagnosis,
            "1 object problem",
            "2 object problems",
        ),
    ];
    for (rule, one, many) in cases {
        assert_eq!(rule.chip_label(1), one);
        assert_eq!(rule.chip_label(2), many);
    }
}

#[test]
fn the_chip_names_one_rule_or_counts_the_problems() {
    assert_eq!(checks_chip(&[]), None);
    let one_rule = Fixture::default()
        .with_pod(tool_with(&[Ref::EnvConfigMap("a")]))
        .graph();
    assert_eq!(
        checks_chip(&one_rule.checks),
        Some(("1 missing ConfigMap".to_owned(), StatusTone::Warn))
    );
    let mixed = Fixture::default()
        .with_pod(tool_with(&[Ref::EnvConfigMap("a"), Ref::MountClaim("c")]))
        .graph();
    assert_eq!(
        checks_chip(&mixed.checks),
        Some(("2 config problems".to_owned(), StatusTone::Bad))
    );
}

#[test]
fn coverage_names_off_and_loading_feeds() {
    let rows = [
        (TopologyKind::Secret, FeedRows::Off),
        (TopologyKind::Service, FeedRows::Loading),
        (TopologyKind::Deployment, FeedRows::Ready(&[])),
    ];
    assert_eq!(
        topology_coverage(&rows).as_deref(),
        Some("Not checked: secrets (not permitted). Loading: services.")
    );
    let ready = [(TopologyKind::Deployment, FeedRows::Ready(&[]))];
    assert_eq!(topology_coverage(&ready), None);
}

#[test]
fn coverage_skips_chip_disabled_kinds() {
    // A kind whose chip is off has no entry, so it cannot be listed.
    let rows = [(TopologyKind::Service, FeedRows::Loading)];
    let note = topology_coverage(&rows).expect("a note");
    assert!(!note.contains("secrets") && !note.contains("configmaps"));
}

#[test]
fn coverage_says_watch_failed_only_for_a_failed_feed() {
    let rows = [
        (TopologyKind::Secret, FeedRows::Off),
        (TopologyKind::Service, FeedRows::Failed),
        (TopologyKind::Ingress, FeedRows::Loading),
    ];
    assert_eq!(
        topology_coverage(&rows).as_deref(),
        Some("Not checked: services (watch failed), secrets (not permitted). Loading: ingresses.")
    );
    let failed = [(TopologyKind::Service, FeedRows::Failed)];
    assert_eq!(
        topology_coverage(&failed).as_deref(),
        Some("Not checked: services (watch failed).")
    );
}

#[test]
fn a_failed_feed_draws_its_targets_unchecked() {
    let graph = Fixture::default()
        .failed(TopologyKind::ConfigMap)
        .with_pod(tool_with(&[Ref::EnvConfigMap("settings")]))
        .graph();
    assert!(graph.checks.is_empty());
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.look == NodeLook::Unchecked)
    );
}
