//! What a log tab streams: one pod, or every pod of a workload. Pure: no session, no GPUI.

use std::fmt;

use cluster::{ContainerSummary, PodSummary};
use gpui_kit::SharedString;

use crate::kind_row::{DAEMON_SET_KIND, JOB_KIND, PodOwner, REPLICA_SET_KIND, STATEFUL_SET_KIND};
use crate::pod_drawer::default_container;

#[derive(Clone)]
pub(crate) enum LogTarget {
    Pod(PodTarget),
    Workload(WorkloadTarget),
}

/// Whether a target names its container or takes the default one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainerChoice {
    Default,
    Explicit,
}

/// One pod, with the containers the picker offers.
#[derive(Clone)]
pub(crate) struct PodTarget {
    pub(crate) namespace: String,
    pub(crate) pod: String,
    pub(crate) containers: Vec<ContainerSummary>,
    pub(crate) initial_container: String,
    pub(crate) choice: ContainerChoice,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkloadTarget {
    pub(crate) owner: PodOwner,
    /// `deploy/api`, the tab title.
    pub(crate) label: String,
}

impl LogTarget {
    /// `None` when the pod has no containers to read.
    pub(crate) fn of_pod(pod: &PodSummary) -> Option<Self> {
        let index = default_container(&pod.containers)?;
        let initial_container = pod.containers.get(index)?.name.clone();
        Some(Self::Pod(PodTarget {
            namespace: pod.namespace.clone(),
            pod: pod.name.clone(),
            containers: pod.containers.clone(),
            initial_container,
            choice: ContainerChoice::Default,
        }))
    }

    /// A pod tab on `container`; `None` when the pod has no container of that name.
    pub(crate) fn of_container(pod: &PodSummary, container: &str) -> Option<Self> {
        let Self::Pod(mut target) = Self::of_pod(pod)? else {
            return None;
        };
        pod.containers
            .iter()
            .find(|candidate| candidate.name == container)?;
        target.initial_container = container.to_owned();
        target.choice = ContainerChoice::Explicit;
        Some(Self::Pod(target))
    }

    /// `None` for a node: its pods are not one workload.
    pub(crate) fn of_workload(owner: PodOwner) -> Option<Self> {
        let label = workload_label(&owner)?;
        Some(Self::Workload(WorkloadTarget { owner, label }))
    }
}

impl PodTarget {
    pub(crate) fn is_same(&self, other: &Self) -> bool {
        self.namespace == other.namespace && self.pod == other.pod
    }
}

impl WorkloadTarget {
    pub(crate) fn is_same(&self, other: &Self) -> bool {
        self.owner == other.owner
    }
}

/// `deploy/api`, `sts/db`, `ds/agent`, `rs/api-7d9f8c`, `job/migrate`; `None` for a node.
pub(crate) fn workload_label(owner: &PodOwner) -> Option<String> {
    let (short_kind, name) = match owner {
        PodOwner::Deployment { name, .. } => ("deploy", name),
        PodOwner::Controller { kind, name, .. } => {
            let short_kind = match *kind {
                STATEFUL_SET_KIND => "sts",
                DAEMON_SET_KIND => "ds",
                REPLICA_SET_KIND => "rs",
                JOB_KIND => "job",
                _ => return None,
            };
            (short_kind, name)
        }
        PodOwner::Node { .. } => return None,
    };
    Some(format!("{short_kind}/{name}"))
}

/// Why "Logs of selected" cannot open a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NoLogTarget {
    NotConnected,
    NotLoggable,
    /// The access review denies `get pods/log`, or has no answer yet; the text says which.
    AccessDenied(SharedString),
}

impl fmt::Display for NoLogTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotConnected => "Not connected",
            Self::NotLoggable => "Select a pod or workload",
            Self::AccessDenied(reason) => reason,
        })
    }
}

/// The ViewLogs access check as a target error: the first thing "Logs of selected" asks, so a
/// denied cluster shows the real reason instead of "Select a pod".
pub(crate) fn check_logs_access(reason: Option<SharedString>) -> Result<(), NoLogTarget> {
    match reason {
        Some(reason) => Err(NoLogTarget::AccessDenied(reason)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use cluster::{ContainerKind, ContainerState, PodStatus, ReadyCount, StatusReason};

    use super::*;

    fn container(name: &str, kind: ContainerKind, is_ready: bool) -> ContainerSummary {
        ContainerSummary {
            name: name.to_owned(),
            image: "img".to_owned(),
            kind,
            state: ContainerState::NotReported,
            is_ready,
            restart_count: 0,
            last_termination: None,
            image_digest: None,
            pull_policy: None,
            is_started: None,
            ports: Vec::new(),
            resources: Vec::new(),
            probes: cluster::ContainerProbes::default(),
            env: Vec::new(),
            env_from: Vec::new(),
            mounts: Vec::new(),
        }
    }

    fn pod(name: &str, containers: Vec<ContainerSummary>) -> PodSummary {
        PodSummary {
            namespace: "ns".to_owned(),
            name: name.to_owned(),
            status: PodStatus::Reason(StatusReason::Running),
            ready: ReadyCount { ready: 0, total: 0 },
            restarts: 0,
            node_name: None,
            created_at: None,
            pod_ip: None,
            qos_class: None,
            service_account: None,
            controller: None,
            conditions: Vec::new(),
            status_message: None,
            labels: Vec::new(),
            host_network: false,
            image_pull_secrets: Vec::new(),
            containers,
        }
    }

    fn controller(kind: &'static str, name: &str) -> PodOwner {
        PodOwner::Controller {
            namespace: "ns".to_owned(),
            kind,
            name: name.to_owned(),
        }
    }

    fn deployment(name: &str) -> PodOwner {
        PodOwner::Deployment {
            namespace: "ns".to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn workload_label_uses_kubectl_short_kinds() {
        assert_eq!(
            workload_label(&deployment("api")).as_deref(),
            Some("deploy/api")
        );
        assert_eq!(
            workload_label(&controller(STATEFUL_SET_KIND, "db")).as_deref(),
            Some("sts/db")
        );
        assert_eq!(
            workload_label(&controller(DAEMON_SET_KIND, "agent")).as_deref(),
            Some("ds/agent")
        );
        assert_eq!(
            workload_label(&controller(REPLICA_SET_KIND, "api-7d9f8c")).as_deref(),
            Some("rs/api-7d9f8c")
        );
        assert_eq!(
            workload_label(&controller(JOB_KIND, "migrate")).as_deref(),
            Some("job/migrate")
        );
        assert_eq!(workload_label(&PodOwner::Node { name: "n1".into() }), None);
    }

    #[test]
    fn log_target_of_pod_picks_default_container() {
        let pod = pod(
            "pod",
            vec![
                container("init", ContainerKind::Init, true),
                container("ready", ContainerKind::Main, true),
                container("waiting", ContainerKind::Main, false),
            ],
        );
        let Some(LogTarget::Pod(target)) = LogTarget::of_pod(&pod) else {
            panic!("a pod target");
        };
        assert_eq!(target.initial_container, "waiting");
        assert_eq!(target.containers.len(), 3);
        assert_eq!(
            (target.namespace.as_str(), target.pod.as_str()),
            ("ns", "pod")
        );
    }

    #[test]
    fn log_target_of_pod_without_containers_is_none() {
        assert!(LogTarget::of_pod(&pod("pod", Vec::new())).is_none());
    }

    #[test]
    fn log_target_of_container_is_explicit() {
        let pod = pod(
            "pod",
            vec![
                container("app", ContainerKind::Main, true),
                container("sidecar", ContainerKind::Main, true),
            ],
        );
        let Some(LogTarget::Pod(target)) = LogTarget::of_container(&pod, "sidecar") else {
            panic!("a pod target");
        };
        assert_eq!(target.initial_container, "sidecar");
        assert_eq!(target.choice, ContainerChoice::Explicit);
        assert!(LogTarget::of_container(&pod, "missing").is_none());
        let Some(LogTarget::Pod(default)) = LogTarget::of_pod(&pod) else {
            panic!("a pod target");
        };
        assert_eq!(default.choice, ContainerChoice::Default);
    }

    #[test]
    fn no_log_target_displays_reason() {
        assert_eq!(NoLogTarget::NotConnected.to_string(), "Not connected");
        assert_eq!(
            NoLogTarget::NotLoggable.to_string(),
            "Select a pod or workload"
        );
    }

    #[test]
    fn denied_logs_access_is_a_target_error_with_the_reason() {
        assert_eq!(check_logs_access(None), Ok(()));
        let denied = check_logs_access(Some("Not permitted: get pods/log".into()));
        assert_eq!(
            denied,
            Err(NoLogTarget::AccessDenied(
                "Not permitted: get pods/log".into()
            ))
        );
        assert_eq!(
            denied.expect_err("denied").to_string(),
            "Not permitted: get pods/log"
        );
    }

    #[test]
    fn log_target_of_workload_rejects_nodes() {
        assert!(LogTarget::of_workload(PodOwner::Node { name: "n1".into() }).is_none());
        let Some(LogTarget::Workload(target)) = LogTarget::of_workload(deployment("api")) else {
            panic!("a workload target");
        };
        assert_eq!(target.label, "deploy/api");
    }

    #[test]
    fn same_target_matches_pods_by_name_and_workloads_by_owner() {
        let main = vec![container("app", ContainerKind::Main, true)];
        let (Some(LogTarget::Pod(a)), Some(LogTarget::Pod(b)), Some(LogTarget::Pod(c))) = (
            LogTarget::of_pod(&pod("a", main.clone())),
            LogTarget::of_pod(&pod("a", main.clone())),
            LogTarget::of_pod(&pod("c", main)),
        ) else {
            panic!("pod targets");
        };
        assert!(a.is_same(&b));
        assert!(!a.is_same(&c));
        let (
            Some(LogTarget::Workload(x)),
            Some(LogTarget::Workload(y)),
            Some(LogTarget::Workload(z)),
        ) = (
            LogTarget::of_workload(deployment("api")),
            LogTarget::of_workload(deployment("api")),
            LogTarget::of_workload(controller(STATEFUL_SET_KIND, "api")),
        )
        else {
            panic!("workload targets");
        };
        assert!(x.is_same(&y));
        assert!(!x.is_same(&z));
    }
}
