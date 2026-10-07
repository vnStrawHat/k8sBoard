//! The spec side of a container summary: resources, probes, env, and mounts.
//!
//! Secret safety: only names and source references are read. `EnvVar.value`, exec
//! commands, HTTP headers, HTTP hosts, and query strings are never touched, and nothing
//! here logs.

use std::collections::{BTreeMap, BTreeSet};

use k8s_openapi::api::core::v1::{
    Container, EnvFromSource as ApiEnvFromSource, EnvVar, Probe, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

use crate::pod_status::non_negative;
use crate::workload::{int_or_string_text, non_empty};

const DEFAULT_PROBE_PERIOD_SECONDS: i32 = 10;
const DEFAULT_PROBE_FAILURE_THRESHOLD: i32 = 3;
const CONTAINER_RESOURCE_ORDER: [&str; 3] = ["cpu", "memory", "ephemeral-storage"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerResource {
    /// For example `cpu` or `memory`.
    pub name: String,
    /// The quantity as written, for example `500m`.
    pub request: Option<String>,
    pub limit: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContainerProbes {
    pub liveness: Option<ProbeSummary>,
    pub readiness: Option<ProbeSummary>,
    pub startup: Option<ProbeSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeSummary {
    pub action: ProbeAction,
    /// Defaults to 10.
    pub period_seconds: u32,
    /// Defaults to 3.
    pub failure_threshold: u32,
    /// `initialDelaySeconds`; defaults to 0.
    pub initial_delay_seconds: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeAction {
    HttpGet {
        /// Defaults to `HTTP`.
        scheme: String,
        /// A number or a port name.
        port: String,
        /// Without the query string. Defaults to `/`.
        path: String,
    },
    TcpSocket {
        /// A number or a port name.
        port: String,
    },
    Grpc {
        port: u16,
    },
    /// `exec.command`, the argv as written. It can hold credentials, so nothing logs it.
    Exec {
        command: Vec<String>,
    },
    /// No handler is set, or the gRPC port is out of range.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvEntry {
    pub name: String,
    pub source: EnvSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvSource {
    /// The variable has a literal `value`. The value is never read.
    Literal,
    ConfigMapKey {
        name: String,
        key: String,
    },
    SecretKey {
        name: String,
        key: String,
    },
    /// `fieldRef.fieldPath`.
    Field {
        path: String,
    },
    /// `resourceFieldRef.resource`.
    ResourceField {
        resource: String,
    },
    /// `valueFrom` with no known reference.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvFromEntry {
    pub source: EnvFromSource,
    /// An empty prefix is `None`.
    pub prefix: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvFromSource {
    ConfigMap { name: String },
    Secret { name: String },
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountEntry {
    pub path: String,
    pub volume: String,
    pub source: VolumeSource,
    pub is_read_only: bool,
    /// An empty `subPath` is `None`.
    pub sub_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VolumeSource {
    ConfigMap {
        name: String,
    },
    Secret {
        name: String,
    },
    PersistentVolumeClaim {
        claim: String,
    },
    EmptyDir,
    HostPath {
        path: String,
    },
    Projected {
        /// The config map names of `projected.sources[].configMap`, in order.
        config_maps: Vec<String>,
        /// The secret names of `projected.sources[].secret`, in order.
        secrets: Vec<String>,
    },
    DownwardApi,
    /// Any other volume type, or a volume name missing from `spec.volumes`.
    Other,
}

/// One named quantity pair, such as a request and a limit, or a capacity and an allocatable.
pub(crate) struct QuantityPair {
    pub(crate) name: String,
    pub(crate) first: Option<String>,
    pub(crate) second: Option<String>,
}

/// The union of both maps' keys. Names in `priority` come first in that order, then the
/// rest by name. Quantities are kept as written.
pub(crate) fn quantity_pairs(
    first: Option<&BTreeMap<String, Quantity>>,
    second: Option<&BTreeMap<String, Quantity>>,
    priority: &[&str],
) -> Vec<QuantityPair> {
    let names: BTreeSet<&String> = first
        .into_iter()
        .chain(second)
        .flat_map(BTreeMap::keys)
        .collect();
    let mut pairs: Vec<_> = names
        .into_iter()
        .map(|name| QuantityPair {
            name: name.clone(),
            first: first
                .and_then(|map| map.get(name))
                .map(|value| value.0.clone()),
            second: second
                .and_then(|map| map.get(name))
                .map(|value| value.0.clone()),
        })
        .collect();
    // The sort is stable, so names outside `priority` keep their alphabetical order.
    pairs.sort_by_key(|pair| {
        priority
            .iter()
            .position(|name| *name == pair.name)
            .unwrap_or(priority.len())
    });
    pairs
}

pub(crate) fn container_resources(container: &Container) -> Vec<ContainerResource> {
    let resources = container.resources.as_ref();
    quantity_pairs(
        resources.and_then(|resources| resources.requests.as_ref()),
        resources.and_then(|resources| resources.limits.as_ref()),
        &CONTAINER_RESOURCE_ORDER,
    )
    .into_iter()
    .map(|pair| ContainerResource {
        name: pair.name,
        request: pair.first,
        limit: pair.second,
    })
    .collect()
}

pub(crate) fn container_probes(container: &Container) -> ContainerProbes {
    ContainerProbes {
        liveness: container.liveness_probe.as_ref().map(probe_summary),
        readiness: container.readiness_probe.as_ref().map(probe_summary),
        startup: container.startup_probe.as_ref().map(probe_summary),
    }
}

fn probe_summary(probe: &Probe) -> ProbeSummary {
    ProbeSummary {
        action: probe_action(probe),
        period_seconds: non_negative(probe.period_seconds.unwrap_or(DEFAULT_PROBE_PERIOD_SECONDS)),
        failure_threshold: non_negative(
            probe
                .failure_threshold
                .unwrap_or(DEFAULT_PROBE_FAILURE_THRESHOLD),
        ),
        initial_delay_seconds: non_negative(probe.initial_delay_seconds.unwrap_or(0)),
    }
}

fn probe_action(probe: &Probe) -> ProbeAction {
    if let Some(http) = &probe.http_get {
        let path = http.path.as_deref().unwrap_or_default();
        // Only the path survives: query strings can carry tokens, and headers and the
        // host are never read.
        let path = path.split('?').next().unwrap_or_default();
        return ProbeAction::HttpGet {
            scheme: non_empty(http.scheme.as_deref()).unwrap_or_else(|| "HTTP".to_owned()),
            port: int_or_string_text(&http.port),
            path: if path.is_empty() { "/" } else { path }.to_owned(),
        };
    }
    if let Some(tcp) = &probe.tcp_socket {
        return ProbeAction::TcpSocket {
            port: int_or_string_text(&tcp.port),
        };
    }
    if let Some(grpc) = &probe.grpc {
        return u16::try_from(grpc.port)
            .map_or(ProbeAction::Unknown, |port| ProbeAction::Grpc { port });
    }
    if let Some(exec) = &probe.exec {
        return ProbeAction::Exec {
            command: exec.command.clone().unwrap_or_default(),
        };
    }
    ProbeAction::Unknown
}

pub(crate) fn env_entries(container: &Container) -> Vec<EnvEntry> {
    container.env.iter().flatten().map(env_entry).collect()
}

fn env_entry(variable: &EnvVar) -> EnvEntry {
    // `variable.value` is deliberately never read.
    let source = match &variable.value_from {
        None => EnvSource::Literal,
        Some(from) => {
            if let Some(config_map) = &from.config_map_key_ref {
                EnvSource::ConfigMapKey {
                    name: config_map.name.clone(),
                    key: config_map.key.clone(),
                }
            } else if let Some(secret) = &from.secret_key_ref {
                EnvSource::SecretKey {
                    name: secret.name.clone(),
                    key: secret.key.clone(),
                }
            } else if let Some(field) = &from.field_ref {
                EnvSource::Field {
                    path: field.field_path.clone(),
                }
            } else if let Some(resource) = &from.resource_field_ref {
                EnvSource::ResourceField {
                    resource: resource.resource.clone(),
                }
            } else {
                EnvSource::Unknown
            }
        }
    };
    EnvEntry {
        name: variable.name.clone(),
        source,
    }
}

pub(crate) fn env_from_entries(container: &Container) -> Vec<EnvFromEntry> {
    container
        .env_from
        .iter()
        .flatten()
        .map(env_from_entry)
        .collect()
}

fn env_from_entry(entry: &ApiEnvFromSource) -> EnvFromEntry {
    let source = if let Some(config_map) = &entry.config_map_ref {
        EnvFromSource::ConfigMap {
            name: config_map.name.clone(),
        }
    } else if let Some(secret) = &entry.secret_ref {
        EnvFromSource::Secret {
            name: secret.name.clone(),
        }
    } else {
        EnvFromSource::Unknown
    };
    EnvFromEntry {
        source,
        prefix: non_empty(entry.prefix.as_deref()),
    }
}

pub(crate) fn mount_entries(container: &Container, volumes: &[Volume]) -> Vec<MountEntry> {
    container
        .volume_mounts
        .iter()
        .flatten()
        .map(|mount| mount_entry(mount, volumes))
        .collect()
}

fn mount_entry(mount: &VolumeMount, volumes: &[Volume]) -> MountEntry {
    let source = volumes
        .iter()
        .find(|volume| volume.name == mount.name)
        .map_or(VolumeSource::Other, volume_source);
    MountEntry {
        path: mount.mount_path.clone(),
        volume: mount.name.clone(),
        source,
        is_read_only: mount.read_only.unwrap_or(false),
        sub_path: non_empty(mount.sub_path.as_deref()),
    }
}

fn volume_source(volume: &Volume) -> VolumeSource {
    if let Some(config_map) = &volume.config_map {
        VolumeSource::ConfigMap {
            name: config_map.name.clone(),
        }
    } else if let Some(secret) = &volume.secret {
        VolumeSource::Secret {
            name: secret.secret_name.clone().unwrap_or_default(),
        }
    } else if let Some(claim) = &volume.persistent_volume_claim {
        VolumeSource::PersistentVolumeClaim {
            claim: claim.claim_name.clone(),
        }
    } else if volume.empty_dir.is_some() {
        VolumeSource::EmptyDir
    } else if let Some(host_path) = &volume.host_path {
        VolumeSource::HostPath {
            path: host_path.path.clone(),
        }
    } else if let Some(projected) = &volume.projected {
        VolumeSource::Projected {
            config_maps: projected
                .sources
                .iter()
                .flatten()
                .filter_map(|source| source.config_map.as_ref())
                .map(|config_map| config_map.name.clone())
                .collect(),
            secrets: projected
                .sources
                .iter()
                .flatten()
                .filter_map(|source| source.secret.as_ref())
                .map(|secret| secret.name.clone())
                .collect(),
        }
    } else if volume.downward_api.is_some() {
        VolumeSource::DownwardApi
    } else {
        VolumeSource::Other
    }
}

/// The digest of `status.imageID`: the text after its last `@`, else the whole id when it
/// is a bare `sha256:` digest.
pub(crate) fn image_digest(image_id: &str) -> Option<String> {
    let digest = match image_id.rsplit_once('@') {
        Some((_, digest)) => digest,
        None if image_id.starts_with("sha256:") => image_id,
        None => return None,
    };
    non_empty(Some(digest))
}

#[cfg(test)]
#[path = "container_spec_tests.rs"]
mod container_spec_tests;
