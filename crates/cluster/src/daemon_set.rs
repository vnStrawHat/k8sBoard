use futures::Stream;
use k8s_openapi::api::apps::v1::DaemonSet;
use k8s_openapi::api::core::v1::PodSpec;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::pod_status::non_negative;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{
    AnnotationTerms, TemplateContainer, annotation_terms, key_value_terms, label_terms,
    optional_count, selector_terms, template_containers,
};

/// Has no conditions: the DaemonSet controller never writes any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonSetSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `key=value` terms in key order, for the drawer's folded Annotations section: see
    /// `annotation_terms` for what is cut and hidden.
    pub annotations: AnnotationTerms,
    /// `status.desiredNumberScheduled`.
    pub desired: u32,
    pub current: u32,
    pub ready: u32,
    pub up_to_date: u32,
    pub available: u32,
    pub misscheduled: u32,
    /// `spec.template.spec.nodeSelector` as `key=value` terms.
    pub node_selector: Vec<String>,
    /// The label keys that `spec.template.spec.affinity.nodeAffinity` requires
    /// (`requiredDuringSchedulingIgnoredDuringExecution` match expressions), in order, no repeats.
    /// Preferred terms only rank nodes, so they are left out.
    pub node_affinity_keys: Vec<String>,
    /// `spec.updateStrategy.type`, or empty when absent.
    pub update_strategy: String,
    pub selector: Vec<String>,
    pub containers: Vec<TemplateContainer>,
}

impl ClusterConnection {
    /// Watches daemon sets in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_daemon_sets(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<DaemonSetSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching daemon sets",
            daemon_set_summary,
        )
    }
}

pub(crate) fn daemon_set_summary(daemon_set: &DaemonSet) -> DaemonSetSummary {
    let spec = daemon_set.spec.as_ref();
    let status = daemon_set.status.as_ref();
    let pod_spec = spec.and_then(|spec| spec.template.spec.as_ref());
    DaemonSetSummary {
        namespace: daemon_set.metadata.namespace.clone().unwrap_or_default(),
        name: daemon_set.metadata.name.clone().unwrap_or_default(),
        created_at: daemon_set
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&daemon_set.metadata),
        annotations: annotation_terms(&daemon_set.metadata),
        desired: status.map_or(0, |status| non_negative(status.desired_number_scheduled)),
        current: status.map_or(0, |status| non_negative(status.current_number_scheduled)),
        ready: status.map_or(0, |status| non_negative(status.number_ready)),
        up_to_date: optional_count(status.and_then(|status| status.updated_number_scheduled)),
        available: optional_count(status.and_then(|status| status.number_available)),
        misscheduled: status.map_or(0, |status| non_negative(status.number_misscheduled)),
        node_selector: key_value_terms(pod_spec.and_then(|spec| spec.node_selector.as_ref())),
        node_affinity_keys: required_affinity_keys(pod_spec),
        update_strategy: spec
            .and_then(|spec| spec.update_strategy.as_ref())
            .and_then(|strategy| strategy.type_.clone())
            .unwrap_or_default(),
        selector: spec.map_or_else(Vec::new, |spec| selector_terms(&spec.selector)),
        containers: spec.map_or_else(Vec::new, |spec| template_containers(&spec.template)),
    }
}

/// The label keys the pod template must find on a node: every `matchExpressions` key of the required
/// node affinity terms.
fn required_affinity_keys(pod_spec: Option<&PodSpec>) -> Vec<String> {
    let terms = pod_spec
        .and_then(|spec| spec.affinity.as_ref())
        .and_then(|affinity| affinity.node_affinity.as_ref())
        .and_then(|affinity| {
            affinity
                .required_during_scheduling_ignored_during_execution
                .as_ref()
        })
        .map(|selector| selector.node_selector_terms.as_slice())
        .unwrap_or_default();
    let mut keys: Vec<String> = Vec::new();
    for expression in terms
        .iter()
        .flat_map(|term| term.match_expressions.iter().flatten())
    {
        if !keys.contains(&expression.key) {
            keys.push(expression.key.clone());
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::apps::v1::{DaemonSetSpec, DaemonSetStatus, DaemonSetUpdateStrategy};
    use k8s_openapi::api::core::v1::{PodSpec, PodTemplateSpec};

    use super::*;

    #[test]
    fn daemon_set_summary_reads_scheduling_counts() {
        let daemon_set = DaemonSet {
            spec: Some(DaemonSetSpec {
                update_strategy: Some(DaemonSetUpdateStrategy {
                    type_: Some("RollingUpdate".to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: Some(DaemonSetStatus {
                desired_number_scheduled: 5,
                current_number_scheduled: 4,
                number_ready: 3,
                updated_number_scheduled: Some(2),
                number_available: Some(1),
                number_misscheduled: 6,
                ..Default::default()
            }),
            ..Default::default()
        };
        let summary = daemon_set_summary(&daemon_set);
        assert_eq!(
            (
                summary.desired,
                summary.current,
                summary.ready,
                summary.up_to_date,
                summary.available,
                summary.misscheduled
            ),
            (5, 4, 3, 2, 1, 6)
        );
        assert_eq!(summary.update_strategy, "RollingUpdate");
    }

    #[test]
    fn daemon_set_node_selector_terms() {
        let node_selector = BTreeMap::from([
            ("kubernetes.io/os".to_owned(), "linux".to_owned()),
            ("disk".to_owned(), "ssd".to_owned()),
        ]);
        let daemon_set = DaemonSet {
            spec: Some(DaemonSetSpec {
                template: PodTemplateSpec {
                    spec: Some(PodSpec {
                        node_selector: Some(node_selector),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            daemon_set_summary(&daemon_set).node_selector,
            ["disk=ssd", "kubernetes.io/os=linux"]
        );
    }

    #[test]
    fn daemon_set_reads_the_keys_of_its_required_node_affinity_only() {
        use k8s_openapi::api::core::v1::{
            Affinity, NodeAffinity, NodeSelector, NodeSelectorRequirement, NodeSelectorTerm,
            PreferredSchedulingTerm,
        };

        let term = |keys: &[&str]| NodeSelectorTerm {
            match_expressions: Some(
                keys.iter()
                    .map(|key| NodeSelectorRequirement {
                        key: (*key).to_owned(),
                        operator: "Exists".to_owned(),
                        values: None,
                    })
                    .collect(),
            ),
            ..Default::default()
        };
        let daemon_set = DaemonSet {
            spec: Some(DaemonSetSpec {
                template: PodTemplateSpec {
                    spec: Some(PodSpec {
                        affinity: Some(Affinity {
                            node_affinity: Some(NodeAffinity {
                                required_during_scheduling_ignored_during_execution: Some(
                                    NodeSelector {
                                        node_selector_terms: vec![
                                            term(&["disk", "team"]),
                                            term(&["disk"]),
                                        ],
                                    },
                                ),
                                preferred_during_scheduling_ignored_during_execution: Some(vec![
                                    PreferredSchedulingTerm {
                                        weight: 1,
                                        preference: term(&["zone"]),
                                    },
                                ]),
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            daemon_set_summary(&daemon_set).node_affinity_keys,
            ["disk", "team"]
        );
        assert!(
            daemon_set_summary(&DaemonSet::default())
                .node_affinity_keys
                .is_empty()
        );
    }
}
