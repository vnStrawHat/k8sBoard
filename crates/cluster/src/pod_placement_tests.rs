use k8s_openapi::api::core::v1::Node;
use serde_json::{Value, json};

use super::*;
use crate::node::node_summary;

fn placement(spec: Value) -> PodPlacement {
    let pod: Pod =
        serde_json::from_value(json!({"metadata": {"name": "p"}, "spec": spec})).expect("a pod");
    PodPlacement::of(&pod)
}

fn node(labels: Value, taints: Value) -> NodeSummary {
    let node: Node = serde_json::from_value(json!({
        "metadata": {"name": "wk", "labels": labels},
        "spec": {"taints": taints},
    }))
    .expect("a node");
    node_summary(&node)
}

fn data_taint() -> Value {
    json!([{"key": "workload", "value": "data", "effect": "NoSchedule"}])
}

#[test]
fn an_untolerated_taint_keeps_the_pod_off_and_is_named_without_its_effect() {
    let tainted = node(json!({}), data_taint());
    assert_eq!(
        placement(json!({"containers": []})).misfit(&tainted),
        Some(Misfit::Taints(vec!["workload=data".to_owned()]))
    );
}

#[test]
fn a_toleration_by_value_or_exists_lets_the_pod_on() {
    let tainted = node(json!({}), data_taint());
    let by_value = placement(json!({"containers": [], "tolerations": [
        {"key": "workload", "operator": "Equal", "value": "data", "effect": "NoSchedule"},
    ]}));
    let by_key = placement(json!({"containers": [], "tolerations": [
        {"key": "workload", "operator": "Exists"},
    ]}));
    let everything = placement(json!({"containers": [], "tolerations": [{"operator": "Exists"}]}));
    let wrong_value = placement(json!({"containers": [], "tolerations": [
        {"key": "workload", "value": "web"},
    ]}));
    assert_eq!(by_value.misfit(&tainted), None);
    assert_eq!(by_key.misfit(&tainted), None);
    assert_eq!(everything.misfit(&tainted), None);
    assert!(wrong_value.misfit(&tainted).is_some());
}

#[test]
fn a_prefer_no_schedule_taint_does_not_keep_the_pod_off() {
    let soft = node(
        json!({}),
        json!([{"key": "spot", "effect": "PreferNoSchedule"}]),
    );
    assert_eq!(placement(json!({"containers": []})).misfit(&soft), None);
}

#[test]
fn a_node_selector_must_match_the_labels() {
    let pod = placement(json!({"containers": [], "nodeSelector": {"disk": "ssd"}}));
    assert_eq!(
        pod.misfit(&node(json!({"disk": "hdd"}), json!([]))),
        Some(Misfit::Selector)
    );
    assert_eq!(pod.misfit(&node(json!({"disk": "ssd"}), json!([]))), None);
}

#[test]
fn required_affinity_terms_are_alternatives_and_requirements_all_hold() {
    let pod = placement(json!({"containers": [], "affinity": {"nodeAffinity": {
        "requiredDuringSchedulingIgnoredDuringExecution": {"nodeSelectorTerms": [
            {"matchExpressions": [
                {"key": "zone", "operator": "In", "values": ["a", "b"]},
                {"key": "spot", "operator": "DoesNotExist"},
            ]},
            {"matchExpressions": [{"key": "pool", "operator": "Exists"}]},
        ]},
    }}}));
    assert_eq!(pod.misfit(&node(json!({"zone": "a"}), json!([]))), None);
    assert_eq!(pod.misfit(&node(json!({"pool": "x"}), json!([]))), None);
    assert_eq!(
        pod.misfit(&node(json!({"zone": "a", "spot": "1"}), json!([]))),
        Some(Misfit::Selector)
    );
    assert_eq!(
        pod.misfit(&node(json!({"zone": "c"}), json!([]))),
        Some(Misfit::Selector)
    );
}

#[test]
fn gt_and_lt_compare_numbers_and_not_in_accepts_a_missing_label() {
    let newer = placement(json!({"containers": [], "affinity": {"nodeAffinity": {
        "requiredDuringSchedulingIgnoredDuringExecution": {"nodeSelectorTerms": [
            {"matchExpressions": [{"key": "gen", "operator": "Gt", "values": ["3"]}]},
        ]},
    }}}));
    assert_eq!(newer.misfit(&node(json!({"gen": "4"}), json!([]))), None);
    assert!(
        newer
            .misfit(&node(json!({"gen": "3"}), json!([])))
            .is_some()
    );
    assert!(newer.misfit(&node(json!({}), json!([]))).is_some());
    let not_gpu = placement(json!({"containers": [], "affinity": {"nodeAffinity": {
        "requiredDuringSchedulingIgnoredDuringExecution": {"nodeSelectorTerms": [
            {"matchExpressions": [{"key": "gpu", "operator": "NotIn", "values": ["yes"]}]},
        ]},
    }}}));
    assert_eq!(not_gpu.misfit(&node(json!({}), json!([]))), None);
    assert!(
        not_gpu
            .misfit(&node(json!({"gpu": "yes"}), json!([])))
            .is_some()
    );
}

#[test]
fn the_taint_of_a_cordoned_node_is_not_held_against_the_pod() {
    let cordoned = node(
        json!({}),
        json!([{"key": "node.kubernetes.io/unschedulable", "effect": "NoSchedule"}]),
    );
    assert_eq!(placement(json!({"containers": []})).misfit(&cordoned), None);
}
