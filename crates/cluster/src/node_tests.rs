use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{
    NodeAddress, NodeCondition, NodeSpec, NodeStatus as ApiNodeStatus,
    NodeSystemInfo as ApiNodeSystemInfo,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
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
        time_added: None,
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
            node_info: Some(ApiNodeSystemInfo {
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

fn node_with_status(status: ApiNodeStatus) -> Node {
    Node {
        status: Some(status),
        ..Default::default()
    }
}

fn quantity_map(pairs: &[(&str, &str)]) -> Option<BTreeMap<String, Quantity>> {
    Some(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), Quantity((*value).to_owned())))
            .collect(),
    )
}

fn api_condition(type_: &str, status: &str) -> NodeCondition {
    NodeCondition {
        type_: type_.to_owned(),
        status: status.to_owned(),
        ..Default::default()
    }
}

#[test]
fn node_conditions_keep_status_reason_message_and_transition() {
    let changed = "2024-05-01T10:00:00Z".parse().expect("valid timestamp");
    let node = node_with_status(ApiNodeStatus {
        conditions: Some(vec![
            NodeCondition {
                reason: Some("KubeletReady".to_owned()),
                message: Some(" kubelet is posting ready status ".to_owned()),
                last_transition_time: Some(Time(changed)),
                ..api_condition("Ready", "True")
            },
            api_condition("MemoryPressure", "False"),
            api_condition("DiskPressure", "Unknown"),
            api_condition("PIDPressure", "Weird"),
        ]),
        ..Default::default()
    });
    let conditions = node_summary(&node).conditions;
    let listed: Vec<_> = conditions
        .iter()
        .map(|condition| (condition.name.as_str(), condition.status))
        .collect();
    assert_eq!(
        listed,
        [
            ("Ready", ConditionStatus::True),
            ("MemoryPressure", ConditionStatus::False),
            ("DiskPressure", ConditionStatus::Unknown),
            ("PIDPressure", ConditionStatus::Unknown),
        ]
    );
    assert_eq!(conditions[0].reason.as_deref(), Some("KubeletReady"));
    assert_eq!(
        conditions[0].message.as_deref(),
        Some("kubelet is posting ready status")
    );
    assert_eq!(conditions[0].changed_at, Some(changed));
    assert_eq!(conditions[1].reason, None);
    assert_eq!(conditions[1].message, None);
    assert_eq!(conditions[1].changed_at, None);
}

#[test]
fn node_conditions_ignore_heartbeat() {
    let at = |text: &str| Time(text.parse().expect("valid timestamp"));
    let node_at = |heartbeat: &str| {
        node_with_status(ApiNodeStatus {
            conditions: Some(vec![NodeCondition {
                last_heartbeat_time: Some(at(heartbeat)),
                last_transition_time: Some(at("2024-05-01T10:00:00Z")),
                ..api_condition("Ready", "True")
            }]),
            ..Default::default()
        })
    };
    assert_eq!(
        node_summary(&node_at("2024-05-02T10:00:00Z")),
        node_summary(&node_at("2024-05-02T10:00:10Z"))
    );
}

#[test]
fn node_addresses_keep_api_order() {
    let node = node_with_status(ApiNodeStatus {
        addresses: Some(vec![
            address("Hostname", "node-1"),
            address("InternalIP", "10.0.0.5"),
            address("ExternalIP", "203.0.113.9"),
        ]),
        ..Default::default()
    });
    let listed: Vec<_> = node_summary(&node)
        .addresses
        .into_iter()
        .map(|address| (address.kind, address.address))
        .collect();
    assert_eq!(
        listed,
        [
            ("Hostname".to_owned(), "node-1".to_owned()),
            ("InternalIP".to_owned(), "10.0.0.5".to_owned()),
            ("ExternalIP".to_owned(), "203.0.113.9".to_owned()),
        ]
    );
}

#[test]
fn node_system_info_reads_node_info() {
    let node = node_with_status(ApiNodeStatus {
        node_info: Some(ApiNodeSystemInfo {
            operating_system: "linux".to_owned(),
            architecture: "amd64".to_owned(),
            os_image: "Ubuntu 22.04.4 LTS".to_owned(),
            kernel_version: "5.15.0-105".to_owned(),
            container_runtime_version: "containerd://1.7.13".to_owned(),
            kubelet_version: "v1.29.5".to_owned(),
            ..Default::default()
        }),
        ..Default::default()
    });
    let summary = node_summary(&node);
    assert_eq!(summary.system.operating_system, "linux");
    assert_eq!(summary.system.architecture, "amd64");
    assert_eq!(summary.system.os_image, "Ubuntu 22.04.4 LTS");
    assert_eq!(summary.system.kernel_version, "5.15.0-105");
    assert_eq!(summary.system.container_runtime, "containerd://1.7.13");
    assert_eq!(summary.kubelet_version, "v1.29.5");

    assert_eq!(
        node_summary(&Node::default()).system,
        NodeSystemInfo::default()
    );
}

#[test]
fn node_resources_union_order_and_drop_zero_hugepages() {
    let node = node_with_status(ApiNodeStatus {
        capacity: quantity_map(&[
            ("pods", "110"),
            ("hugepages-2Mi", "0"),
            ("hugepages-1Gi", "0"),
            ("cpu", "4"),
            ("memory", "16Gi"),
            ("ephemeral-storage", "100Gi"),
            ("example.com/gpu", "2"),
        ]),
        allocatable: quantity_map(&[
            ("pods", "110"),
            ("hugepages-2Mi", "0"),
            ("hugepages-1Gi", "1Gi"),
            ("cpu", "3920m"),
            ("memory", "15Gi"),
        ]),
        ..Default::default()
    });
    let listed: Vec<_> = node_summary(&node)
        .resources
        .into_iter()
        .map(|resource| (resource.name, resource.capacity, resource.allocatable))
        .collect();
    let text = |value: &str| Some(value.to_owned());
    assert_eq!(
        listed,
        [
            ("cpu".to_owned(), text("4"), text("3920m")),
            ("memory".to_owned(), text("16Gi"), text("15Gi")),
            ("pods".to_owned(), text("110"), text("110")),
            ("ephemeral-storage".to_owned(), text("100Gi"), None),
            ("example.com/gpu".to_owned(), text("2"), None),
            ("hugepages-1Gi".to_owned(), text("0"), text("1Gi")),
        ]
    );
}

#[test]
fn node_labels_are_terms_and_annotations_are_absent() {
    let mut node = node_with_labels(&[("zone", "a"), ("arch", "amd64")]);
    node.metadata.annotations = Some(BTreeMap::from([(
        "owner".to_owned(),
        "SECRETANNOTATION".to_owned(),
    )]));
    let summary = node_summary(&node);
    assert_eq!(summary.labels, ["arch=amd64", "zone=a"]);
    assert!(!format!("{summary:?}").contains("SECRETANNOTATION"));
}

#[test]
fn node_condition_message_hides_url_userinfo() {
    let node = node_with_status(ApiNodeStatus {
        conditions: Some(vec![NodeCondition {
            message: Some("pull from https://ci:s3cret@registry.example.com/v2 failed".to_owned()),
            ..api_condition("ImagePullProblem", "True")
        }]),
        ..Default::default()
    });
    let message = node_summary(&node).conditions[0].message.clone();
    assert_eq!(
        message.as_deref(),
        Some("pull from https://<hidden>@registry.example.com/v2 failed")
    );
}

mod edit {
    use serde_json::{Value, json};

    use super::*;
    use crate::fake_api::FakeApi;
    use crate::object_write::WritePolicy;

    fn node_json() -> Value {
        json!({
            "apiVersion": "v1", "kind": "Node",
            "metadata": {
                "name": "wk-04", "resourceVersion": "9912",
                "labels": {"kubernetes.io/hostname": "wk-04", "team": "infra"},
            },
            "spec": {"taints": [
                {"key": "dedicated", "value": "ingress", "effect": "NoSchedule"},
                {
                    "key": "node.kubernetes.io/unreachable", "effect": "NoExecute",
                    "timeAdded": "2026-10-02T08:00:00Z",
                },
            ]},
        })
    }

    #[test]
    fn taint_keeps_time_added() {
        let node: Node = serde_json::from_value(node_json()).expect("a node");
        let taints = node_summary(&node).taints;
        assert_eq!(taints[0].time_added, None);
        assert_eq!(
            taints[1].time_added,
            Some("2026-10-02T08:00:00Z".parse().expect("a timestamp"))
        );
    }

    #[tokio::test]
    async fn node_for_edit_reads_taints_labels_and_version() {
        let (connection, api) =
            FakeApi::connection(WritePolicy::Blocked, |_| (200, node_json().to_string()));
        let edit = connection.node_for_edit("wk-04").await.expect("the node");
        assert_eq!(edit.resource_version, "9912");
        assert_eq!(edit.taints.len(), 2);
        assert_eq!(edit.labels.get("team").map(String::as_str), Some("infra"));
        let requests = api.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/api/v1/nodes/wk-04");
    }

    #[tokio::test]
    async fn node_without_a_resource_version_is_refused() {
        let body = json!({"apiVersion": "v1", "kind": "Node", "metadata": {"name": "wk-04"}});
        let (connection, _api) =
            FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.to_string()));
        let error = connection
            .node_for_edit("wk-04")
            .await
            .expect_err("no version");
        assert!(
            matches!(error, ClusterError::UnexpectedResponse { .. }),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn node_for_edit_refuses_an_unsafe_name_without_a_request() {
        let (connection, api) =
            FakeApi::connection(WritePolicy::Blocked, |_| (200, node_json().to_string()));
        connection
            .node_for_edit("a/../b")
            .await
            .expect_err("an unsafe name");
        assert!(api.requests().is_empty());
    }
}
