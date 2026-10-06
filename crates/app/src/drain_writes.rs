//! The writes of a drain (spec 0034): the cordon of a node and the eviction of one pod, or its direct
//! delete when the budgets are skipped (spec 0040), as the `WriteIntent` that `checked_write` sends,
//! dry-runs first. Pure: nothing here sends anything.

use cluster::{
    DeletePropagation, NodeScheduling, ObjectKind, ObjectRef, WriteOperation, WriteRequest,
};

use crate::app_shell::write_flow::{WriteIntent, cordon_intent};
use crate::cluster_registry::ClusterRef;
use crate::drain_plan::{BudgetPolicy, DrainOptions, PodKey};
use crate::resource_actions::{ResourceAction, action_risk};

/// The cluster the drain runs on: every write names it, so the guard, the connection, the tier,
/// and the audit line all come from its own slot, never from the primary.
#[derive(Clone, Copy)]
pub(crate) struct DrainScope<'a> {
    pub(crate) cluster: &'a ClusterRef,
    pub(crate) cluster_name: &'a str,
}

/// The cordon of `node`. `None` when the node name is not a valid object name.
pub(crate) fn cordon_write(scope: DrainScope<'_>, node: &str) -> Option<WriteIntent> {
    cordon_intent(
        scope.cluster,
        scope.cluster_name,
        node,
        &NodeScheduling::Enabled,
    )
}

/// The eviction or the direct delete of `pod` under `options`, pinned to its uid so a pod recreated
/// under the same name is never removed. `None` when the pod has no uid or a name that is not a
/// valid object name. The delete takes no grace: it uses the pod's own (0033 decision 6).
pub(crate) fn removal_write(
    scope: DrainScope<'_>,
    pod: &PodKey,
    options: &DrainOptions,
) -> Option<WriteIntent> {
    let target = ObjectRef::new(
        ObjectKind::Pod,
        Some(pod.namespace.clone()),
        pod.name.clone(),
    )?;
    let uid = pod.uid.clone();
    let (operation, verb) = match options.budgets {
        BudgetPolicy::Respect => (
            WriteOperation::EvictPod {
                uid,
                grace: options.grace,
            },
            "Evict",
        ),
        BudgetPolicy::Skip => (
            WriteOperation::DeleteObject {
                uid,
                propagation: DeletePropagation::Background,
            },
            "Delete",
        ),
    };
    let request = WriteRequest::new(target, operation)?;
    Some(WriteIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action: ResourceAction::Drain,
        label: format!("{verb} pod {}", pod.text()).into(),
        button: verb.into(),
        request,
        risk: action_risk(ResourceAction::Drain),
        warnings: Vec::new(),
    })
}

#[cfg(test)]
#[path = "drain_writes_tests.rs"]
mod drain_writes_tests;
