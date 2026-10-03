use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use futures::Stream;
use k8s_openapi::api::core::v1::{Node, NodeCondition as ApiNodeCondition, Taint};
use kube::Api;

use crate::connection::{ClusterConnection, ClusterError};
use crate::container_spec::quantity_pairs;
use crate::dns_name::is_dns_subdomain;
use crate::event::optional_message;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{label_terms, non_empty};

const ROLE_LABEL_PREFIX: &str = "node-role.kubernetes.io/";
const LEGACY_ROLE_LABEL: &str = "kubernetes.io/role";
const NODE_RESOURCE_ORDER: [&str; 4] = ["cpu", "memory", "pods", "ephemeral-storage"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeSummary {
    pub name: String,
    pub status: NodeStatus,
    /// Sorted, deduplicated, possibly empty. No `worker` is inferred.
    pub roles: Vec<String>,
    /// In API order.
    pub taints: Vec<NodeTaint>,
    pub kubelet_version: String,
    pub internal_ip: Option<String>,
    pub created_at: Option<jiff::Timestamp>,
    /// In API order. `lastHeartbeatTime` is not kept: the kubelet refreshes it constantly and
    /// every refresh would change the summary.
    pub conditions: Vec<NodeCondition>,
    /// In API order.
    pub addresses: Vec<NodeAddress>,
    pub system: NodeSystemInfo,
    /// `cpu`, `memory`, `pods`, `ephemeral-storage`, then the rest by name.
    pub resources: Vec<NodeResource>,
    /// `key=value` terms in key order. Annotations are never read.
    pub labels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeCondition {
    /// The condition type, for example `MemoryPressure`.
    pub name: String,
    pub status: ConditionStatus,
    /// An empty reason is `None`.
    pub reason: Option<String>,
    /// URL userinfo hidden, then cut like event messages.
    pub message: Option<String>,
    /// `lastTransitionTime`.
    pub changed_at: Option<jiff::Timestamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionStatus {
    True,
    False,
    /// `Unknown` and any other text.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeAddress {
    /// `InternalIP`, `ExternalIP`, `Hostname`, and so on.
    pub kind: String,
    pub address: String,
}

/// `status.nodeInfo`; every field is empty when the node reports none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeSystemInfo {
    pub operating_system: String,
    pub architecture: String,
    pub os_image: String,
    pub kernel_version: String,
    pub container_runtime: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeResource {
    pub name: String,
    /// The quantity as written.
    pub capacity: Option<String>,
    pub allocatable: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeStatus {
    pub readiness: NodeReadiness,
    pub scheduling: NodeScheduling,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeReadiness {
    Ready,
    NotReady,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeScheduling {
    Enabled,
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeTaint {
    pub key: String,
    pub value: Option<String>,
    /// `NoSchedule`, `PreferNoSchedule`, or `NoExecute`.
    pub effect: String,
    /// When the taint was added; set by the node lifecycle controller for `NoExecute` taints and
    /// kept when a taint edit sends the list back.
    pub time_added: Option<jiff::Timestamp>,
}

/// What the node editors need, fresh from the server (0034): the summaries do not carry
/// `resourceVersion`, which changes with every status update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeEdit {
    pub taints: Vec<NodeTaint>,
    pub labels: BTreeMap<String, String>,
    pub resource_version: String,
}

impl fmt::Display for NodeTaint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.value.as_deref() {
            Some(value) if !value.is_empty() => {
                write!(formatter, "{}={}:{}", self.key, value, self.effect)
            }
            _ => write!(formatter, "{}:{}", self.key, self.effect),
        }
    }
}

impl ClusterConnection {
    /// Lists all nodes, ordered by name.
    pub async fn list_nodes(&self) -> Result<Vec<NodeSummary>, ClusterError> {
        let api = Api::<Node>::all(self.client().clone());
        let nodes = self.list_all(api, "listing nodes").await?;
        let mut summaries: Vec<_> = nodes.iter().map(node_summary).collect();
        summaries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(summaries)
    }

    /// Reads one node for the taint and label editors (a single GET).
    pub async fn node_for_edit(&self, name: &str) -> Result<NodeEdit, ClusterError> {
        const ACTION: &str = "reading a node for editing";
        // kube does not encode a name in the path.
        if !is_dns_subdomain(name) {
            return Err(self.unexpected_response(ACTION, "the node name is not valid"));
        }
        let api = Api::<Node>::all(self.client().clone());
        let node = self.run(ACTION, api.get(name)).await?;
        node_edit(&node)
            .ok_or_else(|| self.unexpected_response(ACTION, "the node has no resourceVersion"))
    }

    /// Watches all nodes. Yields batched snapshots ordered by name.
    pub fn watch_nodes(&self) -> impl Stream<Item = WatchUpdate<NodeSummary>> + Send + 'static {
        let api = Api::<Node>::all(self.client().clone());
        summary_watch(self, vec![(None, api)], "watching nodes", node_summary)
    }
}

pub(crate) fn node_summary(node: &Node) -> NodeSummary {
    let status = node.status.as_ref();
    NodeSummary {
        name: node.metadata.name.clone().unwrap_or_default(),
        status: NodeStatus {
            readiness: node_readiness(node),
            scheduling: node_scheduling(node),
        },
        roles: node_roles(node),
        taints: node
            .spec
            .iter()
            .flat_map(|spec| spec.taints.iter().flatten())
            .map(node_taint)
            .collect(),
        kubelet_version: status
            .and_then(|status| status.node_info.as_ref())
            .map(|info| info.kubelet_version.clone())
            .unwrap_or_default(),
        internal_ip: status
            .and_then(|status| status.addresses.as_ref())
            .and_then(|addresses| {
                addresses
                    .iter()
                    .find(|address| address.type_ == "InternalIP")
            })
            .map(|address| address.address.clone()),
        created_at: node.metadata.creation_timestamp.as_ref().map(|time| time.0),
        conditions: node_conditions(node),
        addresses: node_addresses(node),
        system: node_system_info(node),
        resources: node_resources(node),
        labels: label_terms(&node.metadata),
    }
}

fn node_conditions(node: &Node) -> Vec<NodeCondition> {
    node.status
        .iter()
        .flat_map(|status| status.conditions.iter().flatten())
        .map(node_condition)
        .collect()
}

fn node_condition(condition: &ApiNodeCondition) -> NodeCondition {
    NodeCondition {
        name: condition.type_.clone(),
        status: match condition.status.as_str() {
            "True" => ConditionStatus::True,
            "False" => ConditionStatus::False,
            _ => ConditionStatus::Unknown,
        },
        reason: non_empty(condition.reason.as_deref()),
        message: optional_message(condition.message.as_deref()),
        changed_at: condition.last_transition_time.as_ref().map(|time| time.0),
    }
}

fn node_addresses(node: &Node) -> Vec<NodeAddress> {
    node.status
        .iter()
        .flat_map(|status| status.addresses.iter().flatten())
        .map(|address| NodeAddress {
            kind: address.type_.clone(),
            address: address.address.clone(),
        })
        .collect()
}

fn node_system_info(node: &Node) -> NodeSystemInfo {
    node.status
        .as_ref()
        .and_then(|status| status.node_info.as_ref())
        .map_or_else(NodeSystemInfo::default, |info| NodeSystemInfo {
            operating_system: info.operating_system.clone(),
            architecture: info.architecture.clone(),
            os_image: info.os_image.clone(),
            kernel_version: info.kernel_version.clone(),
            container_runtime: info.container_runtime_version.clone(),
        })
}

/// Drops entries whose capacity and allocatable are both `0`, such as unused `hugepages-*`.
fn node_resources(node: &Node) -> Vec<NodeResource> {
    let status = node.status.as_ref();
    quantity_pairs(
        status.and_then(|status| status.capacity.as_ref()),
        status.and_then(|status| status.allocatable.as_ref()),
        &NODE_RESOURCE_ORDER,
    )
    .into_iter()
    .filter(|pair| !(pair.first.as_deref() == Some("0") && pair.second.as_deref() == Some("0")))
    .map(|pair| NodeResource {
        name: pair.name,
        capacity: pair.first,
        allocatable: pair.second,
    })
    .collect()
}

fn node_readiness(node: &Node) -> NodeReadiness {
    let ready_condition = node
        .status
        .iter()
        .flat_map(|status| status.conditions.iter().flatten())
        .find(|condition| condition.type_ == "Ready");
    match ready_condition.map(|condition| condition.status.as_str()) {
        Some("True") => NodeReadiness::Ready,
        Some("False") => NodeReadiness::NotReady,
        _ => NodeReadiness::Unknown,
    }
}

fn node_scheduling(node: &Node) -> NodeScheduling {
    let is_unschedulable = node.spec.as_ref().and_then(|spec| spec.unschedulable) == Some(true);
    if is_unschedulable {
        NodeScheduling::Disabled
    } else {
        NodeScheduling::Enabled
    }
}

/// Mirrors kubectl's `findNodeRoles`.
fn node_roles(node: &Node) -> Vec<String> {
    let Some(labels) = &node.metadata.labels else {
        return Vec::new();
    };
    let mut roles = BTreeSet::new();
    for (key, value) in labels {
        if let Some(role) = key.strip_prefix(ROLE_LABEL_PREFIX)
            && !role.is_empty()
        {
            roles.insert(role.to_owned());
        }
        if key == LEGACY_ROLE_LABEL && !value.is_empty() {
            roles.insert(value.clone());
        }
    }
    roles.into_iter().collect()
}

fn node_taint(taint: &Taint) -> NodeTaint {
    NodeTaint {
        key: taint.key.clone(),
        value: taint.value.clone(),
        effect: taint.effect.clone(),
        time_added: taint.time_added.as_ref().map(|time| time.0),
    }
}

fn node_edit(node: &Node) -> Option<NodeEdit> {
    let resource_version = node
        .metadata
        .resource_version
        .clone()
        .filter(|version| !version.is_empty())?;
    Some(NodeEdit {
        taints: node
            .spec
            .iter()
            .flat_map(|spec| spec.taints.iter().flatten())
            .map(node_taint)
            .collect(),
        labels: node.metadata.labels.clone().unwrap_or_default(),
        resource_version,
    })
}

#[cfg(test)]
#[path = "node_tests.rs"]
mod node_tests;
