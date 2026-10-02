//! Row builders for the network kinds: Services and Ingresses.

use cluster::{IngressSummary, IngressTls, ServicePortSummary, ServiceSummary};

use crate::kind_row::{
    DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent, chips,
};
use crate::status_tone::{StatusLabel, StatusTone};
use crate::table_selection::ResourceKey;

const LOAD_BALANCER: &str = "LoadBalancer";

pub(crate) fn service_row(service: &ServiceSummary) -> KindRow {
    let is_pending = is_address_pending(service);
    let cluster_ip = cluster_ip_cell(service);
    let external = if is_pending {
        KindCell::Toned(StatusLabel {
            text: "<pending>".into(),
            tone: StatusTone::Warn,
        })
    } else {
        KindCell::mono_or_absent(&service.external_addresses.join(","))
    };
    let ports: Vec<String> = service.ports.iter().map(ToString::to_string).collect();
    let mut details = vec![
        DetailRow::field("Type", KindCell::Text(service.service_type.clone().into())),
        DetailRow::field("Cluster IP", cluster_ip.clone()),
        DetailRow::field("External", external.clone()),
    ];
    if service.selector.is_empty() {
        details.push(DetailRow::Note(
            "No selector: endpoints are managed manually".into(),
        ));
    }
    let mut sections = vec![DetailSection {
        title: "Service",
        rows: details,
    }];
    if !service.ports.is_empty() {
        sections.push(DetailSection {
            title: "Ports",
            rows: service
                .ports
                .iter()
                .map(|port| DetailRow::Port {
                    text: port_text(port).into(),
                })
                .collect(),
        });
    }
    sections.push(DetailSection {
        title: "Selector",
        rows: vec![DetailRow::Chips(chips(&service.selector))],
    });
    sections.push(DetailSection {
        title: "Endpoints",
        rows: vec![DetailRow::Live(LiveContent::Endpoints)],
    });
    KindRow {
        namespace: Some(service.namespace.clone()),
        name: service.name.clone(),
        created_at: service.created_at,
        status: service_status(service),
        cells: vec![
            KindCell::Text(service.service_type.clone().into()),
            cluster_ip,
            external,
            KindCell::mono_or_absent(&ports.join(",")),
            // The endpoint join fills it once the pods and endpoint slices have loaded.
            KindCell::Absent,
            KindCell::age(service.created_at),
        ],
        sections,
        event: None,
        related_pods: None,
        labels: chips(&service.labels),
        object: KindObject::Service(service.clone()),
    }
}

pub(crate) fn ingress_row(ingress: &IngressSummary) -> KindRow {
    let has_tls = !ingress.tls.is_empty();
    let status = if ingress.addresses.is_empty() {
        StatusLabel {
            text: "No address yet".into(),
            tone: StatusTone::Warn,
        }
    } else {
        StatusLabel {
            text: "Address assigned".into(),
            tone: StatusTone::Ok,
        }
    };
    let hosts = if ingress.hosts.is_empty() {
        "*".to_owned()
    } else {
        ingress.hosts.join(",")
    };
    let address = KindCell::mono_or_absent(&ingress.addresses.join(","));
    let default_backend = match (&ingress.default_backend, &ingress.default_service) {
        (Some(backend), Some(service)) => {
            service_link("Default backend", backend, &ingress.namespace, service)
        }
        (Some(backend), None) => {
            DetailRow::field("Default backend", KindCell::Mono(backend.clone().into()))
        }
        (None, _) => DetailRow::field("Default backend", KindCell::Absent),
    };
    let mut sections = vec![
        DetailSection {
            title: "Ingress",
            rows: vec![
                DetailRow::field("Class", KindCell::text_or_absent(ingress.class.as_deref())),
                DetailRow::field("Address", address.clone()),
                default_backend,
            ],
        },
        DetailSection {
            title: "Rules",
            rows: ingress
                .rules
                .iter()
                .map(|rule| {
                    let label = format!(
                        "{}{}",
                        rule.host.as_deref().unwrap_or("*"),
                        rule.path.as_deref().unwrap_or("")
                    );
                    match &rule.service {
                        Some(service) => {
                            stacked_service_link(label, &rule.backend, &ingress.namespace, service)
                        }
                        None => {
                            DetailRow::stacked(label, KindCell::Mono(rule.backend.clone().into()))
                        }
                    }
                })
                .collect(),
        },
    ];
    if has_tls {
        sections.push(DetailSection {
            title: "TLS",
            rows: ingress
                .tls
                .iter()
                .map(|tls| {
                    let label = if tls.hosts.is_empty() {
                        "*".to_owned()
                    } else {
                        tls.hosts.join(", ")
                    };
                    let secret = tls
                        .secret_name
                        .as_ref()
                        .map(|name| format!("secret {name}"));
                    DetailRow::stacked(label, KindCell::text_or_absent(secret.as_deref()))
                })
                .collect(),
        });
    }
    KindRow {
        namespace: Some(ingress.namespace.clone()),
        name: ingress.name.clone(),
        created_at: ingress.created_at,
        status,
        cells: vec![
            KindCell::text_or_absent(ingress.class.as_deref()),
            KindCell::Text(hosts.into()),
            address,
            KindCell::Text(if has_tls { "80, 443" } else { "80" }.into()),
            KindCell::age(ingress.created_at),
        ],
        sections,
        event: None,
        related_pods: None,
        labels: chips(&ingress.labels),
        object: KindObject::Ingress(ingress.clone()),
    }
}

/// A row whose backend text opens the Service it names.
fn service_link(label: &str, backend: &str, namespace: &str, service: &str) -> DetailRow {
    match ResourceKey::of_object("Service", Some(namespace), service) {
        Some(target) => DetailRow::Link {
            label: label.to_owned().into(),
            text: backend.to_owned().into(),
            target,
        },
        None => DetailRow::field(label.to_owned(), KindCell::Mono(backend.to_owned().into())),
    }
}

/// `service_link` with the label above the text, for a rule's host and path.
fn stacked_service_link(label: String, backend: &str, namespace: &str, service: &str) -> DetailRow {
    match ResourceKey::of_object("Service", Some(namespace), service) {
        Some(target) => DetailRow::StackedLink {
            label: label.into(),
            text: backend.to_owned().into(),
            target,
        },
        None => DetailRow::stacked(label, KindCell::Mono(backend.to_owned().into())),
    }
}

/// The URLs the Open URL action offers, in rule order and without duplicates. Only plain hosts and
/// paths become URLs, so nothing but `http` or `https` and a safe string ever reaches the browser.
pub(crate) fn ingress_urls(ingress: &IngressSummary) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for rule in &ingress.rules {
        let Some(host) = rule.host.as_deref().filter(|host| is_plain_host(host)) else {
            continue;
        };
        let scheme = if is_covered_by_tls(host, &ingress.tls) {
            "https"
        } else {
            "http"
        };
        let path = rule
            .path
            .as_deref()
            .filter(|path| is_plain_path(path))
            .unwrap_or("/");
        let url = format!("{scheme}://{host}{path}");
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    urls
}

/// Letters, digits, `.` and `-` only: a wildcard host (`*.example.com`) is not one.
fn is_plain_host(host: &str) -> bool {
    !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// A path of unreserved characters and `/`; anything else (a regex, a query) falls back to `/`.
fn is_plain_path(path: &str) -> bool {
    path.starts_with('/')
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.' | '~'))
}

/// Whether a TLS entry lists `host`, or a wildcard `*.suffix` that covers exactly one more label.
fn is_covered_by_tls(host: &str, tls: &[IngressTls]) -> bool {
    tls.iter()
        .flat_map(|entry| &entry.hosts)
        .any(|listed| match listed.strip_prefix("*") {
            Some(suffix) if suffix.starts_with('.') => host
                .strip_suffix(suffix)
                .is_some_and(|label| !label.is_empty() && !label.contains('.')),
            _ => listed == host,
        })
}

/// The status a Service has before its pods and endpoint slices are known; the endpoint join
/// replaces it once they are.
pub(crate) fn service_status(service: &ServiceSummary) -> StatusLabel {
    if is_address_pending(service) {
        return StatusLabel {
            text: "Address pending".into(),
            tone: StatusTone::Warn,
        };
    }
    StatusLabel {
        text: service.service_type.clone().into(),
        tone: StatusTone::Ok,
    }
}

/// A LoadBalancer the cloud has not given an address yet.
pub(crate) fn is_address_pending(service: &ServiceSummary) -> bool {
    service.service_type == LOAD_BALANCER && service.external_addresses.is_empty()
}

/// `None` (muted) for a headless service, else the cluster IPs.
fn cluster_ip_cell(service: &ServiceSummary) -> KindCell {
    if service.is_headless {
        return KindCell::Toned(StatusLabel {
            text: "None".into(),
            tone: StatusTone::Done,
        });
    }
    KindCell::mono_or_absent(&service.cluster_ips.join(","))
}

/// `{port} → {target}/{protocol} · {name} · node {nodePort}`. The port comes first so a
/// truncated line still shows it.
fn port_text(port: &ServicePortSummary) -> String {
    let mut text = match &port.target_port {
        Some(target) => format!("{} → {target}/{}", port.port, port.protocol),
        None => format!("{}/{}", port.port, port.protocol),
    };
    if let Some(name) = &port.name {
        text.push_str(&format!(" · {name}"));
    }
    if let Some(node_port) = port.node_port {
        text.push_str(&format!(" · node {node_port}"));
    }
    text
}

#[cfg(test)]
#[path = "network_rows_tests.rs"]
mod network_rows_tests;
