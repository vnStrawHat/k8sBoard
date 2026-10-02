//! What the Nodes summary chips count: groups of nodes by health, scheduling, and version.
//! Pure, so it is tested without a window.

use std::collections::BTreeMap;

use cluster::{NodeReadiness, NodeScheduling, NodeSummary};

use crate::table_sort::natural_cmp;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NodeGroup {
    /// Ready and schedulable.
    Ready,
    /// Not ready, or its readiness is unknown.
    NotReady,
    /// Scheduling is disabled, whatever the readiness: a cordoned NotReady node is in both.
    Cordoned,
    Version(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NodeCounts {
    pub(crate) total: usize,
    pub(crate) ready: usize,
    pub(crate) not_ready: usize,
    pub(crate) cordoned: usize,
    /// Count descending, then the higher version first.
    pub(crate) versions: Vec<(String, usize)>,
    /// The version most nodes run; the others are skewed.
    pub(crate) common_version: Option<String>,
}

pub(crate) fn node_counts(nodes: &[NodeSummary]) -> NodeCounts {
    let count = |group: &NodeGroup| {
        nodes
            .iter()
            .filter(|node| node_in_group(node, group))
            .count()
    };
    let mut by_version: BTreeMap<&str, usize> = BTreeMap::new();
    for node in nodes {
        *by_version.entry(&node.kubelet_version).or_default() += 1;
    }
    let mut versions: Vec<(String, usize)> = by_version
        .into_iter()
        .map(|(version, count)| (version.to_owned(), count))
        .collect();
    versions.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| natural_cmp(&b.0, &a.0)));
    NodeCounts {
        total: nodes.len(),
        ready: count(&NodeGroup::Ready),
        not_ready: count(&NodeGroup::NotReady),
        cordoned: count(&NodeGroup::Cordoned),
        common_version: versions.first().map(|(version, _)| version.clone()),
        versions,
    }
}

pub(crate) fn node_in_group(node: &NodeSummary, group: &NodeGroup) -> bool {
    let status = node.status;
    match group {
        NodeGroup::Ready => {
            status.readiness == NodeReadiness::Ready && status.scheduling == NodeScheduling::Enabled
        }
        NodeGroup::NotReady => status.readiness != NodeReadiness::Ready,
        NodeGroup::Cordoned => status.scheduling == NodeScheduling::Disabled,
        NodeGroup::Version(version) => node.kubelet_version == *version,
    }
}

/// How many nodes carry each role, most first, then by name. A node without roles adds
/// nothing: no `worker` is inferred.
pub(crate) fn role_counts(nodes: &[NodeSummary]) -> Vec<(String, usize)> {
    let mut by_role: BTreeMap<&str, usize> = BTreeMap::new();
    for role in nodes.iter().flat_map(|node| &node.roles) {
        *by_role.entry(role).or_default() += 1;
    }
    let mut roles: Vec<(String, usize)> = by_role
        .into_iter()
        .map(|(role, count)| (role.to_owned(), count))
        .collect();
    roles.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    roles
}

#[cfg(test)]
mod tests {
    use cluster::{NodeStatus, NodeSystemInfo};

    use super::*;

    fn node(
        version: &str,
        readiness: NodeReadiness,
        scheduling: NodeScheduling,
        roles: &[&str],
    ) -> NodeSummary {
        NodeSummary {
            name: "n".to_owned(),
            status: NodeStatus {
                readiness,
                scheduling,
            },
            roles: roles.iter().map(|role| (*role).to_owned()).collect(),
            taints: Vec::new(),
            kubelet_version: version.to_owned(),
            internal_ip: None,
            created_at: None,
            conditions: Vec::new(),
            addresses: Vec::new(),
            system: NodeSystemInfo::default(),
            resources: Vec::new(),
            labels: Vec::new(),
        }
    }

    fn ready(version: &str) -> NodeSummary {
        node(version, NodeReadiness::Ready, NodeScheduling::Enabled, &[])
    }

    #[test]
    fn node_counts_group_ready_not_ready_and_cordoned() {
        let nodes = [
            ready("v1.29.5"),
            node(
                "v1.29.5",
                NodeReadiness::Ready,
                NodeScheduling::Disabled,
                &[],
            ),
            node(
                "v1.29.5",
                NodeReadiness::NotReady,
                NodeScheduling::Disabled,
                &[],
            ),
            node(
                "v1.29.5",
                NodeReadiness::Unknown,
                NodeScheduling::Enabled,
                &[],
            ),
        ];
        let counts = node_counts(&nodes);
        assert_eq!(counts.total, 4);
        // A cordoned healthy node is not Ready: it takes no pods.
        assert_eq!(counts.ready, 1);
        assert_eq!(counts.not_ready, 2);
        // The cordoned NotReady node counts in both groups.
        assert_eq!(counts.cordoned, 2);
    }

    #[test]
    fn versions_sort_by_count_and_pick_common() {
        let nodes = [
            ready("v1.28.9"),
            ready("v1.29.5"),
            ready("v1.29.5"),
            ready("v1.30.1"),
        ];
        let counts = node_counts(&nodes);
        assert_eq!(
            counts.versions,
            [
                ("v1.29.5".to_owned(), 2),
                ("v1.30.1".to_owned(), 1),
                ("v1.28.9".to_owned(), 1),
            ]
        );
        assert_eq!(counts.common_version.as_deref(), Some("v1.29.5"));
    }

    #[test]
    fn a_version_tie_goes_to_the_higher_version() {
        let tie = node_counts(&[ready("v1.9.0"), ready("v1.10.0")]);
        assert_eq!(tie.common_version.as_deref(), Some("v1.10.0"));
    }

    #[test]
    fn no_nodes_have_no_common_version() {
        assert_eq!(node_counts(&[]).common_version, None);
    }

    #[test]
    fn role_counts_skip_nodes_without_roles() {
        let nodes = [
            node(
                "v",
                NodeReadiness::Ready,
                NodeScheduling::Enabled,
                &["control-plane", "etcd"],
            ),
            node(
                "v",
                NodeReadiness::Ready,
                NodeScheduling::Enabled,
                &["etcd"],
            ),
            ready("v"),
        ];
        assert_eq!(
            role_counts(&nodes),
            [("etcd".to_owned(), 2), ("control-plane".to_owned(), 1)]
        );
        assert!(role_counts(&[ready("v")]).is_empty());
    }

    #[test]
    fn version_group_matches_one_version() {
        let node = ready("v1.29.5");
        assert!(node_in_group(
            &node,
            &NodeGroup::Version("v1.29.5".to_owned())
        ));
        assert!(!node_in_group(
            &node,
            &NodeGroup::Version("v1.30.0".to_owned())
        ));
    }
}
