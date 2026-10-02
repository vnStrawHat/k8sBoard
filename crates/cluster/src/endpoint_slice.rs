use futures::Stream;
use k8s_openapi::api::discovery::v1::{Endpoint, EndpointPort as ApiEndpointPort, EndpointSlice};

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, summary_watch};

const SERVICE_NAME_LABEL: &str = "kubernetes.io/service-name";

/// One EndpointSlice (discovery.k8s.io/v1), reduced to what a Service's endpoint counts
/// need. No other labels, annotations, hints, hostnames, or zones are kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointSliceSummary {
    pub namespace: String,
    pub name: String,
    /// The `kubernetes.io/service-name` label; a slice without it belongs to no service.
    pub service: Option<String>,
    /// `IPv4`, `IPv6`, or `FQDN`.
    pub address_type: String,
    pub ports: Vec<EndpointPort>,
    pub endpoints: Vec<EndpointSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointPort {
    pub name: Option<String>,
    pub port: Option<u16>,
    /// Defaults to `TCP`.
    pub protocol: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointSummary {
    /// The first address; an endpoint without one is dropped.
    pub address: String,
    /// `conditions.ready`; an unset value reads true, as the API documents.
    pub is_ready: bool,
    /// `conditions.terminating`; an unset value reads false.
    pub is_terminating: bool,
    /// `targetRef.name` when `targetRef.kind` is `Pod`.
    pub pod: Option<String>,
    pub node: Option<String>,
}

impl ClusterConnection {
    /// Watches endpoint slices in `scope`. Yields batched snapshots ordered by
    /// (namespace, name).
    pub fn watch_endpoint_slices(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<EndpointSliceSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching endpoint slices",
            endpoint_slice_summary,
        )
    }
}

fn endpoint_slice_summary(slice: &EndpointSlice) -> EndpointSliceSummary {
    EndpointSliceSummary {
        namespace: slice.metadata.namespace.clone().unwrap_or_default(),
        name: slice.metadata.name.clone().unwrap_or_default(),
        service: slice
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get(SERVICE_NAME_LABEL))
            .filter(|name| !name.is_empty())
            .cloned(),
        address_type: slice.address_type.clone(),
        ports: slice.ports.iter().flatten().map(endpoint_port).collect(),
        endpoints: slice.endpoints.iter().filter_map(endpoint).collect(),
    }
}

fn endpoint_port(port: &ApiEndpointPort) -> EndpointPort {
    EndpointPort {
        name: port.name.clone().filter(|name| !name.is_empty()),
        port: port.port.and_then(|port| u16::try_from(port).ok()),
        protocol: port.protocol.clone().unwrap_or_else(|| "TCP".to_owned()),
    }
}

fn endpoint(endpoint: &Endpoint) -> Option<EndpointSummary> {
    let address = endpoint.addresses.first()?.clone();
    let conditions = endpoint.conditions.as_ref();
    Some(EndpointSummary {
        address,
        is_ready: conditions
            .and_then(|conditions| conditions.ready)
            .unwrap_or(true),
        is_terminating: conditions
            .and_then(|conditions| conditions.terminating)
            .unwrap_or(false),
        pod: endpoint
            .target_ref
            .as_ref()
            .filter(|target| target.kind.as_deref() == Some("Pod"))
            .and_then(|target| target.name.clone()),
        node: endpoint.node_name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::core::v1::ObjectReference;
    use k8s_openapi::api::discovery::v1::EndpointConditions;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    use super::*;

    fn api_endpoint(address: &str, ready: Option<bool>, terminating: Option<bool>) -> Endpoint {
        Endpoint {
            addresses: vec![address.to_owned()],
            conditions: Some(EndpointConditions {
                ready,
                terminating,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn slice_of(service: Option<&str>, endpoints: Vec<Endpoint>) -> EndpointSlice {
        EndpointSlice {
            metadata: ObjectMeta {
                namespace: Some("shop".to_owned()),
                name: Some("api-abc12".to_owned()),
                labels: service.map(|service| {
                    BTreeMap::from([
                        (SERVICE_NAME_LABEL.to_owned(), service.to_owned()),
                        ("other".to_owned(), "dropped".to_owned()),
                    ])
                }),
                ..Default::default()
            },
            address_type: "IPv4".to_owned(),
            endpoints,
            ports: None,
        }
    }

    #[test]
    fn endpoint_slice_summary_maps_service_ports_and_endpoints() {
        let mut slice = slice_of(
            Some("api"),
            vec![Endpoint {
                node_name: Some("node-1".to_owned()),
                target_ref: Some(ObjectReference {
                    kind: Some("Pod".to_owned()),
                    name: Some("api-0".to_owned()),
                    ..Default::default()
                }),
                ..api_endpoint("10.0.0.7", Some(false), Some(true))
            }],
        );
        slice.ports = Some(vec![
            ApiEndpointPort {
                name: Some("http".to_owned()),
                port: Some(8080),
                protocol: None,
                ..Default::default()
            },
            ApiEndpointPort {
                port: Some(70_000),
                protocol: Some("UDP".to_owned()),
                ..Default::default()
            },
        ]);
        let summary = endpoint_slice_summary(&slice);
        assert_eq!(
            (summary.namespace.as_str(), summary.name.as_str()),
            ("shop", "api-abc12")
        );
        assert_eq!(summary.service.as_deref(), Some("api"));
        assert_eq!(summary.address_type, "IPv4");
        assert_eq!(
            summary.ports,
            [
                EndpointPort {
                    name: Some("http".to_owned()),
                    port: Some(8080),
                    protocol: "TCP".to_owned(),
                },
                EndpointPort {
                    name: None,
                    port: None,
                    protocol: "UDP".to_owned(),
                },
            ]
        );
        assert_eq!(
            summary.endpoints,
            [EndpointSummary {
                address: "10.0.0.7".to_owned(),
                is_ready: false,
                is_terminating: true,
                pod: Some("api-0".to_owned()),
                node: Some("node-1".to_owned()),
            }]
        );
    }

    #[test]
    fn endpoint_nil_conditions_default() {
        let no_conditions = Endpoint {
            addresses: vec!["10.0.0.8".to_owned()],
            ..Default::default()
        };
        let slice = slice_of(
            Some("api"),
            vec![no_conditions, api_endpoint("10.0.0.9", None, None)],
        );
        for endpoint in endpoint_slice_summary(&slice).endpoints {
            assert!(endpoint.is_ready, "{}", endpoint.address);
            assert!(!endpoint.is_terminating, "{}", endpoint.address);
        }
    }

    #[test]
    fn endpoint_without_address_is_dropped() {
        let addressless = Endpoint {
            addresses: Vec::new(),
            ..Default::default()
        };
        let slice = slice_of(
            Some("api"),
            vec![addressless, api_endpoint("10.0.0.1", Some(true), None)],
        );
        let addresses: Vec<_> = endpoint_slice_summary(&slice)
            .endpoints
            .into_iter()
            .map(|endpoint| endpoint.address)
            .collect();
        assert_eq!(addresses, ["10.0.0.1"]);
    }

    #[test]
    fn non_pod_target_has_no_pod() {
        let node_target = Endpoint {
            target_ref: Some(ObjectReference {
                kind: Some("Node".to_owned()),
                name: Some("node-1".to_owned()),
                ..Default::default()
            }),
            ..api_endpoint("10.0.0.2", Some(true), None)
        };
        let untargeted = api_endpoint("10.0.0.3", Some(true), None);
        let slice = slice_of(Some("api"), vec![node_target, untargeted]);
        let pods: Vec<_> = endpoint_slice_summary(&slice)
            .endpoints
            .into_iter()
            .map(|endpoint| endpoint.pod)
            .collect();
        assert_eq!(pods, [None, None]);
    }

    #[test]
    fn slice_without_service_label_has_no_service() {
        let summary = endpoint_slice_summary(&slice_of(None, Vec::new()));
        assert_eq!(summary.service, None);
        let empty_label = slice_of(Some(""), Vec::new());
        assert_eq!(endpoint_slice_summary(&empty_label).service, None);
    }
}
