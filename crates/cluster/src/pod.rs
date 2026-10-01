use std::fmt;

use futures::Stream;
use k8s_openapi::api::core::v1::{
    Container, ContainerState as ApiContainerState, ContainerStateTerminated,
    ContainerStatus as ApiContainerStatus, Pod,
};

use crate::connection::{ClusterConnection, ClusterError};
use crate::namespace::NamespaceScope;
use crate::pod_status::{PodStatus, StatusReason, is_sidecar, non_negative, pod_display};
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{ControllerRef, controller_ref, non_empty};

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodCondition {
    /// The condition type, for example `Ready`.
    pub name: String,
    /// `status == "True"`; `False` and `Unknown` are both `false`.
    pub is_true: bool,
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
        let pods = self
            .list_all(self.scoped_api(scope), "listing pods")
            .await?;
        let mut summaries: Vec<_> = pods.iter().map(pod_summary).collect();
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
        summary_watch(self, self.scoped_api(scope), "watching pods", pod_summary)
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
            })
            .collect(),
        containers: container_summaries(pod),
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

    let init_containers = spec.init_containers.iter().flatten().map(|container| {
        let kind = if is_sidecar(container) {
            ContainerKind::Sidecar
        } else {
            ContainerKind::Init
        };
        container_summary(container, kind, init_statuses)
    });
    let main_containers = spec
        .containers
        .iter()
        .map(|container| container_summary(container, ContainerKind::Main, main_statuses));
    init_containers.chain(main_containers).collect()
}

fn container_summary(
    container: &Container,
    kind: ContainerKind,
    statuses: &[ApiContainerStatus],
) -> ContainerSummary {
    let Some(status) = statuses.iter().find(|status| status.name == container.name) else {
        return ContainerSummary {
            name: container.name.clone(),
            image: image(container),
            kind,
            state: ContainerState::NotReported,
            is_ready: false,
            restart_count: 0,
            last_termination: None,
        };
    };
    ContainerSummary {
        name: container.name.clone(),
        image: image(container),
        kind,
        state: status
            .state
            .as_ref()
            .map_or(ContainerState::NotReported, container_state),
        is_ready: status.ready,
        restart_count: non_negative(status.restart_count),
        last_termination: status
            .last_state
            .as_ref()
            .and_then(|state| state.terminated.as_ref())
            .map(termination),
    }
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
