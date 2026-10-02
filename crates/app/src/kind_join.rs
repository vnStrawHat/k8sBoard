//! Cross-list cells of explorer rows, joined on the main thread after each relevant update: the
//! pods list and the endpoint-slice companion fill a Service's status and Endpoints cell. Pure:
//! the session passes its live lists and the rows are rewritten in place. Each join starts from
//! the row builder's own status and cells, so a rerun never builds on an earlier join.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use cluster::{
    ByteAmount, CpuAmount, EndpointPort, EndpointSliceSummary, EndpointSummary, EnvFromSource,
    EnvSource, NamespaceScope, PodSummary, Selector, ServiceSummary, VolumeSource,
};

use crate::cluster_session::{CompanionLists, LiveList};
use crate::kind_row::{KindCell, KindObject, KindRow, deployment_of_replica_set};
use crate::network_rows::{is_address_pending, service_status};
use crate::node_usage::{requests_of, takes_room};
use crate::resource_kind::ResourceKind;
use crate::status_tone::{StatusLabel, StatusTone};
use crate::table_selection::ResourceKey;
use crate::usage_format::Measure;

/// The index of the Endpoints cell in a Services row (the Name column is not a cell).
pub(crate) const SERVICE_ENDPOINTS: usize = 4;
/// The index of the Used by cell in a ConfigMaps row.
pub(crate) const CONFIG_MAP_USED_BY: usize = 1;
/// The indices of the Pods, CPU req, and Memory req cells in a Namespaces row.
pub(crate) const NAMESPACE_PODS: usize = 1;
pub(crate) const NAMESPACE_CPU: usize = 2;
pub(crate) const NAMESPACE_MEMORY: usize = 3;

const EXTERNAL_NAME: &str = "ExternalName";
/// Slices of this address type name hosts, not pods; counting them would double a dual-stack
/// service.
const FQDN: &str = "FQDN";

/// What a join reads besides the rows.
pub(crate) struct JoinInputs<'a> {
    pub(crate) pods: &'a LiveList<PodSummary>,
    /// EndpointSlices for the Services screen.
    pub(crate) companion: Option<&'a CompanionLists>,
    /// The pods list covers only this scope, so a Namespaces row outside it has no pod numbers.
    pub(crate) scope: &'a NamespaceScope,
}

/// Rewrites the joined cells (and the Service status) of `rows`. Other kinds: no-op.
pub(crate) fn join_rows(kind: ResourceKind, rows: &mut [KindRow], inputs: &JoinInputs) {
    match kind {
        ResourceKind::Services => join_services(rows, inputs),
        ResourceKind::ConfigMaps => join_config_maps(rows, inputs),
        ResourceKind::Namespaces => join_namespaces(rows, inputs),
        _ => {}
    }
}

/// What the pods and endpoint slices say about a Service. `None` means not loaded, denied, or
/// not applicable (an ExternalName or selector-less Service has no matching pods to count).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ServiceHealth {
    pub(crate) matching_pods: Option<usize>,
    pub(crate) endpoints: Option<EndpointCounts>,
}

impl ServiceHealth {
    /// Endpoints exist and none takes traffic (the WHY box V2).
    pub(crate) fn is_unserved(&self) -> bool {
        self.endpoints
            .is_some_and(|counts| counts.total > 0 && counts.ready == 0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EndpointCounts {
    pub(crate) ready: usize,
    /// Terminating endpoints are not part of the total.
    pub(crate) total: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EndpointState {
    Ready,
    NotReady,
    Terminating,
}

/// One endpoint of a Service after the slices were merged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EndpointEntry<'a> {
    pub(crate) endpoint: &'a EndpointSummary,
    pub(crate) state: EndpointState,
}

/// The pure core: counts the pods `service` selects among `pods` and the endpoints of its slices
/// in `slices`. `slices` is `None` while they are not loaded (or not permitted); an empty slice
/// list is loaded and means no endpoints.
pub(crate) fn service_health(
    service: &ServiceSummary,
    pods: &[PodSummary],
    slices: Option<&[EndpointSliceSummary]>,
) -> ServiceHealth {
    let namespace_pods: Vec<&PodSummary> = pods
        .iter()
        .filter(|pod| pod.namespace == service.namespace)
        .collect();
    let service_slices = slices.map(|slices| service_slices(service, slices));
    health_of(service, &namespace_pods, service_slices.as_deref())
}

/// `service_health` for the session's lists: a pods list that has not loaded leaves
/// `matching_pods` unknown, and slices that have not loaded leave `endpoints` unknown.
pub(crate) fn service_health_of(
    service: &ServiceSummary,
    pods: &LiveList<PodSummary>,
    slices: Option<&LiveList<EndpointSliceSummary>>,
) -> ServiceHealth {
    let loaded_pods = pods.ready_items();
    let health = service_health(
        service,
        loaded_pods.unwrap_or(&[]),
        slices.and_then(LiveList::ready_items),
    );
    ServiceHealth {
        matching_pods: loaded_pods.and(health.matching_pods),
        ..health
    }
}

/// The pods of `pods` that `service` selects, in list order. A selector-less Service selects none.
pub(crate) fn matching_pods<'a>(
    service: &ServiceSummary,
    pods: &'a [PodSummary],
) -> Vec<&'a PodSummary> {
    let Some(selector) = Selector::of_labels(&service.selector) else {
        return Vec::new();
    };
    pods.iter()
        .filter(|pod| pod.namespace == service.namespace && selector.matches(&pod.labels))
        .collect()
}

/// The slices that belong to `service`: its namespace, the service-name label, no FQDN slices.
pub(crate) fn service_slices<'a>(
    service: &ServiceSummary,
    slices: &'a [EndpointSliceSummary],
) -> Vec<&'a EndpointSliceSummary> {
    slices
        .iter()
        .filter(|slice| {
            slice.namespace == service.namespace
                && slice.service.as_deref() == Some(service.name.as_str())
                && slice.address_type != FQDN
        })
        .collect()
}

/// The endpoints of `slices`, each pod (else each address) once: a dual-stack service lists a
/// pod in an IPv4 and an IPv6 slice. The first copy decides the state.
pub(crate) fn endpoint_entries<'a>(slices: &[&'a EndpointSliceSummary]) -> Vec<EndpointEntry<'a>> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for endpoint in slices.iter().flat_map(|slice| &slice.endpoints) {
        let key = match &endpoint.pod {
            Some(pod) => (true, pod.as_str()),
            None => (false, endpoint.address.as_str()),
        };
        if !seen.insert(key) {
            continue;
        }
        // Only `ready` decides traffic for a non-terminating endpoint; `serving` is not read.
        let state = if endpoint.is_terminating {
            EndpointState::Terminating
        } else if endpoint.is_ready {
            EndpointState::Ready
        } else {
            EndpointState::NotReady
        };
        entries.push(EndpointEntry { endpoint, state });
    }
    entries
}

/// The distinct ports of `slices`, in first-seen order.
pub(crate) fn endpoint_ports<'a>(slices: &[&'a EndpointSliceSummary]) -> Vec<&'a EndpointPort> {
    let mut ports: Vec<&EndpointPort> = Vec::new();
    for port in slices.iter().flat_map(|slice| &slice.ports) {
        let is_seen = ports
            .iter()
            .any(|seen| seen.port == port.port && seen.protocol == port.protocol);
        if port.port.is_some() && !is_seen {
            ports.push(port);
        }
    }
    ports
}

fn health_of(
    service: &ServiceSummary,
    namespace_pods: &[&PodSummary],
    slices: Option<&[&EndpointSliceSummary]>,
) -> ServiceHealth {
    if service.service_type == EXTERNAL_NAME {
        return ServiceHealth::default();
    }
    ServiceHealth {
        matching_pods: Selector::of_labels(&service.selector).map(|selector| {
            namespace_pods
                .iter()
                .filter(|pod| selector.matches(&pod.labels))
                .count()
        }),
        endpoints: slices.map(endpoint_counts),
    }
}

fn endpoint_counts(slices: &[&EndpointSliceSummary]) -> EndpointCounts {
    let entries = endpoint_entries(slices);
    let count = |is_counted: fn(EndpointState) -> bool| {
        entries
            .iter()
            .filter(|entry| is_counted(entry.state))
            .count()
    };
    EndpointCounts {
        ready: count(|state| state == EndpointState::Ready),
        total: count(|state| state != EndpointState::Terminating),
    }
}

fn join_services(rows: &mut [KindRow], inputs: &JoinInputs) {
    let pods = inputs.pods.ready_items();
    let pods_by_namespace = pods_by_namespace(pods.unwrap_or(&[]));
    let slices = inputs
        .companion
        .and_then(CompanionLists::endpoint_slices)
        .and_then(LiveList::ready_items);
    let slices_by_service = slices.map(slices_by_service);
    for row in rows {
        let KindObject::Service(service) = &row.object else {
            continue;
        };
        let namespace_pods = pods_by_namespace
            .get(service.namespace.as_str())
            .map_or(&[][..], Vec::as_slice);
        let service_slices = slices_by_service.as_ref().map(|index| {
            index
                .get(&(service.namespace.as_str(), service.name.as_str()))
                .map_or(&[][..], Vec::as_slice)
        });
        let mut health = health_of(service, namespace_pods, service_slices);
        if pods.is_none() {
            health.matching_pods = None;
        }
        let (status, endpoints) = service_state(service, health);
        row.status = status;
        if let Some(cell) = row.cells.get_mut(SERVICE_ENDPOINTS) {
            *cell = endpoints;
        }
    }
}

/// One pass over the pods, so each Service reads only the pods of its namespace.
fn pods_by_namespace(pods: &[PodSummary]) -> HashMap<&str, Vec<&PodSummary>> {
    let mut index: HashMap<&str, Vec<&PodSummary>> = HashMap::new();
    for pod in pods {
        index.entry(pod.namespace.as_str()).or_default().push(pod);
    }
    index
}

/// Slices by `(namespace, service)`; those without the service label or with FQDN addresses
/// belong to no service and are left out.
fn slices_by_service(
    slices: &[EndpointSliceSummary],
) -> HashMap<(&str, &str), Vec<&EndpointSliceSummary>> {
    let mut index: HashMap<(&str, &str), Vec<&EndpointSliceSummary>> = HashMap::new();
    for slice in slices {
        let Some(service) = slice.service.as_deref() else {
            continue;
        };
        if slice.address_type == FQDN {
            continue;
        }
        index
            .entry((slice.namespace.as_str(), service))
            .or_default()
            .push(slice);
    }
    index
}

/// The status and the Endpoints cell of a Service; the first matching case wins.
fn service_state(service: &ServiceSummary, health: ServiceHealth) -> (StatusLabel, KindCell) {
    let builder_status = service_status(service);
    if service.service_type == EXTERNAL_NAME {
        return (builder_status, KindCell::Absent);
    }
    let Some(verdict) = endpoints_verdict(service, health) else {
        return (builder_status, KindCell::Absent);
    };
    // A LoadBalancer without an address keeps the louder fact in its status.
    let status = if is_address_pending(service) {
        builder_status
    } else {
        verdict.status
    };
    (status, KindCell::Toned(verdict.cell))
}

struct EndpointsVerdict {
    status: StatusLabel,
    cell: StatusLabel,
}

fn endpoints_verdict(service: &ServiceSummary, health: ServiceHealth) -> Option<EndpointsVerdict> {
    let verdict = |tone, status: String, cell: String| EndpointsVerdict {
        status: StatusLabel {
            text: status.into(),
            tone,
        },
        cell: StatusLabel {
            text: cell.into(),
            tone,
        },
    };
    if health.matching_pods == Some(0) {
        return Some(verdict(
            StatusTone::Bad,
            "Matches no pods".to_owned(),
            "0".to_owned(),
        ));
    }
    let EndpointCounts { ready, total } = health.endpoints?;
    Some(if total == 0 {
        // A selector-less Service has its slices managed by hand, so none is a warning only.
        let tone = if service.selector.is_empty() {
            StatusTone::Warn
        } else {
            StatusTone::Bad
        };
        verdict(tone, "No endpoints".to_owned(), "0".to_owned())
    } else if ready == total {
        let noun = if total == 1 { "endpoint" } else { "endpoints" };
        verdict(
            StatusTone::Ok,
            format!("{total} {noun} ready"),
            total.to_string(),
        )
    } else {
        let tone = if ready == 0 {
            StatusTone::Bad
        } else {
            StatusTone::Warn
        };
        let noun = if total == 1 { "endpoint" } else { "endpoints" };
        verdict(
            tone,
            format!("{ready} of {total} {noun} ready"),
            format!("{ready} of {total}"),
        )
    })
}

// ---- ConfigMaps ----

/// How a pod's owner refers to a config map.
const WAY_ENV: &str = "env";
const WAY_ENV_FROM: &str = "env from";
const WAY_VOLUME: &str = "volume";
/// A Job the CronJob controller creates is named `{cronjob}-{scheduled minute}`, at least this many
/// digits.
const CRON_JOB_SUFFIX_DIGITS: usize = 8;

/// One workload (or bare pod) that reads a config map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UsedBy {
    /// `deployment/api`, `cronjob/nightly`, `statefulset/db`, or `pod/{name}`.
    pub(crate) owner: String,
    /// What the owner link opens; `None` when k8sBoard has no screen for the owner's kind.
    pub(crate) target: Option<ResourceKey>,
    /// `env`, `env from`, `volume`.
    pub(crate) ways: BTreeSet<&'static str>,
}

/// Namespace, then config map name, then the users by owner text (sorted, one entry per owner).
pub(crate) type ConfigMapUsers = HashMap<String, HashMap<String, BTreeMap<String, UsedBy>>>;

/// Which config maps the pods use, from env, envFrom, mounted volumes, and projected sources of
/// every container kind. A workload with no pod running right now is missed (README open item 1).
pub(crate) fn config_map_users<'a>(
    pods: impl IntoIterator<Item = &'a PodSummary>,
) -> ConfigMapUsers {
    let mut users = ConfigMapUsers::new();
    for pod in pods {
        let mut ways: HashMap<&str, BTreeSet<&'static str>> = HashMap::new();
        for container in &pod.containers {
            for entry in &container.env {
                if let EnvSource::ConfigMapKey { name, .. } = &entry.source {
                    ways.entry(name).or_default().insert(WAY_ENV);
                }
            }
            for entry in &container.env_from {
                if let EnvFromSource::ConfigMap { name } = &entry.source {
                    ways.entry(name).or_default().insert(WAY_ENV_FROM);
                }
            }
            for mount in &container.mounts {
                match &mount.source {
                    VolumeSource::ConfigMap { name } => {
                        ways.entry(name).or_default().insert(WAY_VOLUME);
                    }
                    VolumeSource::Projected { config_maps } => {
                        for name in config_maps {
                            ways.entry(name).or_default().insert(WAY_VOLUME);
                        }
                    }
                    _ => {}
                }
            }
        }
        if ways.is_empty() {
            continue;
        }
        let (owner, target) = pod_owner(pod);
        let in_namespace = users.entry(pod.namespace.clone()).or_default();
        for (name, pod_ways) in ways {
            let used_by = in_namespace
                .entry(name.to_owned())
                .or_default()
                .entry(owner.clone())
                .or_insert_with(|| UsedBy {
                    owner: owner.clone(),
                    target: target.clone(),
                    ways: BTreeSet::new(),
                });
            used_by.ways.extend(pod_ways);
        }
    }
    users
}

/// The users of one config map, sorted by owner.
pub(crate) fn users_of<'a>(
    users: &'a ConfigMapUsers,
    namespace: &str,
    name: &str,
) -> impl Iterator<Item = &'a UsedBy> {
    users
        .get(namespace)
        .and_then(|in_namespace| in_namespace.get(name))
        .into_iter()
        .flat_map(BTreeMap::values)
}

/// The top owner by naming convention, without a lookup: a ReplicaSet `{d}-{hash}` is
/// `deployment/{d}`, a Job `{c}-{digits}` is `cronjob/{c}`, any other controller is
/// `{kind}/{name}`, and a pod without one is `pod/{name}`.
fn pod_owner(pod: &PodSummary) -> (String, Option<ResourceKey>) {
    let namespace = Some(pod.namespace.as_str());
    let Some(controller) = &pod.controller else {
        return (format!("pod/{}", pod.name), Some(ResourceKey::of_pod(pod)));
    };
    let by_convention = match controller.kind.as_str() {
        "ReplicaSet" => deployment_of_replica_set(&controller.name)
            .map(|deployment| ("deployment", "Deployment", deployment)),
        "Job" => cron_job_of_job(&controller.name).map(|cron_job| ("cronjob", "CronJob", cron_job)),
        _ => None,
    };
    match by_convention {
        Some((prefix, kind, name)) => (
            format!("{prefix}/{name}"),
            ResourceKey::of_object(kind, namespace, name),
        ),
        None => (
            format!("{}/{}", controller.kind.to_lowercase(), controller.name),
            ResourceKey::of_owner(&pod.namespace, controller),
        ),
    }
}

/// `nightly` for a Job named `nightly-29012345`.
// ponytail: a hand-made Job named `backup-20240101` also maps to `cronjob/backup`; the Job's
// owner reference names no CronJob, so only the name tells them apart. A Jobs lookup would settle it.
fn cron_job_of_job(job: &str) -> Option<&str> {
    let (cron_job, suffix) = job.rsplit_once('-')?;
    let is_scheduled_minute =
        suffix.len() >= CRON_JOB_SUFFIX_DIGITS && suffix.chars().all(|c| c.is_ascii_digit());
    (is_scheduled_minute && !cron_job.is_empty()).then_some(cron_job)
}

fn join_config_maps(rows: &mut [KindRow], inputs: &JoinInputs) {
    let users = inputs.pods.ready_items().map(config_map_users);
    for row in rows {
        let (Some(namespace), KindObject::ConfigMap(_)) = (&row.namespace, &row.object) else {
            continue;
        };
        let cell = users.as_ref().map_or(KindCell::Absent, |users| {
            used_by_cell(users_of(users, namespace, &row.name))
        });
        if let Some(slot) = row.cells.get_mut(CONFIG_MAP_USED_BY) {
            *slot = cell;
        }
    }
}

/// The first owner, plus ` +{n}` for the others; `Absent` for none.
fn used_by_cell<'a>(mut users: impl Iterator<Item = &'a UsedBy>) -> KindCell {
    let Some(first) = users.next() else {
        return KindCell::Absent;
    };
    match users.count() {
        0 => KindCell::Mono(first.owner.clone().into()),
        more => KindCell::MonoWithMore {
            text: first.owner.clone().into(),
            more,
        },
    }
}

// ---- Namespaces ----

/// What the pods of a namespace add up to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NamespaceLoad {
    /// Every pod of the namespace, finished ones included.
    pub(crate) pods: usize,
    /// The requests of the pods that still take room (the node rule of 0010).
    pub(crate) cpu: CpuAmount,
    pub(crate) memory: ByteAmount,
}

pub(crate) fn namespace_load(pods: &[&PodSummary]) -> NamespaceLoad {
    let (cpu, memory) = requests_of(pods.iter().copied().filter(|pod| takes_room(pod)));
    NamespaceLoad {
        pods: pods.len(),
        cpu,
        memory,
    }
}

fn join_namespaces(rows: &mut [KindRow], inputs: &JoinInputs) {
    let pods = inputs.pods.ready_items().map(pods_by_namespace);
    for row in rows {
        // The pods list covers the scope only; a namespace outside it would read as empty.
        let is_in_scope = match inputs.scope {
            NamespaceScope::All => true,
            scope => scope.namespaces().contains(&row.name),
        };
        let cells = match (&pods, is_in_scope) {
            (Some(pods), true) => {
                let namespace_pods = pods.get(row.name.as_str()).map_or(&[][..], Vec::as_slice);
                load_cells(namespace_load(namespace_pods))
            }
            _ => [KindCell::Absent, KindCell::Absent, KindCell::Absent],
        };
        let indices = [NAMESPACE_PODS, NAMESPACE_CPU, NAMESPACE_MEMORY];
        for (index, cell) in indices.into_iter().zip(cells) {
            if let Some(slot) = row.cells.get_mut(index) {
                *slot = cell;
            }
        }
    }
}

/// The Pods, CPU req, and Memory req cells, in column order.
fn load_cells(load: NamespaceLoad) -> [KindCell; 3] {
    let bytes = load.memory.bytes();
    [
        KindCell::count(load.pods),
        KindCell::Quantity {
            text: Measure::Cpu.format(load.cpu.cores()).into(),
            value: load.cpu.nanocores(),
            tone: None,
        },
        KindCell::Quantity {
            text: Measure::Bytes.format(bytes as f64).into(),
            value: bytes,
            tone: None,
        },
    ]
}

#[cfg(test)]
#[path = "kind_join_tests.rs"]
mod kind_join_tests;
