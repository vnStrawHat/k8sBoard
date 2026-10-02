use std::fmt;

use futures::Stream;
use futures::future::try_join_all;
use k8s_openapi::api::core::v1::{
    Container, ContainerState as ApiContainerState, ContainerStateTerminated,
    ContainerStatus as ApiContainerStatus, Pod, Volume,
};

use crate::connection::{ClusterConnection, ClusterError};
use crate::container_spec::{
    ContainerProbes, ContainerResource, EnvEntry, EnvFromEntry, MountEntry, container_probes,
    container_resources, env_entries, env_from_entries, image_digest, mount_entries,
};
use crate::event::optional_message;
use crate::namespace::NamespaceScope;
use crate::pod_status::{PodStatus, StatusReason, is_sidecar, non_negative, pod_display};
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{
    ContainerPort, ControllerRef, container_ports, controller_ref, label_terms, non_empty,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodSummary {
    pub namespace: String,
    pub name: String,
    pub status: PodStatus,
    pub ready: ReadyCount,
    pub restarts: u32,
    pub node_name: Option<String>,
    pub created_at: Option<jiff::Timestamp>,
    /// `status.podIP`; empty is `None`.
    pub pod_ip: Option<String>,
    /// `Guaranteed`, `Burstable`, or `BestEffort`. Display only, so it stays text.
    pub qos_class: Option<String>,
    /// `spec.serviceAccountName`; empty is `None`.
    pub service_account: Option<String>,
    /// The owner reference with `controller == true`.
    pub controller: Option<ControllerRef>,
    /// `status.conditions` in API order.
    pub conditions: Vec<PodCondition>,
    /// Init and sidecar containers in spec order, then main containers.
    pub containers: Vec<ContainerSummary>,
    /// `status.message`, kept only when the phase is `Failed` or the reason is `Evicted`.
    /// URL userinfo hidden, then cut like event messages.
    pub status_message: Option<String>,
    /// `key=value` terms in key order. Labels only: annotations are never read.
    pub labels: Vec<String>,
    /// The pod uses the node's network namespace; its network stats are the node's.
    pub host_network: bool,
    /// `spec.imagePullSecrets[].name`; empty names are dropped.
    pub image_pull_secrets: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodCondition {
    /// The condition type, for example `Ready`.
    pub name: String,
    /// `status == "True"`; `False` and `Unknown` are both `false`.
    pub is_true: bool,
    /// An empty reason is `None`.
    pub reason: Option<String>,
    /// URL userinfo hidden, then cut like event messages.
    pub message: Option<String>,
    /// `lastTransitionTime`.
    pub changed_at: Option<jiff::Timestamp>,
}

/// Ready containers over total (main plus sidecar), like kubectl's `READY` column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadyCount {
    pub ready: u32,
    pub total: u32,
}

impl fmt::Display for ReadyCount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.ready, self.total)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerSummary {
    pub name: String,
    /// The image as written in the spec; digests are not resolved.
    pub image: String,
    pub kind: ContainerKind,
    pub state: ContainerState,
    pub is_ready: bool,
    pub restart_count: u32,
    pub last_termination: Option<Termination>,
    /// `status.imageID` after its last `@`, or the whole id when it starts with `sha256:`.
    pub image_digest: Option<String>,
    /// `imagePullPolicy` as written; the API defaults it.
    pub pull_policy: Option<String>,
    /// `status.started`; `None` when unreported.
    pub is_started: Option<bool>,
    /// In spec order.
    pub ports: Vec<ContainerPort>,
    /// `cpu`, `memory`, `ephemeral-storage`, then the rest by name.
    pub resources: Vec<ContainerResource>,
    pub probes: ContainerProbes,
    /// Names and sources only; values are never read. In spec order.
    pub env: Vec<EnvEntry>,
    pub env_from: Vec<EnvFromEntry>,
    pub mounts: Vec<MountEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerKind {
    Init,
    Sidecar,
    Main,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerState {
    Waiting {
        reason: Option<StatusReason>,
        /// URL userinfo hidden, then cut like event messages.
        message: Option<String>,
    },
    Running {
        started_at: Option<jiff::Timestamp>,
    },
    Terminated(Termination),
    /// The kubelet has not reported a status yet.
    NotReported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Termination {
    pub reason: Option<StatusReason>,
    pub exit_code: i32,
    /// `None` when absent or zero.
    pub signal: Option<i32>,
    pub started_at: Option<jiff::Timestamp>,
    pub finished_at: Option<jiff::Timestamp>,
}

impl ClusterConnection {
    /// Lists pods in `scope`, ordered by (namespace, name).
    pub async fn list_pods(&self, scope: NamespaceScope) -> Result<Vec<PodSummary>, ClusterError> {
        let is_several = scope.namespaces().len() > 1;
        let lists = self
            .scoped_apis(&scope)
            .into_iter()
            .map(|(namespace, api)| async move {
                let pods = self.list_all(api, "listing pods").await;
                pods.map_err(|source| match namespace {
                    Some(namespace) if is_several => ClusterError::Namespace {
                        namespace,
                        source: Box::new(source),
                    },
                    _ => source,
                })
            });
        let mut summaries: Vec<PodSummary> = try_join_all(lists)
            .await?
            .iter()
            .flatten()
            .map(pod_summary)
            .collect();
        summaries.sort_by(|left, right| {
            (&left.namespace, &left.name).cmp(&(&right.namespace, &right.name))
        });
        Ok(summaries)
    }

    /// Watches pods in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_pods(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<PodSummary>> + Send + 'static {
        summary_watch(self, self.scoped_apis(&scope), "watching pods", pod_summary)
    }
}

pub(crate) fn pod_summary(pod: &Pod) -> PodSummary {
    let display = pod_display(pod);
    PodSummary {
        namespace: pod.metadata.namespace.clone().unwrap_or_default(),
        name: pod.metadata.name.clone().unwrap_or_default(),
        status: display.status,
        ready: display.ready,
        restarts: display.restarts,
        node_name: pod.spec.as_ref().and_then(|spec| spec.node_name.clone()),
        created_at: pod.metadata.creation_timestamp.as_ref().map(|time| time.0),
        pod_ip: non_empty(
            pod.status
                .as_ref()
                .and_then(|status| status.pod_ip.as_deref()),
        ),
        qos_class: non_empty(
            pod.status
                .as_ref()
                .and_then(|status| status.qos_class.as_deref()),
        ),
        service_account: non_empty(
            pod.spec
                .as_ref()
                .and_then(|spec| spec.service_account_name.as_deref()),
        ),
        controller: controller_ref(&pod.metadata),
        conditions: pod
            .status
            .iter()
            .flat_map(|status| status.conditions.iter().flatten())
            .map(|condition| PodCondition {
                name: condition.type_.clone(),
                is_true: condition.status == "True",
                reason: non_empty(condition.reason.as_deref()),
                message: optional_message(condition.message.as_deref()),
                changed_at: condition.last_transition_time.as_ref().map(|time| time.0),
            })
            .collect(),
        containers: container_summaries(pod),
        status_message: status_message(pod),
        labels: label_terms(&pod.metadata),
        host_network: pod
            .spec
            .as_ref()
            .and_then(|spec| spec.host_network)
            .unwrap_or(false),
        image_pull_secrets: pod
            .spec
            .iter()
            .flat_map(|spec| spec.image_pull_secrets.iter().flatten())
            .filter_map(|reference| non_empty(Some(reference.name.as_str())))
            .collect(),
    }
}

fn image(container: &Container) -> String {
    container.image.clone().unwrap_or_default()
}

fn container_summaries(pod: &Pod) -> Vec<ContainerSummary> {
    let Some(spec) = &pod.spec else {
        return Vec::new();
    };
    let status = pod.status.as_ref();
    let init_statuses = status
        .and_then(|status| status.init_container_statuses.as_deref())
        .unwrap_or_default();
    let main_statuses = status
        .and_then(|status| status.container_statuses.as_deref())
        .unwrap_or_default();
    let volumes = spec.volumes.as_deref().unwrap_or_default();

    let init_containers = spec.init_containers.iter().flatten().map(|container| {
        let kind = if is_sidecar(container) {
            ContainerKind::Sidecar
        } else {
            ContainerKind::Init
        };
        container_summary(container, kind, init_statuses, volumes)
    });
    let main_containers = spec
        .containers
        .iter()
        .map(|container| container_summary(container, ContainerKind::Main, main_statuses, volumes));
    init_containers.chain(main_containers).collect()
}

fn container_summary(
    container: &Container,
    kind: ContainerKind,
    statuses: &[ApiContainerStatus],
    volumes: &[Volume],
) -> ContainerSummary {
    let status = statuses.iter().find(|status| status.name == container.name);
    ContainerSummary {
        name: container.name.clone(),
        image: image(container),
        kind,
        state: status
            .and_then(|status| status.state.as_ref())
            .map_or(ContainerState::NotReported, container_state),
        is_ready: status.is_some_and(|status| status.ready),
        restart_count: status.map_or(0, |status| non_negative(status.restart_count)),
        last_termination: status
            .and_then(|status| status.last_state.as_ref())
            .and_then(|state| state.terminated.as_ref())
            .map(termination),
        image_digest: status.and_then(|status| image_digest(&status.image_id)),
        pull_policy: non_empty(container.image_pull_policy.as_deref()),
        is_started: status.and_then(|status| status.started),
        ports: container_ports(container),
        resources: container_resources(container),
        probes: container_probes(container),
        env: env_entries(container),
        env_from: env_from_entries(container),
        mounts: mount_entries(container, volumes),
    }
}

/// Kept only for pods that are over (`Failed` or evicted): on a live pod the message is
/// transient and would read as a cause.
fn status_message(pod: &Pod) -> Option<String> {
    let status = pod.status.as_ref()?;
    let is_over =
        status.phase.as_deref() == Some("Failed") || status.reason.as_deref() == Some("Evicted");
    if !is_over {
        return None;
    }
    optional_message(status.message.as_deref())
}

/// `terminated` wins over `running`, which wins over `waiting`.
fn container_state(state: &ApiContainerState) -> ContainerState {
    if let Some(terminated) = &state.terminated {
        return ContainerState::Terminated(termination(terminated));
    }
    if let Some(running) = &state.running {
        return ContainerState::Running {
            started_at: running.started_at.as_ref().map(|time| time.0),
        };
    }
    if let Some(waiting) = &state.waiting {
        return ContainerState::Waiting {
            reason: status_reason(waiting.reason.as_deref()),
            message: optional_message(waiting.message.as_deref()),
        };
    }
    ContainerState::NotReported
}

fn termination(terminated: &ContainerStateTerminated) -> Termination {
    Termination {
        reason: status_reason(terminated.reason.as_deref()),
        exit_code: terminated.exit_code,
        signal: terminated.signal.filter(|signal| *signal != 0),
        started_at: terminated.started_at.as_ref().map(|time| time.0),
        finished_at: terminated.finished_at.as_ref().map(|time| time.0),
    }
}

/// An empty reason is `None`.
fn status_reason(reason: Option<&str>) -> Option<StatusReason> {
    reason
        .filter(|reason| !reason.is_empty())
        .map(StatusReason::from_api)
}

#[cfg(test)]
#[path = "pod_tests.rs"]
mod pod_tests;
