use std::fmt;

use futures::Stream;
use k8s_openapi::api::core::v1::{LoadBalancerIngress, Service, ServicePort};

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{int_or_string_text, key_value_terms, label_terms, non_empty};

const DEFAULT_SERVICE_TYPE: &str = "ClusterIP";
const HEADLESS_CLUSTER_IP: &str = "None";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `spec.type`, defaulting to `ClusterIP`.
    pub service_type: String,
    /// Empty when the service is headless or has no cluster IP.
    pub cluster_ips: Vec<String>,
    pub is_headless: bool,
    /// Load balancer addresses, then `spec.externalIPs`, then the `ExternalName` target.
    pub external_addresses: Vec<String>,
    pub ports: Vec<ServicePortSummary>,
    /// `key=value` terms in key order.
    pub selector: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServicePortSummary {
    pub name: Option<String>,
    pub port: u16,
    /// The `IntOrString` text.
    pub target_port: Option<String>,
    pub node_port: Option<u16>,
    /// Defaults to `TCP`.
    pub protocol: String,
}

/// kubectl's `PORT(S)` text: `80/TCP`, or `80:30080/TCP` with a node port.
impl fmt::Display for ServicePortSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.port)?;
        if let Some(node_port) = self.node_port {
            write!(formatter, ":{node_port}")?;
        }
        write!(formatter, "/{}", self.protocol)
    }
}

impl ClusterConnection {
    /// Watches services in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_services(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<ServiceSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_api(scope),
            "watching services",
            service_summary,
        )
    }
}

pub(crate) fn service_summary(service: &Service) -> ServiceSummary {
    let spec = service.spec.as_ref();
    let service_type = non_empty(spec.and_then(|spec| spec.type_.as_deref()))
        .unwrap_or_else(|| DEFAULT_SERVICE_TYPE.to_owned());
    let cluster_ip = spec.and_then(|spec| spec.cluster_ip.as_deref());
    let is_headless = cluster_ip == Some(HEADLESS_CLUSTER_IP);
    ServiceSummary {
        namespace: service.metadata.namespace.clone().unwrap_or_default(),
        name: service.metadata.name.clone().unwrap_or_default(),
        created_at: service
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&service.metadata),
        cluster_ips: if is_headless {
            Vec::new()
        } else {
            cluster_ips(
                spec.and_then(|spec| spec.cluster_ips.as_deref()),
                cluster_ip,
            )
        },
        is_headless,
        external_addresses: external_addresses(service, &service_type),
        ports: spec
            .into_iter()
            .flat_map(|spec| spec.ports.iter().flatten())
            .filter_map(port_summary)
            .collect(),
        selector: key_value_terms(spec.and_then(|spec| spec.selector.as_ref())),
        service_type,
    }
}

/// `spec.clusterIPs`, else the single `spec.clusterIP`; empty values are dropped.
fn cluster_ips(cluster_ips: Option<&[String]>, cluster_ip: Option<&str>) -> Vec<String> {
    let listed: Vec<String> = cluster_ips
        .into_iter()
        .flatten()
        .filter(|address| !address.is_empty())
        .cloned()
        .collect();
    if !listed.is_empty() {
        return listed;
    }
    non_empty(cluster_ip).into_iter().collect()
}

fn external_addresses(service: &Service, service_type: &str) -> Vec<String> {
    let spec = service.spec.as_ref();
    let balancer = service
        .status
        .iter()
        .filter_map(|status| status.load_balancer.as_ref())
        .flat_map(|balancer| balancer.ingress.iter().flatten())
        .filter_map(balancer_address);
    let external_ips = spec
        .into_iter()
        .flat_map(|spec| spec.external_ips.iter().flatten())
        .cloned();
    let external_name = spec
        .filter(|_| service_type == "ExternalName")
        .and_then(|spec| non_empty(spec.external_name.as_deref()));
    balancer.chain(external_ips).chain(external_name).collect()
}

/// The IP, else the hostname.
fn balancer_address(ingress: &LoadBalancerIngress) -> Option<String> {
    non_empty(ingress.ip.as_deref()).or_else(|| non_empty(ingress.hostname.as_deref()))
}

/// A port outside `u16` is dropped; the API rejects such ports anyway.
fn port_summary(port: &ServicePort) -> Option<ServicePortSummary> {
    Some(ServicePortSummary {
        name: non_empty(port.name.as_deref()),
        port: u16::try_from(port.port).ok()?,
        target_port: port.target_port.as_ref().map(int_or_string_text),
        node_port: port
            .node_port
            .and_then(|node_port| u16::try_from(node_port).ok()),
        protocol: non_empty(port.protocol.as_deref()).unwrap_or_else(|| "TCP".to_owned()),
    })
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod service_tests;
