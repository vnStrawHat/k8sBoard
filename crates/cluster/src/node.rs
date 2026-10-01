use std::collections::BTreeSet;
use std::fmt;

use futures::Stream;
use k8s_openapi::api::core::v1::{Node, Taint};
use kube::Api;

use crate::connection::{ClusterConnection, ClusterError};
use crate::resource_watch::{WatchUpdate, summary_watch};

const ROLE_LABEL_PREFIX: &str = "node-role.kubernetes.io/";
const LEGACY_ROLE_LABEL: &str = "kubernetes.io/role";

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

    /// Watches all nodes. Yields batched snapshots ordered by name.
    pub fn watch_nodes(&self) -> impl Stream<Item = WatchUpdate<NodeSummary>> + Send + 'static {
        let api = Api::<Node>::all(self.client().clone());
        summary_watch(self, api, "watching nodes", node_summary)
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
    }
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
    }
}

#[cfg(test)]
#[path = "node_tests.rs"]
mod node_tests;
