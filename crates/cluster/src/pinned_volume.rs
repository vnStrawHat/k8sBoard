//! Volumes a node cannot hand over (spec 0034, drain dialog): a local-path, local, or hostPath
//! volume carries a node affinity on one host name, so a pod that mounts it can only run on that
//! node, and the replacement of an evicted pod stays Pending once the node is cordoned.

use std::collections::HashSet;

use k8s_openapi::api::core::v1::{NodeSelectorRequirement, NodeSelectorTerm, PersistentVolume};
use kube::Api;

use crate::connection::{ClusterConnection, ClusterError};
use crate::pod::DrainPod;

const HOSTNAME_LABEL: &str = "kubernetes.io/hostname";
const NODE_NAME_FIELD: &str = "metadata.name";

impl ClusterConnection {
    /// Sets `pinned_volume` on the pods of `node` that mount a claim bound to a volume pinned to
    /// the node's host name. Read-only: a node read and a volume list, sent only when some pod
    /// mounts a claim. A zone or region affinity does not pin a pod to one node.
    pub async fn pin_volumes(&self, node: &str, pods: &mut [DrainPod]) -> Result<(), ClusterError> {
        if pods.iter().all(|pod| pod.claims.is_empty()) {
            return Ok(());
        }
        let edit = self.node_for_edit(node).await?;
        let mut hosts = vec![node.to_owned()];
        hosts.extend(edit.labels.get(HOSTNAME_LABEL).cloned());
        let api = Api::<PersistentVolume>::all(self.client().clone());
        let volumes = self.list_all(api, "listing persistent volumes").await?;
        let pinned: HashSet<(&str, &str)> = volumes
            .iter()
            .filter(|volume| is_pinned_to(volume, &hosts))
            .filter_map(|volume| {
                let claim = volume.spec.as_ref()?.claim_ref.as_ref()?;
                Some((claim.namespace.as_deref()?, claim.name.as_deref()?))
            })
            .collect();
        for pod in pods {
            pod.pinned_volume = pod
                .claims
                .iter()
                .find(|claim| pinned.contains(&(pod.namespace.as_str(), claim.as_str())))
                .cloned();
        }
        Ok(())
    }
}

/// Whether a term of the volume's required node affinity names one of `hosts`. Terms are
/// alternatives, so one is enough.
fn is_pinned_to(volume: &PersistentVolume, hosts: &[String]) -> bool {
    volume
        .spec
        .iter()
        .filter_map(|spec| spec.node_affinity.as_ref()?.required.as_ref())
        .flat_map(|selector| &selector.node_selector_terms)
        .any(|term| term_names_host(term, hosts))
}

fn term_names_host(term: &NodeSelectorTerm, hosts: &[String]) -> bool {
    let names_host = |requirement: &NodeSelectorRequirement, key: &str| {
        requirement.key == key
            && requirement.operator == "In"
            && requirement
                .values
                .iter()
                .flatten()
                .any(|value| hosts.contains(value))
    };
    let by_label = term
        .match_expressions
        .iter()
        .flatten()
        .any(|requirement| names_host(requirement, HOSTNAME_LABEL));
    let by_field = term
        .match_fields
        .iter()
        .flatten()
        .any(|requirement| names_host(requirement, NODE_NAME_FIELD));
    by_label || by_field
}

#[cfg(test)]
#[path = "pinned_volume_tests.rs"]
mod pinned_volume_tests;
