use futures::Stream;
use k8s_openapi::api::networking::v1::{Ingress, IngressBackend};

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{label_terms, non_empty};

const CLASS_ANNOTATION: &str = "kubernetes.io/ingress.class";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngressSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `spec.ingressClassName`, else the `kubernetes.io/ingress.class` annotation.
    pub class: Option<String>,
    /// Rule hosts in order, without duplicates. A rule without a host adds nothing.
    pub hosts: Vec<String>,
    /// Load balancer IPs, else hostnames.
    pub addresses: Vec<String>,
    /// One entry per HTTP path.
    pub rules: Vec<IngressPath>,
    pub default_backend: Option<String>,
    pub tls: Vec<IngressTls>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngressPath {
    pub host: Option<String>,
    pub path: Option<String>,
    /// `name:port` for a service, or `Kind/name` for a resource backend.
    pub backend: String,
}

/// Names only: the TLS secret is never read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngressTls {
    pub hosts: Vec<String>,
    pub secret_name: Option<String>,
}

impl ClusterConnection {
    /// Watches ingresses in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_ingresses(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<IngressSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_api(scope),
            "watching ingresses",
            ingress_summary,
        )
    }
}

pub(crate) fn ingress_summary(ingress: &Ingress) -> IngressSummary {
    let spec = ingress.spec.as_ref();
    let rules = spec.map_or(&[][..], |spec| spec.rules.as_deref().unwrap_or_default());
    let mut hosts: Vec<String> = Vec::new();
    for host in rules
        .iter()
        .filter_map(|rule| non_empty(rule.host.as_deref()))
    {
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    IngressSummary {
        namespace: ingress.metadata.namespace.clone().unwrap_or_default(),
        name: ingress.metadata.name.clone().unwrap_or_default(),
        created_at: ingress
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&ingress.metadata),
        class: non_empty(spec.and_then(|spec| spec.ingress_class_name.as_deref())).or_else(|| {
            let annotations = ingress.metadata.annotations.as_ref()?;
            non_empty(annotations.get(CLASS_ANNOTATION).map(String::as_str))
        }),
        hosts,
        addresses: ingress
            .status
            .iter()
            .filter_map(|status| status.load_balancer.as_ref())
            .flat_map(|balancer| balancer.ingress.iter().flatten())
            .filter_map(|entry| {
                non_empty(entry.ip.as_deref()).or_else(|| non_empty(entry.hostname.as_deref()))
            })
            .collect(),
        rules: rules
            .iter()
            .flat_map(|rule| {
                let paths = rule.http.iter().flat_map(|http| http.paths.iter());
                paths.map(|path| IngressPath {
                    host: non_empty(rule.host.as_deref()),
                    path: non_empty(path.path.as_deref()),
                    backend: backend_text(&path.backend).unwrap_or_default(),
                })
            })
            .collect(),
        default_backend: spec
            .and_then(|spec| spec.default_backend.as_ref())
            .and_then(backend_text),
        tls: spec
            .into_iter()
            .flat_map(|spec| spec.tls.iter().flatten())
            .map(|tls| IngressTls {
                hosts: tls.hosts.iter().flatten().cloned().collect(),
                secret_name: non_empty(tls.secret_name.as_deref()),
            })
            .collect(),
    }
}

/// A service backend as `name:port` (the port number, else its name; just the name when
/// the port is absent), a resource backend as `Kind/name`.
fn backend_text(backend: &IngressBackend) -> Option<String> {
    if let Some(service) = &backend.service {
        let port = service.port.as_ref().and_then(|port| {
            port.number
                .map(|number| number.to_string())
                .or_else(|| non_empty(port.name.as_deref()))
        });
        return Some(match port {
            Some(port) => format!("{}:{port}", service.name),
            None => service.name.clone(),
        });
    }
    let resource = backend.resource.as_ref()?;
    Some(format!("{}/{}", resource.kind, resource.name))
}

#[cfg(test)]
#[path = "ingress_tests.rs"]
mod ingress_tests;
