use std::fmt;

use k8s_openapi::api::core::v1::{Container, Pod, PodStatus as ApiPodStatus};

use crate::pod::ReadyCount;
use crate::workload::non_empty;

/// The kubectl `STATUS` column of a pod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PodStatus {
    /// The phase, the pod reason, or the first failing main container's reason.
    Reason(StatusReason),
    /// `Init:...`
    Init(InitStatus),
    /// A completed container plus a running one while the pod is not Ready.
    NotReady,
    /// Deletion was requested and the phase is not terminal.
    Terminating,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InitStatus {
    /// `Init:<first_incomplete>/<total>`
    Progress { first_incomplete: u32, total: u32 },
    /// `Init:<reason>`
    Reason(StatusReason),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusReason {
    Running,
    Pending,
    Succeeded,
    Failed,
    Unknown,
    Completed,
    ContainerCreating,
    PodInitializing,
    CrashLoopBackOff,
    ImagePullBackOff,
    ErrImagePull,
    CreateContainerConfigError,
    /// API text `OOMKilled`.
    OomKilled,
    Error,
    ContainerCannotRun,
    Evicted,
    SchedulingGated,
    /// `Signal:<n>`
    Signal(i32),
    /// `ExitCode:<n>`
    ExitCode(i32),
    /// Any other text, verbatim.
    Other(String),
}

impl StatusReason {
    /// Exact, case-sensitive match; unknown text becomes `Other`. Never yields
    /// `Signal` or `ExitCode`, which the status algorithm builds itself.
    pub(crate) fn from_api(text: &str) -> Self {
        match text {
            "Running" => Self::Running,
            "Pending" => Self::Pending,
            "Succeeded" => Self::Succeeded,
            "Failed" => Self::Failed,
            "Unknown" => Self::Unknown,
            "Completed" => Self::Completed,
            "ContainerCreating" => Self::ContainerCreating,
            "PodInitializing" => Self::PodInitializing,
            "CrashLoopBackOff" => Self::CrashLoopBackOff,
            "ImagePullBackOff" => Self::ImagePullBackOff,
            "ErrImagePull" => Self::ErrImagePull,
            "CreateContainerConfigError" => Self::CreateContainerConfigError,
            "OOMKilled" => Self::OomKilled,
            "Error" => Self::Error,
            "ContainerCannotRun" => Self::ContainerCannotRun,
            "Evicted" => Self::Evicted,
            "SchedulingGated" => Self::SchedulingGated,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl fmt::Display for StatusReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Running => "Running",
            Self::Pending => "Pending",
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Unknown => "Unknown",
            Self::Completed => "Completed",
            Self::ContainerCreating => "ContainerCreating",
            Self::PodInitializing => "PodInitializing",
            Self::CrashLoopBackOff => "CrashLoopBackOff",
            Self::ImagePullBackOff => "ImagePullBackOff",
            Self::ErrImagePull => "ErrImagePull",
            Self::CreateContainerConfigError => "CreateContainerConfigError",
            Self::OomKilled => "OOMKilled",
            Self::Error => "Error",
            Self::ContainerCannotRun => "ContainerCannotRun",
            Self::Evicted => "Evicted",
            Self::SchedulingGated => "SchedulingGated",
            Self::Signal(signal) => return write!(formatter, "Signal:{signal}"),
            Self::ExitCode(code) => return write!(formatter, "ExitCode:{code}"),
            Self::Other(text) => text,
        };
        formatter.write_str(text)
    }
}

impl fmt::Display for InitStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Progress {
                first_incomplete,
                total,
            } => write!(formatter, "Init:{first_incomplete}/{total}"),
            Self::Reason(reason) => write!(formatter, "Init:{reason}"),
        }
    }
}

impl fmt::Display for PodStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reason(reason) => reason.fmt(formatter),
            Self::Init(init) => init.fmt(formatter),
            Self::NotReady => formatter.write_str("NotReady"),
            Self::Terminating => formatter.write_str("Terminating"),
        }
    }
}

/// The three values kubectl's `printPod` derives in one pass.
pub(crate) struct PodDisplay {
    pub(crate) status: PodStatus,
    pub(crate) ready: ReadyCount,
    pub(crate) restarts: u32,
}

/// An API count as `u32`; a negative value is clamped to zero.
pub(crate) fn non_negative(count: i32) -> u32 {
    u32::try_from(count).unwrap_or(0)
}

/// An init container with `restartPolicy: Always` is a sidecar.
pub(crate) fn is_sidecar(container: &Container) -> bool {
    container.restart_policy.as_deref() == Some("Always")
}

/// Ports kubectl 1.32 `printPod`. One deliberate divergence: a missing phase is
/// `Unknown` where kubectl prints an empty string.
pub(crate) fn pod_display(pod: &Pod) -> PodDisplay {
    let empty_status = ApiPodStatus::default();
    let status = pod.status.as_ref().unwrap_or(&empty_status);
    let (containers, init_containers) = match &pod.spec {
        Some(spec) => (
            spec.containers.as_slice(),
            spec.init_containers.as_deref().unwrap_or_default(),
        ),
        None => (&[][..], &[][..]),
    };

    let phase = status.phase.as_deref().unwrap_or("Unknown");
    let mut display_status = PodStatus::Reason(StatusReason::from_api(
        non_empty(status.reason.as_deref())
            .as_deref()
            .unwrap_or(phase),
    ));
    if has_condition_reason(status, "PodScheduled", "SchedulingGated") {
        display_status = PodStatus::Reason(StatusReason::SchedulingGated);
    }

    let total = containers.len()
        + init_containers
            .iter()
            .filter(|container| is_sidecar(container))
            .count();
    let mut ready = 0u32;
    let mut restarts = 0u32;
    let mut sidecar_restarts = 0u32;
    let mut is_initializing = false;

    for (index, container_status) in status.init_container_statuses.iter().flatten().enumerate() {
        let restart_count = non_negative(container_status.restart_count);
        restarts = restarts.saturating_add(restart_count);
        let is_sidecar_status = init_containers
            .iter()
            .any(|container| container.name == container_status.name && is_sidecar(container));
        if is_sidecar_status {
            sidecar_restarts = sidecar_restarts.saturating_add(restart_count);
        }

        let state = container_status.state.as_ref();
        let terminated = state.and_then(|state| state.terminated.as_ref());
        if terminated.is_some_and(|terminated| terminated.exit_code == 0) {
            continue;
        }
        if is_sidecar_status && container_status.started == Some(true) {
            if container_status.ready {
                ready += 1;
            }
            continue;
        }

        let waiting_reason = state
            .and_then(|state| state.waiting.as_ref())
            .and_then(|waiting| non_empty(waiting.reason.as_deref()))
            .filter(|reason| reason != "PodInitializing");
        display_status = if let Some(terminated) = terminated {
            PodStatus::Init(InitStatus::Reason(
                match non_empty(terminated.reason.as_deref()).as_deref() {
                    Some(reason) => StatusReason::from_api(reason),
                    None => exit_reason(terminated.signal, terminated.exit_code),
                },
            ))
        } else if let Some(reason) = waiting_reason {
            PodStatus::Init(InitStatus::Reason(StatusReason::from_api(&reason)))
        } else {
            PodStatus::Init(InitStatus::Progress {
                first_incomplete: u32::try_from(index).unwrap_or(u32::MAX),
                total: u32::try_from(init_containers.len()).unwrap_or(u32::MAX),
            })
        };
        is_initializing = true;
        break;
    }

    if !is_initializing || has_condition_status(status, "Initialized", "True") {
        restarts = sidecar_restarts;
        let mut has_running = false;
        // Reversed so the first container in spec order supplies the reason.
        for container_status in status.container_statuses.iter().flatten().rev() {
            restarts = restarts.saturating_add(non_negative(container_status.restart_count));
            let state = container_status.state.as_ref();
            let waiting_reason = state
                .and_then(|state| state.waiting.as_ref())
                .and_then(|waiting| non_empty(waiting.reason.as_deref()));
            let terminated = state.and_then(|state| state.terminated.as_ref());
            if let Some(reason) = waiting_reason {
                display_status = PodStatus::Reason(StatusReason::from_api(&reason));
            } else if let Some(terminated) = terminated {
                display_status =
                    PodStatus::Reason(match non_empty(terminated.reason.as_deref()).as_deref() {
                        Some(reason) => StatusReason::from_api(reason),
                        None => exit_reason(terminated.signal, terminated.exit_code),
                    });
            } else if container_status.ready && state.is_some_and(|state| state.running.is_some()) {
                has_running = true;
                ready += 1;
            }
        }
        if has_running && display_status == PodStatus::Reason(StatusReason::Completed) {
            display_status = if has_condition_status(status, "Ready", "True") {
                PodStatus::Reason(StatusReason::Running)
            } else {
                PodStatus::NotReady
            };
        }
    }

    if pod.metadata.deletion_timestamp.is_some() {
        if status.reason.as_deref() == Some("NodeLost") {
            display_status = PodStatus::Reason(StatusReason::Unknown);
        } else if !matches!(phase, "Failed" | "Succeeded") {
            display_status = PodStatus::Terminating;
        }
    }

    PodDisplay {
        status: display_status,
        ready: ReadyCount {
            ready,
            total: u32::try_from(total).unwrap_or(u32::MAX),
        },
        restarts,
    }
}

/// The reason for a terminated container that reports none: its signal, else its exit code.
fn exit_reason(signal: Option<i32>, exit_code: i32) -> StatusReason {
    match signal {
        Some(signal) if signal != 0 => StatusReason::Signal(signal),
        _ => StatusReason::ExitCode(exit_code),
    }
}

fn has_condition_reason(status: &ApiPodStatus, type_: &str, reason: &str) -> bool {
    status
        .conditions
        .iter()
        .flatten()
        .any(|condition| condition.type_ == type_ && condition.reason.as_deref() == Some(reason))
}

fn has_condition_status(status: &ApiPodStatus, type_: &str, value: &str) -> bool {
    status
        .conditions
        .iter()
        .flatten()
        .any(|condition| condition.type_ == type_ && condition.status == value)
}

#[cfg(test)]
#[path = "pod_status_tests.rs"]
mod pod_status_tests;
