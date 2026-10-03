//! Finding the pod behind a forward target (spec 0035): a pod as is, a Service through its
//! selector and `targetPort`, a Deployment or StatefulSet through its selector. The pick is a
//! ready pod, the first by name, so it is deterministic.

use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::core::v1::{Pod, Service, ServicePort};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::Api;

use crate::connection::{ClusterConnection, ClusterError};
use crate::port_forward::{ForwardError, ForwardRequest, ForwardTarget};
use crate::selector::Selector;
use crate::workload::{key_value_terms, label_terms};

pub(crate) const RESOLVE_ACTION: &str = "finding the forward target";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResolvedPod {
    pub(crate) pod: String,
    pub(crate) pod_port: u16,
}

/// Finds the pod and the pod port behind the target.
pub(crate) async fn resolve_target(
    connection: &ClusterConnection,
    request: &ForwardRequest,
) -> Result<ResolvedPod, ForwardError> {
    let client = connection.client();
    let namespace = request.namespace.as_str();
    match &request.target {
        ForwardTarget::Pod { name } => {
            let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
            connection
                .run(RESOLVE_ACTION, pods.get(name))
                .await
                .map_err(resolve_error)?;
            Ok(ResolvedPod {
                pod: name.clone(),
                pod_port: request.remote_port,
            })
        }
        ForwardTarget::Service { name } => {
            let services: Api<Service> = Api::namespaced(client.clone(), namespace);
            let service = connection
                .run(RESOLVE_ACTION, services.get(name))
                .await
                .map_err(resolve_error)?;
            let spec = service.spec.unwrap_or_default();
            let is_external_name = spec.type_.as_deref() == Some("ExternalName");
            let terms = key_value_terms(spec.selector.as_ref());
            let Some(selector) = Selector::of_labels(&terms).filter(|_| !is_external_name) else {
                return Err(ForwardError::UnsupportedService);
            };
            let pods = ready_pods_matching(connection, namespace, &selector).await?;
            let pod = ready_pod(&pods).ok_or(ForwardError::NoReadyPod)?;
            let service_port = spec
                .ports
                .iter()
                .flatten()
                .find(|port| port.port == i32::from(request.remote_port))
                .ok_or_else(|| ForwardError::PortNotDeclared(request.remote_port.to_string()))?;
            Ok(ResolvedPod {
                pod: pod.metadata.name.clone().unwrap_or_default(),
                pod_port: service_target_port(service_port, pod)?,
            })
        }
        ForwardTarget::Deployment { name } => {
            let deployments: Api<Deployment> = Api::namespaced(client.clone(), namespace);
            let deployment = connection
                .run(RESOLVE_ACTION, deployments.get(name))
                .await
                .map_err(resolve_error)?;
            let selector = deployment.spec.map(|spec| spec.selector);
            workload_pod(connection, request, selector).await
        }
        ForwardTarget::StatefulSet { name } => {
            let sets: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
            let set = connection
                .run(RESOLVE_ACTION, sets.get(name))
                .await
                .map_err(resolve_error)?;
            let selector = set.spec.map(|spec| spec.selector);
            workload_pod(connection, request, selector).await
        }
    }
}

/// A ready pod of a workload; the port is a container port of the pod template.
async fn workload_pod(
    connection: &ClusterConnection,
    request: &ForwardRequest,
    selector: Option<LabelSelector>,
) -> Result<ResolvedPod, ForwardError> {
    let selector = Selector::of(&selector.unwrap_or_default());
    // An empty selector would match every pod of the namespace.
    if selector.selects_everything() {
        return Err(ForwardError::NoReadyPod);
    }
    let pods = ready_pods_matching(connection, &request.namespace, &selector).await?;
    let pod = ready_pod(&pods).ok_or(ForwardError::NoReadyPod)?;
    Ok(ResolvedPod {
        pod: pod.metadata.name.clone().unwrap_or_default(),
        pod_port: request.remote_port,
    })
}

async fn ready_pods_matching(
    connection: &ClusterConnection,
    namespace: &str,
    selector: &Selector,
) -> Result<Vec<Pod>, ForwardError> {
    let api: Api<Pod> = Api::namespaced(connection.client().clone(), namespace);
    let pods = connection
        .list_all(api, RESOLVE_ACTION)
        .await
        .map_err(resolve_error)?;
    Ok(pods
        .into_iter()
        .filter(|pod| selector.matches(&label_terms(&pod.metadata)))
        .collect())
}

fn resolve_error(error: ClusterError) -> ForwardError {
    match error {
        ClusterError::Api { code: 404, .. } => ForwardError::TargetNotFound,
        other => ForwardError::Cluster(other),
    }
}

/// Running, Ready, not deleting; the first by name, so the pick is deterministic.
fn ready_pod<'a>(pods: impl IntoIterator<Item = &'a Pod>) -> Option<&'a Pod> {
    pods.into_iter()
        .filter(|pod| is_ready(pod))
        .min_by(|left, right| left.metadata.name.cmp(&right.metadata.name))
}

fn is_ready(pod: &Pod) -> bool {
    let Some(status) = &pod.status else {
        return false;
    };
    let has_ready_condition = status
        .conditions
        .iter()
        .flatten()
        .any(|condition| condition.type_ == "Ready" && condition.status == "True");
    pod.metadata.deletion_timestamp.is_none()
        && status.phase.as_deref() == Some("Running")
        && has_ready_condition
}

/// The pod port behind a Service port: a number as is, a name looked up in the chosen pod's
/// containers, and the Service port itself when `targetPort` is absent.
fn service_target_port(port: &ServicePort, pod: &Pod) -> Result<u16, ForwardError> {
    match &port.target_port {
        None => port_number(port.port),
        Some(IntOrString::Int(number)) => port_number(*number),
        Some(IntOrString::String(name)) => pod
            .spec
            .iter()
            .flat_map(|spec| &spec.containers)
            .flat_map(|container| container.ports.iter().flatten())
            .find(|container_port| container_port.name.as_deref() == Some(name.as_str()))
            .map_or_else(
                || Err(ForwardError::PortNotDeclared(name.clone())),
                |container_port| port_number(container_port.container_port),
            ),
    }
}

fn port_number(number: i32) -> Result<u16, ForwardError> {
    u16::try_from(number).map_err(|_| ForwardError::PortNotDeclared(number.to_string()))
}

#[cfg(test)]
#[path = "port_forward_target_tests.rs"]
mod port_forward_target_tests;
