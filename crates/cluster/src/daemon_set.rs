use futures::Stream;
use k8s_openapi::api::apps::v1::DaemonSet;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::pod_status::non_negative;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{
    TemplateContainer, key_value_terms, label_terms, optional_count, selector_terms,
    template_containers,
};

/// Has no conditions: the DaemonSet controller never writes any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonSetSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `status.desiredNumberScheduled`.
    pub desired: u32,
    pub current: u32,
    pub ready: u32,
    pub up_to_date: u32,
    pub available: u32,
    pub misscheduled: u32,
    /// `spec.template.spec.nodeSelector` as `key=value` terms.
    pub node_selector: Vec<String>,
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
            self.scoped_api(scope),
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
        desired: status.map_or(0, |status| non_negative(status.desired_number_scheduled)),
        current: status.map_or(0, |status| non_negative(status.current_number_scheduled)),
        ready: status.map_or(0, |status| non_negative(status.number_ready)),
        up_to_date: optional_count(status.and_then(|status| status.updated_number_scheduled)),
        available: optional_count(status.and_then(|status| status.number_available)),
        misscheduled: status.map_or(0, |status| non_negative(status.number_misscheduled)),
        node_selector: key_value_terms(pod_spec.and_then(|spec| spec.node_selector.as_ref())),
        update_strategy: spec
            .and_then(|spec| spec.update_strategy.as_ref())
            .and_then(|strategy| strategy.type_.clone())
            .unwrap_or_default(),
        selector: spec.map_or_else(Vec::new, |spec| selector_terms(&spec.selector)),
        containers: spec.map_or_else(Vec::new, |spec| template_containers(&spec.template)),
    }
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
}
