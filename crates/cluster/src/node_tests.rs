use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{
    NodeAddress, NodeCondition, NodeSpec, NodeStatus as ApiNodeStatus, NodeSystemInfo,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};

use super::*;

fn node_with_ready_condition(status: Option<&str>) -> Node {
    let conditions = status.map(|status| {
        vec![
            NodeCondition {
                type_: "MemoryPressure".to_owned(),
                status: "False".to_owned(),
                ..Default::default()
            },
            NodeCondition {
                type_: "Ready".to_owned(),
                status: status.to_owned(),
                ..Default::default()
            },
        ]
    });
    Node {
        status: Some(ApiNodeStatus {
            conditions,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn node_with_labels(labels: &[(&str, &str)]) -> Node {
    let labels: BTreeMap<String, String> = labels
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    Node {
        metadata: ObjectMeta {
            labels: Some(labels),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn taint(key: &str, value: Option<&str>, effect: &str) -> NodeTaint {
    NodeTaint {
        key: key.to_owned(),
        value: value.map(str::to_owned),
        effect: effect.to_owned(),
    }
}

fn address(type_: &str, address: &str) -> NodeAddress {
    NodeAddress {
        type_: type_.to_owned(),
        address: address.to_owned(),
    }
}

#[test]
fn ready_condition_true_is_ready() {
    let node = node_with_ready_condition(Some("True"));
    assert_eq!(node_summary(&node).status.readiness, NodeReadiness::Ready);
}

#[test]
fn ready_condition_false_is_not_ready() {
    let node = node_with_ready_condition(Some("False"));
    assert_eq!(
        node_summary(&node).status.readiness,
        NodeReadiness::NotReady
    );
}

#[test]
fn ready_condition_unknown_is_unknown() {
    for value in ["Unknown", "Surprise"] {
        let node = node_with_ready_condition(Some(value));
        assert_eq!(node_summary(&node).status.readiness, NodeReadiness::Unknown);
    }
}

#[test]
fn missing_ready_condition_is_unknown() {
    for node in [node_with_ready_condition(None), Node::default()] {
        assert_eq!(node_summary(&node).status.readiness, NodeReadiness::Unknown);
    }
}

#[test]
fn unschedulable_node_has_scheduling_disabled() {
    let scheduling = |unschedulable| {
        let node = Node {
            spec: Some(NodeSpec {
                unschedulable,
                ..Default::default()
            }),
            ..Default::default()
        };
        node_summary(&node).status.scheduling
    };
    assert_eq!(scheduling(Some(true)), NodeScheduling::Disabled);
    assert_eq!(scheduling(Some(false)), NodeScheduling::Enabled);
    assert_eq!(scheduling(None), NodeScheduling::Enabled);
    assert_eq!(
        node_summary(&Node::default()).status.scheduling,
        NodeScheduling::Enabled
    );
}

#[test]
fn roles_come_from_node_role_label_suffixes_sorted() {
    let node = node_with_labels(&[
        ("node-role.kubernetes.io/worker", ""),
        ("node-role.kubernetes.io/control-plane", ""),
    ]);
    assert_eq!(node_summary(&node).roles, ["control-plane", "worker"]);
}

#[test]
fn roles_include_kubernetes_io_role_label_value() {
    let node = node_with_labels(&[
        ("kubernetes.io/role", "master"),
        ("node-role.kubernetes.io/master", ""),
        ("node-role.kubernetes.io/edge", ""),
    ]);
    assert_eq!(node_summary(&node).roles, ["edge", "master"]);
}

#[test]
fn roles_ignore_empty_suffix_and_unrelated_labels() {
    let node = node_with_labels(&[
        ("node-role.kubernetes.io/", ""),
        ("kubernetes.io/role", ""),
        ("kubernetes.io/hostname", "node-1"),
        ("node-role.example.com/infra", ""),
    ]);
    assert!(node_summary(&node).roles.is_empty());
}

#[test]
fn roles_are_empty_without_role_labels() {
    assert!(node_summary(&Node::default()).roles.is_empty());
    assert!(node_summary(&node_with_labels(&[])).roles.is_empty());
}

#[test]
fn taint_with_value_displays_key_value_effect() {
    let taint = taint("dedicated", Some("gpu"), "NoSchedule");
    assert_eq!(taint.to_string(), "dedicated=gpu:NoSchedule");
}

#[test]
fn taint_without_value_displays_key_effect() {
    assert_eq!(taint("k", None, "NoExecute").to_string(), "k:NoExecute");
    assert_eq!(taint("k", Some(""), "NoExecute").to_string(), "k:NoExecute");
}

#[test]
fn taints_keep_api_order() {
    let node = Node {
        spec: Some(NodeSpec {
            taints: Some(vec![
                Taint {
                    key: "z".to_owned(),
                    effect: "NoSchedule".to_owned(),
                    ..Default::default()
                },
                Taint {
                    key: "a".to_owned(),
                    value: Some("1".to_owned()),
                    effect: "NoExecute".to_owned(),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        node_summary(&node).taints,
        [
            taint("z", None, "NoSchedule"),
            taint("a", Some("1"), "NoExecute")
        ]
    );
}

#[test]
fn internal_ip_is_first_internal_ip_address() {
    let node = Node {
        status: Some(ApiNodeStatus {
            addresses: Some(vec![
                address("Hostname", "node-1"),
                address("ExternalIP", "203.0.113.7"),
                address("InternalIP", "10.0.0.5"),
                address("InternalIP", "10.0.0.6"),
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(node_summary(&node).internal_ip.as_deref(), Some("10.0.0.5"));

    let without_internal = Node {
        status: Some(ApiNodeStatus {
            addresses: Some(vec![address("Hostname", "node-1")]),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(node_summary(&without_internal).internal_ip, None);
    assert_eq!(node_summary(&Node::default()).internal_ip, None);
}

#[test]
fn node_summary_reads_kubelet_version_and_creation_time() {
    let created: jiff::Timestamp = "2024-05-01T10:00:00Z".parse().expect("valid timestamp");
    let node = Node {
        metadata: ObjectMeta {
            name: Some("node-1".to_owned()),
            creation_timestamp: Some(Time(created)),
            ..Default::default()
        },
        status: Some(ApiNodeStatus {
            node_info: Some(NodeSystemInfo {
                kubelet_version: "v1.29.5".to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let summary = node_summary(&node);
    assert_eq!(summary.name, "node-1");
    assert_eq!(summary.kubelet_version, "v1.29.5");
    assert_eq!(summary.created_at, Some(created));

    assert_eq!(node_summary(&Node::default()).kubelet_version, "");
}
