//! Row builders for the network kinds: Services and Ingresses.

use cluster::{IngressSummary, ServicePortSummary, ServiceSummary};

use crate::kind_row::{
    DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent, chips,
};
use crate::status_tone::{StatusLabel, StatusTone};

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
    let default_backend = ingress
        .default_backend
        .as_ref()
        .map_or(KindCell::Absent, |backend| {
            KindCell::Mono(backend.clone().into())
        });
    let mut sections = vec![
        DetailSection {
            title: "Ingress",
            rows: vec![
                DetailRow::field("Class", KindCell::text_or_absent(ingress.class.as_deref())),
                DetailRow::field("Address", address.clone()),
                DetailRow::field("Default backend", default_backend),
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
                    DetailRow::stacked(label, KindCell::Mono(rule.backend.clone().into()))
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
        object: KindObject::Plain,
    }
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
