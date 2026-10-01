use k8s_openapi::api::core::v1::{
    ContainerState as ApiContainerState, ContainerStateRunning, ContainerStateWaiting,
    ContainerStatus, PodCondition as ApiPodCondition, PodSpec, PodStatus as ApiPodStatus,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};

use super::*;

fn timestamp(text: &str) -> jiff::Timestamp {
    text.parse().expect("valid timestamp")
}

fn container(name: &str, restart_policy: Option<&str>) -> Container {
    Container {
        name: name.to_owned(),
        restart_policy: restart_policy.map(str::to_owned),
        ..Default::default()
    }
}

fn pod_with_containers(init: Vec<Container>, main: Vec<Container>) -> Pod {
    Pod {
        spec: Some(PodSpec {
            init_containers: Some(init),
            containers: main,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn with_statuses(
    mut pod: Pod,
    init_statuses: Vec<ContainerStatus>,
    main_statuses: Vec<ContainerStatus>,
) -> Pod {
    pod.status = Some(ApiPodStatus {
        init_container_statuses: Some(init_statuses),
        container_statuses: Some(main_statuses),
        ..Default::default()
    });
    pod
}

fn status_with_state(name: &str, state: Option<ApiContainerState>) -> ContainerStatus {
    ContainerStatus {
        name: name.to_owned(),
        state,
        ..Default::default()
    }
}

fn waiting_state(reason: Option<&str>) -> ApiContainerState {
    ApiContainerState {
        waiting: Some(ContainerStateWaiting {
            reason: reason.map(str::to_owned),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn terminated_state(
    reason: Option<&str>,
    exit_code: i32,
    signal: Option<i32>,
) -> ContainerStateTerminated {
    ContainerStateTerminated {
        reason: reason.map(str::to_owned),
        exit_code,
        signal,
        ..Default::default()
    }
}

fn summaries(pod: &Pod) -> Vec<ContainerSummary> {
    container_summaries(pod)
}

#[test]
fn containers_list_init_containers_then_main_in_spec_order() {
    let pod = pod_with_containers(
        vec![
            container("init-a", None),
            container("side", Some("Always")),
            container("init-b", None),
        ],
        vec![container("main-a", None), container("main-b", None)],
    );
    let listed: Vec<_> = summaries(&pod)
        .into_iter()
        .map(|summary| (summary.name, summary.kind))
        .collect();
    assert_eq!(
        listed,
        [
            ("init-a".to_owned(), ContainerKind::Init),
            ("side".to_owned(), ContainerKind::Sidecar),
            ("init-b".to_owned(), ContainerKind::Init),
            ("main-a".to_owned(), ContainerKind::Main),
            ("main-b".to_owned(), ContainerKind::Main),
        ]
    );
}

#[test]
fn init_container_with_restart_policy_always_is_sidecar() {
    let pod = pod_with_containers(vec![container("side", Some("Always"))], Vec::new());
    assert_eq!(summaries(&pod)[0].kind, ContainerKind::Sidecar);
}

#[test]
fn init_container_without_restart_policy_always_is_init() {
    let pod = pod_with_containers(
        vec![container("a", None), container("b", Some("OnFailure"))],
        Vec::new(),
    );
    let kinds: Vec<_> = summaries(&pod).iter().map(|summary| summary.kind).collect();
    assert_eq!(kinds, [ContainerKind::Init, ContainerKind::Init]);
}

#[test]
fn container_without_status_is_not_reported() {
    let pod = pod_with_containers(Vec::new(), vec![container("main", None)]);
    let summary = &summaries(&pod)[0];
    assert_eq!(summary.state, ContainerState::NotReported);
    assert!(!summary.is_ready);
    assert_eq!(summary.restart_count, 0);
    assert_eq!(summary.last_termination, None);
}

#[test]
fn container_state_maps_waiting_running_terminated() {
    let started = timestamp("2024-05-01T10:00:00Z");
    let running = ApiContainerState {
        running: Some(ContainerStateRunning {
            started_at: Some(Time(started)),
        }),
        ..Default::default()
    };
    let terminated = ApiContainerState {
        terminated: Some(terminated_state(Some("Completed"), 0, None)),
        ..Default::default()
    };
    let pod = with_statuses(
        pod_with_containers(
            Vec::new(),
            vec![
                container("waiting", None),
                container("blank-waiting", None),
                container("running", None),
                container("terminated", None),
                container("empty", None),
            ],
        ),
        Vec::new(),
        vec![
            status_with_state("waiting", Some(waiting_state(Some("ImagePullBackOff")))),
            status_with_state("blank-waiting", Some(waiting_state(Some("")))),
            status_with_state("running", Some(running)),
            status_with_state("terminated", Some(terminated)),
            status_with_state("empty", None),
        ],
    );
    let states: Vec<_> = summaries(&pod)
        .into_iter()
        .map(|summary| summary.state)
        .collect();
    assert_eq!(
        states,
        [
            ContainerState::Waiting {
                reason: Some(StatusReason::ImagePullBackOff)
            },
            ContainerState::Waiting { reason: None },
            ContainerState::Running {
                started_at: Some(started)
            },
            ContainerState::Terminated(Termination {
                reason: Some(StatusReason::Completed),
                exit_code: 0,
                signal: None,
                started_at: None,
                finished_at: None,
            }),
            ContainerState::NotReported,
        ]
    );
}

#[test]
fn terminated_state_precedes_running_and_waiting() {
    let everything = ApiContainerState {
        waiting: Some(ContainerStateWaiting::default()),
        running: Some(ContainerStateRunning::default()),
        terminated: Some(terminated_state(None, 2, None)),
    };
    assert!(matches!(
        container_state(&everything),
        ContainerState::Terminated(_)
    ));

    let running_and_waiting = ApiContainerState {
        waiting: Some(ContainerStateWaiting::default()),
        running: Some(ContainerStateRunning::default()),
        terminated: None,
    };
    assert!(matches!(
        container_state(&running_and_waiting),
        ContainerState::Running { .. }
    ));
}

#[test]
fn last_termination_carries_reason_exit_code_and_signal() {
    let mut status = status_with_state("main", Some(waiting_state(Some("CrashLoopBackOff"))));
    status.last_state = Some(ApiContainerState {
        terminated: Some(terminated_state(Some("OOMKilled"), 137, Some(0))),
        ..Default::default()
    });
    let pod = with_statuses(
        pod_with_containers(Vec::new(), vec![container("main", None)]),
        Vec::new(),
        vec![status],
    );
    assert_eq!(
        summaries(&pod)[0].last_termination,
        Some(Termination {
            reason: Some(StatusReason::OomKilled),
            exit_code: 137,
            signal: None,
            started_at: None,
            finished_at: None,
        })
    );

    let signalled = terminated_state(None, 137, Some(9));
    assert_eq!(termination(&signalled).signal, Some(9));
    assert_eq!(termination(&signalled).reason, None);
}

#[test]
fn negative_restart_count_clamps_to_zero() {
    let mut status = status_with_state("main", None);
    status.restart_count = -3;
    let pod = with_statuses(
        pod_with_containers(Vec::new(), vec![container("main", None)]),
        Vec::new(),
        vec![status],
    );
    assert_eq!(summaries(&pod)[0].restart_count, 0);
}

#[test]
fn pod_summary_reads_namespace_name_node_and_creation_time() {
    let created = timestamp("2024-05-01T10:00:00Z");
    let mut pod = pod_with_containers(Vec::new(), vec![container("main", None)]);
    pod.metadata = ObjectMeta {
        namespace: Some("team-a".to_owned()),
        name: Some("web-1".to_owned()),
        creation_timestamp: Some(Time(created)),
        ..Default::default()
    };
    if let Some(spec) = pod.spec.as_mut() {
        spec.node_name = Some("node-1".to_owned());
    }
    pod.status = Some(ApiPodStatus {
        phase: Some("Pending".to_owned()),
        ..Default::default()
    });

    let summary = pod_summary(&pod);
    assert_eq!(summary.namespace, "team-a");
    assert_eq!(summary.name, "web-1");
    assert_eq!(summary.node_name.as_deref(), Some("node-1"));
    assert_eq!(summary.created_at, Some(created));
    assert_eq!(summary.status, PodStatus::Reason(StatusReason::Pending));
    assert_eq!(summary.ready, ReadyCount { ready: 0, total: 1 });
    assert_eq!(summary.restarts, 0);
    assert_eq!(summary.containers.len(), 1);
}

#[test]
fn pod_summary_reads_ip_qos_service_account() {
    let mut pod = pod_with_containers(Vec::new(), vec![container("main", None)]);
    if let Some(spec) = pod.spec.as_mut() {
        spec.service_account_name = Some("web".to_owned());
    }
    pod.status = Some(ApiPodStatus {
        pod_ip: Some("10.1.2.3".to_owned()),
        qos_class: Some("Burstable".to_owned()),
        ..Default::default()
    });
    let summary = pod_summary(&pod);
    assert_eq!(summary.pod_ip.as_deref(), Some("10.1.2.3"));
    assert_eq!(summary.qos_class.as_deref(), Some("Burstable"));
    assert_eq!(summary.service_account.as_deref(), Some("web"));

    if let Some(spec) = pod.spec.as_mut() {
        spec.service_account_name = Some(String::new());
    }
    pod.status = Some(ApiPodStatus {
        pod_ip: Some(String::new()),
        ..Default::default()
    });
    let summary = pod_summary(&pod);
    assert_eq!(summary.pod_ip, None);
    assert_eq!(summary.qos_class, None);
    assert_eq!(summary.service_account, None);
}

#[test]
fn pod_summary_conditions_keep_api_order_and_truth() {
    let condition = |name: &str, status: &str| ApiPodCondition {
        type_: name.to_owned(),
        status: status.to_owned(),
        ..Default::default()
    };
    let mut pod = pod_with_containers(Vec::new(), Vec::new());
    pod.status = Some(ApiPodStatus {
        conditions: Some(vec![
            condition("PodScheduled", "True"),
            condition("Ready", "False"),
            condition("ContainersReady", "Unknown"),
        ]),
        ..Default::default()
    });
    let listed: Vec<_> = pod_summary(&pod)
        .conditions
        .into_iter()
        .map(|condition| (condition.name, condition.is_true))
        .collect();
    assert_eq!(
        listed,
        [
            ("PodScheduled".to_owned(), true),
            ("Ready".to_owned(), false),
            ("ContainersReady".to_owned(), false),
        ]
    );
}

#[test]
fn container_summary_reads_image() {
    let with_image = Container {
        name: "main".to_owned(),
        image: Some("nginx:1.27".to_owned()),
        ..Default::default()
    };
    let pod = pod_with_containers(Vec::new(), vec![with_image, container("bare", None)]);
    let images: Vec<_> = summaries(&pod)
        .into_iter()
        .map(|summary| summary.image)
        .collect();
    assert_eq!(images, ["nginx:1.27", ""]);
}
