//! The Prometheus-compatible metrics source of a cluster (spec 0048): a service reached through the
//! API server proxy, its validation, and detection of likely candidates. Nothing here sends a
//! request, and no credential can be expressed.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::dns_name::is_dns_label;
use crate::service::{ServicePortSummary, ServiceSummary};

const MAX_PREFIX_LEN: usize = 128;
const MAX_PORT_NAME_LEN: usize = 15;
const MAX_CANDIDATES: usize = 20;
const NAME_LABEL: &str = "app.kubernetes.io/name";
const APP_LABEL: &str = "app";
const COMPONENT_LABEL: &str = "app.kubernetes.io/component";

/// What Settings stores. The port is a number or a port name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsSourceFields {
    pub namespace: String,
    pub service: String,
    pub port: String,
    pub scheme: MetricsScheme,
    #[serde(default)]
    pub prefix: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricsScheme {
    #[default]
    Http,
    Https,
}

impl MetricsScheme {
    fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }
}

/// Valid by construction: the fields are private and `new` is the only way in. It derives neither
/// `Deserialize` nor `Default`; settings deserialize into `MetricsSourceFields`, then `new`
/// validates. `Debug` prints `display()`.
#[derive(Clone, PartialEq, Eq)]
pub struct MetricsSource {
    fields: MetricsSourceFields,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MetricsSourceError {
    #[error("Use a namespace name.")]
    Namespace,
    #[error("Use a service name.")]
    Service,
    #[error("Use a port number or the service's port name.")]
    Port,
    #[error("Use a path such as /select/0/prometheus.")]
    Prefix,
}

impl MetricsSource {
    pub fn new(fields: &MetricsSourceFields) -> Result<Self, MetricsSourceError> {
        if !is_dns_label(&fields.namespace) {
            return Err(MetricsSourceError::Namespace);
        }
        if !is_dns_label(&fields.service) {
            return Err(MetricsSourceError::Service);
        }
        if !is_valid_port(&fields.port) {
            return Err(MetricsSourceError::Port);
        }
        if !is_valid_prefix(&fields.prefix) {
            return Err(MetricsSourceError::Prefix);
        }
        Ok(Self {
            fields: fields.clone(),
        })
    }

    pub fn fields(&self) -> MetricsSourceFields {
        self.fields.clone()
    }

    /// `monitoring/vmselect-x:8481 /select/0/prometheus`; the prefix part is left out when empty.
    pub fn display(&self) -> String {
        let fields = &self.fields;
        let location = format!("{}/{}:{}", fields.namespace, fields.service, fields.port);
        if fields.prefix.is_empty() {
            location
        } else {
            format!("{location} {}", fields.prefix)
        }
    }

    /// The API server path up to the backend's own path: every part was validated by `new`, so
    /// none can change the path a request goes to.
    pub(crate) fn proxy_path(&self) -> String {
        let fields = &self.fields;
        format!(
            "/api/v1/namespaces/{}/services/{}:{}:{}/proxy{}",
            fields.namespace,
            fields.scheme.as_str(),
            fields.service,
            fields.port,
            fields.prefix
        )
    }
}

impl fmt::Debug for MetricsSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.display())
    }
}

/// A canonical number in `1..=65535`, or an IANA service name: 1 to 15 of `[a-z0-9-]` with at least
/// one letter, no leading, trailing, or doubled `-`.
fn is_valid_port(port: &str) -> bool {
    if port.bytes().all(|byte| byte.is_ascii_digit()) {
        return !port.starts_with('0') && port.parse::<u16>().is_ok();
    }
    (1..=MAX_PORT_NAME_LEN).contains(&port.len())
        && port
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && port.bytes().any(|byte| byte.is_ascii_lowercase())
        && !port.starts_with('-')
        && !port.ends_with('-')
        && !port.contains("--")
}

/// Empty, or a leading `/` and `/`-separated non-empty segments of `[A-Za-z0-9._~-]`; no `.` or
/// `..` segment, no trailing `/`.
fn is_valid_prefix(prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let Some(segments) = prefix.strip_prefix('/') else {
        return false;
    };
    prefix.len() <= MAX_PREFIX_LEN
        && segments.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'-')
                })
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricsFlavor {
    VictoriaMetricsCluster,
    VictoriaMetricsQuery,
    VictoriaMetricsSingle,
    Prometheus,
    ThanosQuery,
    Mimir,
}

impl MetricsFlavor {
    pub fn label(self) -> &'static str {
        match self {
            Self::VictoriaMetricsCluster => "VictoriaMetrics cluster",
            Self::VictoriaMetricsQuery => "VictoriaMetrics query",
            Self::VictoriaMetricsSingle => "VictoriaMetrics single",
            Self::Prometheus => "Prometheus",
            Self::ThanosQuery => "Thanos Query",
            Self::Mimir => "Mimir",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricsCandidate {
    pub flavor: MetricsFlavor,
    pub fields: MetricsSourceFields,
}

/// One row of the detection table.
struct Detection {
    flavor: MetricsFlavor,
    prefix: &'static str,
    /// Tried in order: a service port with the first name, then the second, ...
    port_names: &'static [&'static str],
    /// Tried in order after the names.
    port_numbers: &'static [u16],
}

const VM_PREFIX: &str = "/select/0/prometheus";

/// Rank order of the detection table.
const DETECTIONS: [Detection; 6] = [
    Detection {
        flavor: MetricsFlavor::VictoriaMetricsCluster,
        prefix: VM_PREFIX,
        port_names: &["http"],
        port_numbers: &[8481],
    },
    Detection {
        flavor: MetricsFlavor::VictoriaMetricsQuery,
        prefix: VM_PREFIX,
        port_names: &["http"],
        port_numbers: &[8481],
    },
    Detection {
        flavor: MetricsFlavor::VictoriaMetricsSingle,
        prefix: "",
        port_names: &["http"],
        port_numbers: &[8428],
    },
    Detection {
        flavor: MetricsFlavor::Prometheus,
        prefix: "",
        port_names: &["web", "http-web", "http"],
        port_numbers: &[9090],
    },
    Detection {
        flavor: MetricsFlavor::ThanosQuery,
        prefix: "",
        port_names: &["http"],
        port_numbers: &[10902, 9090],
    },
    Detection {
        flavor: MetricsFlavor::Mimir,
        prefix: "/prometheus",
        port_names: &["http-metrics", "http"],
        port_numbers: &[8080],
    },
];

/// Services that look like a metrics source, best first (the table's rank, then namespace and
/// name), at most 20. A service with none of its candidate ports is skipped.
pub fn metrics_candidates(services: &[ServiceSummary]) -> Vec<MetricsCandidate> {
    let mut ranked: Vec<(usize, &ServiceSummary, MetricsCandidate)> = services
        .iter()
        .filter_map(|service| {
            let rank = detection_rank(service)?;
            let candidate = candidate_of(&DETECTIONS[rank], service)?;
            Some((rank, service, candidate))
        })
        .collect();
    ranked.sort_by(|(left_rank, left, _), (right_rank, right, _)| {
        left_rank
            .cmp(right_rank)
            .then_with(|| left.namespace.cmp(&right.namespace))
            .then_with(|| left.name.cmp(&right.name))
    });
    ranked
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|(_, _, candidate)| candidate)
        .collect()
}

/// The index into `DETECTIONS`, matching `app.kubernetes.io/name`, else `app`, exactly.
fn detection_rank(service: &ServiceSummary) -> Option<usize> {
    let app = label_value(service, NAME_LABEL).or_else(|| label_value(service, APP_LABEL));
    let component = label_value(service, COMPONENT_LABEL);
    match (app, service.name.as_str()) {
        (Some("vmselect"), _) => Some(0),
        (Some("vmquery"), _) => Some(1),
        (Some("vmsingle" | "victoria-metrics-single"), _) => Some(2),
        (Some("prometheus"), _)
        | (_, "prometheus-operated" | "prometheus-server" | "prometheus-k8s") => Some(3),
        (Some("thanos-query" | "thanos-querier"), _) => Some(4),
        (Some("mimir"), _) if component == Some("query-frontend") => Some(5),
        _ => None,
    }
}

fn label_value<'a>(service: &'a ServiceSummary, key: &str) -> Option<&'a str> {
    service.labels.iter().find_map(|term| {
        let (name, value) = term.split_once('=')?;
        (name == key).then_some(value)
    })
}

fn candidate_of(detection: &Detection, service: &ServiceSummary) -> Option<MetricsCandidate> {
    let port = detection_port(detection, &service.ports)?;
    let is_https = port
        .name
        .as_deref()
        .is_some_and(|name| name.starts_with("https"));
    let fields = MetricsSourceFields {
        namespace: service.namespace.clone(),
        service: service.name.clone(),
        // The number survives a port rename; the proxy accepts either.
        port: port.port.to_string(),
        scheme: if is_https {
            MetricsScheme::Https
        } else {
            MetricsScheme::Http
        },
        prefix: detection.prefix.to_owned(),
    };
    MetricsSource::new(&fields).ok()?;
    Some(MetricsCandidate {
        flavor: detection.flavor,
        fields,
    })
}

fn detection_port<'a>(
    detection: &Detection,
    ports: &'a [ServicePortSummary],
) -> Option<&'a ServicePortSummary> {
    let by_name = detection
        .port_names
        .iter()
        .find_map(|name| ports.iter().find(|port| port.name.as_deref() == Some(name)));
    by_name.or_else(|| {
        detection
            .port_numbers
            .iter()
            .find_map(|number| ports.iter().find(|port| port.port == *number))
    })
}

#[cfg(test)]
#[path = "metrics_source_tests.rs"]
mod metrics_source_tests;
