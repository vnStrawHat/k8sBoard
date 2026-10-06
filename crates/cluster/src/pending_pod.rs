//! Pods waiting for a node (spec 0034, drain tab): after a drain, the replacements of the evicted
//! pods that the scheduler could not place.

use k8s_openapi::api::core::v1::Pod;
use kube::Api;

use crate::connection::{ClusterConnection, ClusterError};
use crate::event::optional_message;
use crate::workload::{ControllerRef, controller_ref};

const POD_SCHEDULED: &str = "PodScheduled";

/// A pod in phase `Pending`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingPod {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub controller: Option<ControllerRef>,
    /// Why the scheduler did not place it: the message of its `PodScheduled=False` condition, the
    /// text of the `FailedScheduling` event. `None` while the scheduler has not looked at it.
    pub reason: Option<String>,
}

impl ClusterConnection {
    /// Lists the pods in phase `Pending` in every namespace (a server-side field selector, paged),
    /// ordered by (namespace, name). Read-only.
    pub async fn pending_pods(&self) -> Result<Vec<PendingPod>, ClusterError> {
        let api = Api::<Pod>::all(self.client().clone());
        let pods = self
            .list_all_where(api, "listing pending pods", Some("status.phase=Pending"))
            .await?;
        let mut pending: Vec<PendingPod> = pods.iter().map(pending_pod).collect();
        pending.sort_by(|left, right| {
            (&left.namespace, &left.name).cmp(&(&right.namespace, &right.name))
        });
        Ok(pending)
    }
}

fn pending_pod(pod: &Pod) -> PendingPod {
    let unscheduled = pod
        .status
        .iter()
        .flat_map(|status| status.conditions.iter().flatten())
        .find(|condition| condition.type_ == POD_SCHEDULED && condition.status == "False");
    PendingPod {
        namespace: pod.metadata.namespace.clone().unwrap_or_default(),
        name: pod.metadata.name.clone().unwrap_or_default(),
        uid: pod.metadata.uid.clone().unwrap_or_default(),
        controller: controller_ref(&pod.metadata),
        reason: unscheduled.and_then(|condition| optional_message(condition.message.as_deref())),
    }
}

#[cfg(test)]
#[path = "pending_pod_tests.rs"]
mod pending_pod_tests;
