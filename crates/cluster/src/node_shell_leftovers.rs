//! The leftover node shell pods of other runs (spec 0037 decision 18): privileged pods k8sBoard
//! created that no live session owns, because an app crashed, was killed, or quit before its delete
//! went through. A read-only list; deleting one is the app's click, through the write path.
//!
//! Only pods that carry every k8sBoard label and the `k8sboard-node-shell-` name are returned, so a
//! sweep can never offer anything else for deletion, whatever the server's selector matched.

use k8s_openapi::api::core::v1::Pod;
use kube::api::ListParams;

use crate::connection::{ClusterConnection, ClusterError};
use crate::debug_pod_bodies::{
    INSTANCE_LABEL, MANAGED_BY, MANAGED_BY_LABEL, NODE_SHELL_PREFIX, NODE_SHELL_PURPOSE,
    PURPOSE_LABEL, is_label_value,
};
use crate::namespace::NamespaceScope;

const ACTION: &str = "listing leftover node shell pods";
/// One page is plenty: a user with more leftovers than this deletes these and sees the rest next
/// session.
// ponytail: first page only; follow the continue token if anyone leaves hundreds behind.
const PAGE_SIZE: u32 = 200;

/// Where a leftover pod stands. A finished pod is safe to delete; a running one may belong to
/// another k8sBoard window or user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftoverPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

impl LeftoverPhase {
    fn of(phase: Option<&str>) -> Self {
        match phase {
            Some("Pending") => Self::Pending,
            Some("Running") => Self::Running,
            Some("Succeeded") => Self::Succeeded,
            Some("Failed") => Self::Failed,
            _ => Self::Unknown,
        }
    }

    /// Succeeded or Failed: nothing runs in it.
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

/// One node shell pod of another run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeShellLeftover {
    pub namespace: String,
    pub name: String,
    /// The delete's precondition: another pod that took the name later is never deleted.
    pub uid: String,
    pub node: Option<String>,
    pub phase: LeftoverPhase,
    pub created_at: Option<jiff::Timestamp>,
}

/// The selector that finds the pods of every run but `instance`.
fn selector(instance: &str) -> String {
    format!(
        "{MANAGED_BY_LABEL}={MANAGED_BY},{PURPOSE_LABEL}={NODE_SHELL_PURPOSE},{INSTANCE_LABEL}!={instance}"
    )
}

/// The leftover shape of `pod`, or `None` when it is not a k8sBoard node shell pod of another run.
fn leftover_of(pod: &Pod, instance: &str) -> Option<NodeShellLeftover> {
    let meta = &pod.metadata;
    let name = meta.name.as_deref()?;
    let labels = meta.labels.as_ref()?;
    let has_label = |key: &str, value: &str| labels.get(key).is_some_and(|found| found == value);
    let is_ours = has_label(MANAGED_BY_LABEL, MANAGED_BY)
        && has_label(PURPOSE_LABEL, NODE_SHELL_PURPOSE)
        && name.starts_with(NODE_SHELL_PREFIX);
    let is_this_run = labels
        .get(INSTANCE_LABEL)
        .is_some_and(|found| found == instance);
    if !is_ours || is_this_run {
        return None;
    }
    Some(NodeShellLeftover {
        namespace: meta.namespace.clone()?,
        name: name.to_owned(),
        uid: meta.uid.clone().filter(|uid| !uid.is_empty())?,
        node: pod.spec.as_ref().and_then(|spec| spec.node_name.clone()),
        phase: LeftoverPhase::of(
            pod.status
                .as_ref()
                .and_then(|status| status.phase.as_deref()),
        ),
        created_at: meta.creation_timestamp.as_ref().map(|time| time.0),
    })
}

impl ClusterConnection {
    /// action: "listing leftover node shell pods". Read-only: one list per namespace of `scope`,
    /// any phase, every run but `instance`. Sorted by namespace, then name. A
    /// forbidden namespace is skipped; the error returns only when no namespace could be listed.
    pub async fn node_shell_leftovers(
        &self,
        scope: &NamespaceScope,
        instance: &str,
    ) -> Result<Vec<NodeShellLeftover>, ClusterError> {
        if !is_label_value(instance) {
            return Err(ClusterError::Rendered {
                message: "the run id is not a label value".to_owned(),
            });
        }
        let params = ListParams::default()
            .labels(&selector(instance))
            .limit(PAGE_SIZE);
        let mut found = Vec::new();
        let mut denied = None;
        let mut listed_any = false;
        for (_, api) in self.scoped_apis::<Pod>(scope) {
            // A namespace the user may not list (for example the default `kube-system`) must not
            // hide the leftovers of the namespaces they may.
            let list = match self.run(ACTION, api.list(&params)).await {
                Ok(list) => list,
                Err(error @ ClusterError::Forbidden { .. }) => {
                    denied.get_or_insert(error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            listed_any = true;
            found.extend(
                list.items
                    .iter()
                    .filter_map(|pod| leftover_of(pod, instance)),
            );
        }
        if let (false, Some(error)) = (listed_any, denied) {
            return Err(error);
        }
        found.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
        Ok(found)
    }
}

#[cfg(test)]
#[path = "node_shell_leftovers_tests.rs"]
mod node_shell_leftovers_tests;
