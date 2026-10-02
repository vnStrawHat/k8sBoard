//! Cross-list cells of explorer rows, joined on the main thread after each relevant update: the
//! pods list and the endpoint-slice companion fill a Service's status and Endpoints cell. Pure:
//! the session passes its live lists and the rows are rewritten in place. Each join starts from
//! the row builder's own status and cells, so a rerun never builds on an earlier join.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use cluster::{
    ByteAmount, CpuAmount, EndpointPort, EndpointSliceSummary, EndpointSummary, EnvFromSource,
    EnvSource, IngressSummary, NamespaceScope, PodSummary, PvcUsage, SecretDetails, SecretSummary,
    Selector, ServiceSummary, VolumeSource,
};

use crate::access_bindings::{
    BindingIndex, BoundRole, is_cluster_admin, pod_account, ready_binding_lists, role_text,
};
use crate::access_rows::service_account_status;
use crate::cluster_session::{CompanionLists, LiveList};
use crate::kind_row::{KindCell, KindObject, KindRow, deployment_of_replica_set};
use crate::kubelet_history::KubeletHistory;
use crate::network_policy_rows::network_policy_status;
use crate::network_rows::{is_address_pending, service_status};
use crate::node_usage::{requests_of, takes_room};
use crate::resource_kind::ResourceKind;
use crate::status_tone::{StatusLabel, StatusTone};
use crate::storage_rows::claim_status;
use crate::table_selection::ResourceKey;
use crate::usage_format::{Measure, format_percent, usage_tone};

/// The index of the Endpoints cell in a Services row (the Name column is not a cell).
pub(crate) const SERVICE_ENDPOINTS: usize = 4;
/// The index of the Used by cell in a ConfigMaps row.
pub(crate) const CONFIG_MAP_USED_BY: usize = 1;
/// The indices of the Pods, CPU req, and Memory req cells in a Namespaces row.
pub(crate) const NAMESPACE_PODS: usize = 1;
pub(crate) const NAMESPACE_CPU: usize = 2;
pub(crate) const NAMESPACE_MEMORY: usize = 3;
/// The index of the Affects cell in a NetworkPolicies row.
pub(crate) const NETWORK_POLICY_AFFECTS: usize = 2;
/// The index of the Used cell in a PVCs row.
pub(crate) const CLAIM_USED: usize = 2;
/// The index of the PVs cell in a StorageClasses row.
pub(crate) const CLASS_VOLUMES: usize = 5;
/// The index of the Bindings cell in a Roles row and in a ClusterRoles row.
pub(crate) const ROLE_BINDINGS: usize = 1;
pub(crate) const CLUSTER_ROLE_BINDINGS: usize = 2;
/// The indices of the Bound roles and Used by cells in a ServiceAccounts row.
pub(crate) const ACCOUNT_BOUND_ROLES: usize = 0;
pub(crate) const ACCOUNT_USED_BY: usize = 1;
/// The index of the Used by cell in a Secrets row.
pub(crate) const SECRET_USED_BY: usize = 2;

const EXTERNAL_NAME: &str = "ExternalName";
/// Slices of this address type name hosts, not pods; counting them would double a dual-stack
/// service.
const FQDN: &str = "FQDN";

/// What a join reads besides the rows.
pub(crate) struct JoinInputs<'a> {
    pub(crate) pods: &'a LiveList<PodSummary>,
    /// EndpointSlices for the Services screen.
    pub(crate) companion: Option<&'a CompanionLists>,
    /// The kubelet history for the PVCs screen; `None` without a metrics feed.
    pub(crate) kubelet: Option<&'a KubeletHistory>,
    /// The pods list covers only this scope, so a Namespaces row outside it has no pod numbers.
    pub(crate) scope: &'a NamespaceScope,
}

/// Rewrites the joined cells (and the Service status) of `rows`. Other kinds: no-op.
pub(crate) fn join_rows(kind: ResourceKind, rows: &mut [KindRow], inputs: &JoinInputs) {
    match kind {
        ResourceKind::Services => join_services(rows, inputs),
        ResourceKind::ConfigMaps => join_config_maps(rows, inputs),
        ResourceKind::Namespaces => join_namespaces(rows, inputs),
        ResourceKind::NetworkPolicies => join_network_policies(rows, inputs),
        ResourceKind::PersistentVolumeClaims => join_claims(rows, inputs),
        ResourceKind::StorageClasses => join_classes(rows, inputs),
        ResourceKind::Roles => join_roles(rows, inputs, ROLE_BINDINGS),
        ResourceKind::ClusterRoles => join_roles(rows, inputs, CLUSTER_ROLE_BINDINGS),
        ResourceKind::ServiceAccounts => join_service_accounts(rows, inputs),
        ResourceKind::Secrets => join_secrets(rows, inputs),
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
const WAY_IMAGE_PULL: &str = "image pull";
const WAY_TLS: &str = "tls";
const WAY_TOKEN: &str = "token";
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
                    VolumeSource::Projected { config_maps, .. } => {
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
        record_users(&mut users, &pod.namespace, &owner, target.as_ref(), ways);
    }
    users
}

/// Records that `owner` reaches each named object of `namespace` in the given ways.
fn record_users<'a>(
    users: &mut ConfigMapUsers,
    namespace: &str,
    owner: &str,
    target: Option<&ResourceKey>,
    ways: impl IntoIterator<Item = (&'a str, BTreeSet<&'static str>)>,
) {
    let in_namespace = users.entry(namespace.to_owned()).or_default();
    for (name, owner_ways) in ways {
        let used_by = in_namespace
            .entry(name.to_owned())
            .or_default()
            .entry(owner.to_owned())
            .or_insert_with(|| UsedBy {
                owner: owner.to_owned(),
                target: target.cloned(),
                ways: BTreeSet::new(),
            });
        used_by.ways.extend(owner_ways);
    }
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

// ---- Secrets ----

/// Namespace, then secret name, then the users by owner text: the shape of `ConfigMapUsers`.
pub(crate) type SecretUsers = ConfigMapUsers;

/// The Secret types a workload can leave unused without anything else naming them. TLS secrets are
/// left out on purpose: Gateway API, Istio, and cert-manager reference them without mounting.
const UNUSED_CANDIDATE_TYPES: [&str; 5] = [
    "Opaque",
    "kubernetes.io/basic-auth",
    "kubernetes.io/ssh-auth",
    "kubernetes.io/dockerconfigjson",
    "kubernetes.io/dockercfg",
];

/// Which secrets the pods and ingresses use: env, envFrom, volumes, projected sources, and image
/// pull secrets of every container kind, and the `tls` secret of each ingress.
pub(crate) fn secret_users<'a>(
    pods: impl IntoIterator<Item = &'a PodSummary>,
    ingresses: impl IntoIterator<Item = &'a IngressSummary>,
) -> SecretUsers {
    let mut users = SecretUsers::new();
    for pod in pods {
        let mut ways: HashMap<&str, BTreeSet<&'static str>> = HashMap::new();
        let mut add = |name: &'a str, way: &'static str| {
            if !name.is_empty() {
                ways.entry(name).or_default().insert(way);
            }
        };
        for container in &pod.containers {
            for entry in &container.env {
                if let EnvSource::SecretKey { name, .. } = &entry.source {
                    add(name, WAY_ENV);
                }
            }
            for entry in &container.env_from {
                if let EnvFromSource::Secret { name } = &entry.source {
                    add(name, WAY_ENV_FROM);
                }
            }
            for mount in &container.mounts {
                match &mount.source {
                    VolumeSource::Secret { name } => add(name, WAY_VOLUME),
                    VolumeSource::Projected { secrets, .. } => {
                        for name in secrets {
                            add(name, WAY_VOLUME);
                        }
                    }
                    _ => {}
                }
            }
        }
        for name in &pod.image_pull_secrets {
            add(name, WAY_IMAGE_PULL);
        }
        if ways.is_empty() {
            continue;
        }
        let (owner, target) = pod_owner(pod);
        record_users(&mut users, &pod.namespace, &owner, target.as_ref(), ways);
    }
    for ingress in ingresses {
        let names = ingress
            .tls
            .iter()
            .filter_map(|tls| tls.secret_name.as_deref())
            .filter(|name| !name.is_empty());
        let target = ResourceKey::of_object("Ingress", Some(&ingress.namespace), &ingress.name);
        let ways = names.map(|name| (name, BTreeSet::from([WAY_TLS])));
        let owner = format!("ingress/{}", ingress.name);
        record_users(
            &mut users,
            &ingress.namespace,
            &owner,
            target.as_ref(),
            ways,
        );
    }
    users
}

/// The users of `secret`, sorted by owner: the pods and ingresses of `users`, plus the account a
/// service-account token belongs to.
pub(crate) fn secret_user_list(secret: &SecretSummary, users: &SecretUsers) -> Vec<UsedBy> {
    let mut list: Vec<UsedBy> = users_of(users, &secret.namespace, &secret.name)
        .cloned()
        .collect();
    if let SecretDetails::ServiceAccountToken {
        account: Some(account),
    } = &secret.details
    {
        list.push(UsedBy {
            owner: format!("serviceaccount/{account}"),
            target: ResourceKey::of_object("ServiceAccount", Some(&secret.namespace), account),
            ways: BTreeSet::from([WAY_TOKEN]),
        });
    }
    list.sort_by(|left, right| left.owner.cmp(&right.owner));
    list
}

/// Whether "unused" may be said of `secret` at all: only a type nothing else references, and
/// never a secret another object owns.
pub(crate) fn may_be_unused(secret: &SecretSummary) -> bool {
    !secret.is_owned && UNUSED_CANDIDATE_TYPES.contains(&secret.secret_type.as_str())
}

/// The Used by cells of the Secrets rows. Nothing shows until the pods have loaded. `unused`
/// also needs the ingresses: a secret an ingress names is in use.
fn join_secrets(rows: &mut [KindRow], inputs: &JoinInputs) {
    let ingresses = inputs
        .companion
        .and_then(CompanionLists::ingresses)
        .and_then(LiveList::ready_items);
    let users = inputs
        .pods
        .ready_items()
        .map(|pods| secret_users(pods, ingresses.unwrap_or(&[])));
    for row in rows {
        let KindObject::Secret(secret) = &row.object else {
            continue;
        };
        let cell = users.as_ref().map_or(KindCell::Absent, |users| {
            secret_used_by_cell(secret, users, ingresses.is_some())
        });
        if let Some(slot) = row.cells.get_mut(SECRET_USED_BY) {
            *slot = cell;
        }
    }
}

/// The first owner, plus ` +{n}`; `unused` for an eligible secret nobody uses once every list
/// that could name a user has loaded.
fn secret_used_by_cell(
    secret: &SecretSummary,
    users: &SecretUsers,
    are_ingresses_loaded: bool,
) -> KindCell {
    let list = secret_user_list(secret, users);
    if list.is_empty() && are_ingresses_loaded && may_be_unused(secret) {
        return KindCell::Toned(StatusLabel {
            text: "unused".into(),
            tone: StatusTone::Done,
        });
    }
    used_by_cell(list.iter())
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

// ---- NetworkPolicies ----

fn join_network_policies(rows: &mut [KindRow], inputs: &JoinInputs) {
    let pods = inputs.pods.ready_items().map(pods_by_namespace);
    for row in rows {
        let KindObject::NetworkPolicy(policy) = &row.object else {
            continue;
        };
        // Until the pods load, the builder's status and an empty cell stand.
        let affected = pods.as_ref().map(|pods| {
            pods.get(policy.namespace.as_str())
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .filter(|pod| policy.pod_selector.matches(&pod.labels))
                .count()
        });
        let (status, cell) = match affected {
            None => (network_policy_status(policy), KindCell::Absent),
            Some(count) => affects_state(count),
        };
        row.status = status;
        if let Some(slot) = row.cells.get_mut(NETWORK_POLICY_AFFECTS) {
            *slot = cell;
        }
    }
}

/// The status and the Affects cell of a policy that selects `count` pods.
fn affects_state(count: usize) -> (StatusLabel, KindCell) {
    let noun = if count == 1 { "pod" } else { "pods" };
    let text = format!("{count} {noun}");
    if count == 0 {
        let status = StatusLabel {
            text: "Selects no pods".into(),
            tone: StatusTone::Warn,
        };
        let cell = KindCell::Quantity {
            text: text.into(),
            value: 0,
            tone: Some(StatusTone::Warn),
        };
        return (status, cell);
    }
    let status = StatusLabel {
        text: text.clone().into(),
        tone: StatusTone::Ok,
    };
    let cell = KindCell::Quantity {
        text: text.into(),
        value: u64::try_from(count).unwrap_or(u64::MAX),
        tone: None,
    };
    (status, cell)
}

// ---- PersistentVolumeClaims ----

/// The used and capacity bytes of a kubelet sample and their ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct UsageSample {
    pub(crate) used: ByteAmount,
    pub(crate) capacity: ByteAmount,
    pub(crate) ratio: f64,
}

/// `None` without both numbers or with a zero capacity.
pub(crate) fn claim_sample(usage: &PvcUsage) -> Option<UsageSample> {
    let (used, capacity) = (usage.used?, usage.capacity?);
    if capacity.bytes() == 0 {
        return None;
    }
    Some(UsageSample {
        used,
        capacity,
        ratio: used.bytes() as f64 / capacity.bytes() as f64,
    })
}

/// Whether the kubelet reports a filesystem larger than the claim. A hostPath or local volume
/// shares the node's disk, so the kubelet numbers describe the node, not the claim: a real
/// volume reports at most its own size.
pub(crate) fn is_shared_filesystem(usage: &PvcUsage, claim_capacity: Option<&str>) -> bool {
    let (Some(reported), Some(claimed)) =
        (usage.capacity, claim_capacity.and_then(ByteAmount::parse))
    else {
        return false;
    };
    reported > claimed
}

fn join_claims(rows: &mut [KindRow], inputs: &JoinInputs) {
    for row in rows {
        let KindObject::PersistentVolumeClaim(claim) = &row.object else {
            continue;
        };
        // Each pass starts from the builder's status, so a claim that emptied again recovers it.
        let mut status = claim_status(claim);
        let usage = inputs
            .kubelet
            .and_then(|history| history.pvc_usage(&claim.namespace, &claim.name))
            .filter(|usage| !is_shared_filesystem(usage, claim.capacity.as_deref()));
        let cell = match usage.and_then(claim_sample) {
            None => KindCell::Absent,
            Some(UsageSample { ratio, .. }) => {
                let tone = usage_tone(ratio);
                let is_idle = claim.phase == "Bound" && status.tone == StatusTone::Ok;
                if let (true, Some(tone)) = (is_idle, tone) {
                    status = StatusLabel {
                        text: format!("{} used", format_percent(ratio)).into(),
                        tone,
                    };
                }
                KindCell::Quantity {
                    text: format_percent(ratio).into(),
                    // Permille keeps 83.4 % apart from 83.0 % when sorting.
                    value: (ratio * 1000.0).round() as u64,
                    tone,
                }
            }
        };
        row.status = status;
        if let Some(slot) = row.cells.get_mut(CLAIM_USED) {
            *slot = cell;
        }
    }
}

// ---- StorageClasses ----

/// The PVs cell: how many persistent volumes use each class, from the PV companion. `Absent`
/// until the companion has loaded, and when it is denied.
fn join_classes(rows: &mut [KindRow], inputs: &JoinInputs) {
    let volumes = inputs
        .companion
        .and_then(CompanionLists::persistent_volumes)
        .and_then(LiveList::ready_items);
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for class in volumes
        .into_iter()
        .flatten()
        .filter_map(|volume| volume.storage_class.as_deref())
    {
        *counts.entry(class).or_default() += 1;
    }
    for row in rows {
        let cell = match volumes {
            None => KindCell::Absent,
            Some(_) => {
                let count = counts.get(row.name.as_str()).copied().unwrap_or(0);
                KindCell::Quantity {
                    text: count.to_string().into(),
                    value: u64::try_from(count).unwrap_or(u64::MAX),
                    tone: None,
                }
            }
        };
        if let Some(slot) = row.cells.get_mut(CLASS_VOLUMES) {
            *slot = cell;
        }
    }
}

// ---- Roles and ClusterRoles ----

/// The Bindings cell of each role: how many bindings name it, from the Bindings companion.
/// `Absent` until the needed lists have loaded, and when one is denied. Warn when a role that
/// grants everything is bound at all.
fn join_roles(rows: &mut [KindRow], inputs: &JoinInputs, column: usize) {
    let index = ready_binding_lists(inputs.companion).map(|lists| BindingIndex::build(&lists));
    for row in rows {
        let KindObject::Role(role) = &row.object else {
            continue;
        };
        let cell = index.as_ref().map_or(KindCell::Absent, |index| {
            let count = index.bindings_of_role(role).len();
            KindCell::Quantity {
                text: count.to_string().into(),
                value: u64::try_from(count).unwrap_or(u64::MAX),
                tone: (role.grants_everything() && count > 0).then_some(StatusTone::Warn),
            }
        });
        if let Some(slot) = row.cells.get_mut(column) {
            *slot = cell;
        }
    }
}

// ---- ServiceAccounts ----

/// The pods per (namespace, service account) in one pass; a pod without an account runs as
/// `default`.
fn pods_by_account(pods: &[PodSummary]) -> HashMap<(&str, &str), usize> {
    let mut counts: HashMap<(&str, &str), usize> = HashMap::new();
    for pod in pods {
        *counts
            .entry((pod.namespace.as_str(), pod_account(pod)))
            .or_default() += 1;
    }
    counts
}

/// The Bound roles and Used by cells, and the status, of each service account. Each part waits for
/// its own list: roles for the Bindings companion, pods for the pods list.
fn join_service_accounts(rows: &mut [KindRow], inputs: &JoinInputs) {
    let index = ready_binding_lists(inputs.companion).map(|lists| BindingIndex::build(&lists));
    let pods = inputs.pods.ready_items().map(pods_by_account);
    for row in rows {
        let KindObject::ServiceAccount(account) = &row.object else {
            continue;
        };
        let roles = index
            .as_ref()
            .map(|index| index.roles_held(&account.namespace, &account.name));
        let used_by = pods.as_ref().map(|pods| {
            pods.get(&(account.namespace.as_str(), account.name.as_str()))
                .copied()
                .unwrap_or(0)
        });
        row.status = account_status(roles.as_deref(), used_by);
        let cells = [
            (ACCOUNT_BOUND_ROLES, bound_roles_cell(roles.as_deref())),
            (ACCOUNT_USED_BY, used_by_pods_cell(used_by)),
        ];
        for (column, cell) in cells {
            if let Some(slot) = row.cells.get_mut(column) {
                *slot = cell;
            }
        }
    }
}

fn is_bound_to_cluster_admin(roles: &[&BoundRole]) -> bool {
    roles.iter().any(|bound| is_cluster_admin(&bound.role))
}

/// The status of a service account: full access first, then whether pods use it. Without the lists
/// it stays at the builder status.
fn account_status(roles: Option<&[&BoundRole]>, used_by: Option<usize>) -> StatusLabel {
    if roles.is_some_and(is_bound_to_cluster_admin) {
        return StatusLabel {
            text: "Cluster admin".into(),
            tone: StatusTone::Warn,
        };
    }
    match used_by {
        None => service_account_status(),
        Some(0) => StatusLabel {
            text: "No pods".into(),
            tone: StatusTone::Done,
        },
        Some(count) => StatusLabel {
            text: pods_text(count).into(),
            tone: StatusTone::Ok,
        },
    }
}

fn pods_text(count: usize) -> String {
    format!("{count} {}", if count == 1 { "pod" } else { "pods" })
}

/// The distinct roles joined with `, `; Warn when one is cluster-admin, a muted dash for none.
fn bound_roles_cell(roles: Option<&[&BoundRole]>) -> KindCell {
    let Some(roles) = roles else {
        return KindCell::Absent;
    };
    let mut texts: Vec<String> = roles.iter().map(|bound| role_text(&bound.role)).collect();
    texts.dedup();
    if texts.is_empty() {
        return KindCell::Toned(StatusLabel {
            text: "—".into(),
            tone: StatusTone::Done,
        });
    }
    let text = texts.join(", ");
    if is_bound_to_cluster_admin(roles) {
        return KindCell::Toned(StatusLabel {
            text: text.into(),
            tone: StatusTone::Warn,
        });
    }
    KindCell::Text(text.into())
}

fn used_by_pods_cell(used_by: Option<usize>) -> KindCell {
    let Some(count) = used_by else {
        return KindCell::Absent;
    };
    KindCell::Quantity {
        text: pods_text(count).into(),
        value: u64::try_from(count).unwrap_or(u64::MAX),
        tone: None,
    }
}

#[cfg(test)]
#[path = "kind_join_tests.rs"]
mod kind_join_tests;
