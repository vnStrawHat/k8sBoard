use k8s_openapi::api::core::v1::{
    ContainerState as ApiContainerState, ContainerStateRunning, ContainerStateTerminated,
    ContainerStateWaiting, ContainerStatus, PodCondition, PodSpec,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

use super::*;

/// A pod with the given main containers and init containers (name, restartPolicy).
fn pod(phase: Option<&str>, main: &[&str], init: &[(&str, Option<&str>)]) -> Pod {
    let container = |name: &str, restart_policy: Option<&str>| Container {
        name: name.to_owned(),
        restart_policy: restart_policy.map(str::to_owned),
        ..Default::default()
    };
    Pod {
        spec: Some(PodSpec {
            containers: main.iter().map(|name| container(name, None)).collect(),
            init_containers: Some(
                init.iter()
                    .map(|(name, policy)| container(name, *policy))
                    .collect(),
            ),
            ..Default::default()
        }),
        status: Some(ApiPodStatus {
            phase: phase.map(str::to_owned),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn status_mut(pod: &mut Pod) -> &mut ApiPodStatus {
    pod.status.get_or_insert_default()
}

fn set_main_statuses(pod: &mut Pod, statuses: Vec<ContainerStatus>) {
    status_mut(pod).container_statuses = Some(statuses);
}

fn set_init_statuses(pod: &mut Pod, statuses: Vec<ContainerStatus>) {
    status_mut(pod).init_container_statuses = Some(statuses);
}

fn add_condition(pod: &mut Pod, type_: &str, status: &str, reason: Option<&str>) {
    status_mut(pod)
        .conditions
        .get_or_insert_default()
        .push(PodCondition {
            type_: type_.to_owned(),
            status: status.to_owned(),
            reason: reason.map(str::to_owned),
            ..Default::default()
        });
}

fn mark_deleting(pod: &mut Pod) {
    pod.metadata.deletion_timestamp = Some(Time(jiff::Timestamp::UNIX_EPOCH));
}

fn container_status(name: &str, state: ApiContainerState) -> ContainerStatus {
    ContainerStatus {
        name: name.to_owned(),
        state: Some(state),
        ..Default::default()
    }
}

fn running(name: &str, is_ready: bool) -> ContainerStatus {
    ContainerStatus {
        ready: is_ready,
        ..container_status(
            name,
            ApiContainerState {
                running: Some(ContainerStateRunning::default()),
                ..Default::default()
            },
        )
    }
}

fn waiting(name: &str, reason: Option<&str>) -> ContainerStatus {
    container_status(
        name,
        ApiContainerState {
            waiting: Some(ContainerStateWaiting {
                reason: reason.map(str::to_owned),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
}

fn terminated(
    name: &str,
    reason: Option<&str>,
    exit_code: i32,
    signal: Option<i32>,
) -> ContainerStatus {
    container_status(
        name,
        ApiContainerState {
            terminated: Some(ContainerStateTerminated {
                reason: reason.map(str::to_owned),
                exit_code,
                signal,
                ..Default::default()
            }),
            ..Default::default()
        },
    )
}

fn with_restarts(mut status: ContainerStatus, restart_count: i32) -> ContainerStatus {
    status.restart_count = restart_count;
    status
}

fn started(mut status: ContainerStatus) -> ContainerStatus {
    status.started = Some(true);
    status
}

fn reason(reason: StatusReason) -> PodStatus {
    PodStatus::Reason(reason)
}

fn status_of(pod: &Pod) -> PodStatus {
    pod_display(pod).status
}

#[test]
fn running_pod_with_ready_containers_is_running() {
    let mut pod = pod(Some("Running"), &["a", "b"], &[]);
    set_main_statuses(&mut pod, vec![running("a", true), running("b", true)]);
    let display = pod_display(&pod);
    assert_eq!(display.status, reason(StatusReason::Running));
    assert_eq!(display.ready, ReadyCount { ready: 2, total: 2 });
}

#[test]
fn pending_pod_without_statuses_is_pending() {
    let pod = pod(Some("Pending"), &["a", "b"], &[]);
    let display = pod_display(&pod);
    assert_eq!(display.status, reason(StatusReason::Pending));
    assert_eq!(display.ready, ReadyCount { ready: 0, total: 2 });
}

#[test]
fn missing_phase_is_unknown() {
    assert_eq!(status_of(&Pod::default()), reason(StatusReason::Unknown));
    assert_eq!(
        status_of(&pod(None, &["a"], &[])),
        reason(StatusReason::Unknown)
    );
}

#[test]
fn pod_reason_overrides_phase() {
    let mut pod = pod(Some("Failed"), &["a"], &[]);
    status_mut(&mut pod).reason = Some("Evicted".to_owned());
    assert_eq!(status_of(&pod), reason(StatusReason::Evicted));
}

#[test]
fn scheduling_gated_condition_shows_scheduling_gated() {
    let mut pod = pod(Some("Pending"), &["a"], &[]);
    add_condition(&mut pod, "PodScheduled", "False", Some("SchedulingGated"));
    assert_eq!(status_of(&pod), reason(StatusReason::SchedulingGated));
}

#[test]
fn waiting_crash_loop_back_off_overrides_phase() {
    let mut pod = pod(Some("Running"), &["a"], &[]);
    set_main_statuses(&mut pod, vec![waiting("a", Some("CrashLoopBackOff"))]);
    assert_eq!(status_of(&pod), reason(StatusReason::CrashLoopBackOff));
}

#[test]
fn waiting_image_pull_back_off_overrides_phase() {
    let mut pod = pod(Some("Pending"), &["a"], &[]);
    set_main_statuses(&mut pod, vec![waiting("a", Some("ImagePullBackOff"))]);
    assert_eq!(status_of(&pod), reason(StatusReason::ImagePullBackOff));
}

#[test]
fn waiting_container_creating_overrides_phase() {
    let mut pod = pod(Some("Pending"), &["a"], &[]);
    set_main_statuses(&mut pod, vec![waiting("a", Some("ContainerCreating"))]);
    assert_eq!(status_of(&pod), reason(StatusReason::ContainerCreating));
}

#[test]
fn terminated_oom_killed_shows_oom_killed() {
    let mut pod = pod(Some("Running"), &["a"], &[]);
    set_main_statuses(
        &mut pod,
        vec![terminated("a", Some("OOMKilled"), 137, None)],
    );
    assert_eq!(status_of(&pod), reason(StatusReason::OomKilled));
}

#[test]
fn succeeded_pod_with_completed_containers_shows_completed() {
    let mut pod = pod(Some("Succeeded"), &["a"], &[]);
    set_main_statuses(&mut pod, vec![terminated("a", Some("Completed"), 0, None)]);
    assert_eq!(status_of(&pod), reason(StatusReason::Completed));
}

#[test]
fn terminated_without_reason_shows_exit_code() {
    let mut pod = pod(Some("Failed"), &["a"], &[]);
    set_main_statuses(&mut pod, vec![terminated("a", None, 1, None)]);
    assert_eq!(status_of(&pod), reason(StatusReason::ExitCode(1)));
}

#[test]
fn terminated_without_reason_with_signal_shows_signal() {
    let mut pod = pod(Some("Failed"), &["a"], &[]);
    set_main_statuses(&mut pod, vec![terminated("a", Some(""), 137, Some(9))]);
    assert_eq!(status_of(&pod), reason(StatusReason::Signal(9)));
}

#[test]
fn first_failing_container_in_spec_order_supplies_reason() {
    let mut pod = pod(Some("Running"), &["a", "b"], &[]);
    set_main_statuses(
        &mut pod,
        vec![
            waiting("a", Some("CrashLoopBackOff")),
            terminated("b", Some("Error"), 1, None),
        ],
    );
    assert_eq!(status_of(&pod), reason(StatusReason::CrashLoopBackOff));
}

#[test]
fn completed_with_running_container_and_ready_condition_is_running() {
    let mut pod = pod(Some("Running"), &["a", "b"], &[]);
    set_main_statuses(
        &mut pod,
        vec![
            terminated("a", Some("Completed"), 0, None),
            running("b", true),
        ],
    );
    add_condition(&mut pod, "Ready", "True", None);
    assert_eq!(status_of(&pod), reason(StatusReason::Running));
}

#[test]
fn completed_with_running_container_without_ready_condition_is_not_ready() {
    let mut pod = pod(Some("Running"), &["a", "b"], &[]);
    set_main_statuses(
        &mut pod,
        vec![
            terminated("a", Some("Completed"), 0, None),
            running("b", true),
        ],
    );
    assert_eq!(status_of(&pod), PodStatus::NotReady);
}

#[test]
fn deleting_running_pod_is_terminating() {
    let mut pod = pod(Some("Running"), &["a"], &[]);
    set_main_statuses(&mut pod, vec![running("a", true)]);
    mark_deleting(&mut pod);
    assert_eq!(status_of(&pod), PodStatus::Terminating);
}

#[test]
fn deleting_succeeded_pod_keeps_completed() {
    let mut pod = pod(Some("Succeeded"), &["a"], &[]);
    set_main_statuses(&mut pod, vec![terminated("a", Some("Completed"), 0, None)]);
    mark_deleting(&mut pod);
    assert_eq!(status_of(&pod), reason(StatusReason::Completed));
}

#[test]
fn deleting_pod_with_node_lost_reason_is_unknown() {
    let mut pod = pod(Some("Running"), &["a"], &[]);
    status_mut(&mut pod).reason = Some("NodeLost".to_owned());
    mark_deleting(&mut pod);
    assert_eq!(status_of(&pod), reason(StatusReason::Unknown));
}

#[test]
fn init_progress_shows_first_incomplete_index_over_total() {
    let mut pod = pod(
        Some("Pending"),
        &["main"],
        &[("init-a", None), ("init-b", None), ("side", Some("Always"))],
    );
    set_init_statuses(
        &mut pod,
        vec![
            terminated("init-a", Some("Completed"), 0, None),
            waiting("init-b", None),
            waiting("side", None),
        ],
    );
    assert_eq!(
        status_of(&pod),
        PodStatus::Init(InitStatus::Progress {
            first_incomplete: 1,
            total: 3
        })
    );
}

#[test]
fn init_waiting_reason_shows_init_prefix() {
    let mut pod = pod(Some("Pending"), &["main"], &[("init-a", None)]);
    set_init_statuses(&mut pod, vec![waiting("init-a", Some("CrashLoopBackOff"))]);
    assert_eq!(
        status_of(&pod),
        PodStatus::Init(InitStatus::Reason(StatusReason::CrashLoopBackOff))
    );
}

#[test]
fn init_waiting_pod_initializing_shows_progress() {
    let mut pod = pod(
        Some("Pending"),
        &["main"],
        &[("init-a", None), ("init-b", None)],
    );
    set_init_statuses(
        &mut pod,
        vec![
            waiting("init-a", Some("PodInitializing")),
            waiting("init-b", None),
        ],
    );
    assert_eq!(
        status_of(&pod),
        PodStatus::Init(InitStatus::Progress {
            first_incomplete: 0,
            total: 2
        })
    );
}

#[test]
fn init_terminated_failure_shows_init_reason() {
    let mut pod = pod(Some("Pending"), &["main"], &[("init-a", None)]);
    set_init_statuses(&mut pod, vec![terminated("init-a", Some("Error"), 1, None)]);
    assert_eq!(
        status_of(&pod),
        PodStatus::Init(InitStatus::Reason(StatusReason::Error))
    );
}

#[test]
fn init_terminated_without_reason_shows_init_exit_code_or_signal() {
    let mut exit_code_pod = pod(Some("Pending"), &["main"], &[("init-a", None)]);
    set_init_statuses(
        &mut exit_code_pod,
        vec![terminated("init-a", None, 1, None)],
    );
    assert_eq!(
        status_of(&exit_code_pod),
        PodStatus::Init(InitStatus::Reason(StatusReason::ExitCode(1)))
    );
    assert_eq!(status_of(&exit_code_pod).to_string(), "Init:ExitCode:1");

    let mut signal_pod = pod(Some("Pending"), &["main"], &[("init-a", None)]);
    set_init_statuses(
        &mut signal_pod,
        vec![terminated("init-a", None, 137, Some(9))],
    );
    assert_eq!(
        status_of(&signal_pod),
        PodStatus::Init(InitStatus::Reason(StatusReason::Signal(9)))
    );
    assert_eq!(status_of(&signal_pod).to_string(), "Init:Signal:9");
}

#[test]
fn started_sidecar_does_not_block_init_progress() {
    let mut pod = pod(
        Some("Pending"),
        &["main"],
        &[("side", Some("Always")), ("init-b", None)],
    );
    set_init_statuses(
        &mut pod,
        vec![started(running("side", true)), waiting("init-b", None)],
    );
    let display = pod_display(&pod);
    assert_eq!(
        display.status,
        PodStatus::Init(InitStatus::Progress {
            first_incomplete: 1,
            total: 2
        })
    );
    assert_eq!(display.ready.ready, 1);
}

#[test]
fn initialized_condition_uses_main_container_status() {
    let mut pod = pod(Some("Running"), &["main"], &[("init-a", None)]);
    set_init_statuses(&mut pod, vec![waiting("init-a", Some("PodInitializing"))]);
    set_main_statuses(&mut pod, vec![waiting("main", Some("CrashLoopBackOff"))]);
    assert_eq!(
        status_of(&pod),
        PodStatus::Init(InitStatus::Progress {
            first_incomplete: 0,
            total: 1
        })
    );

    add_condition(&mut pod, "Initialized", "True", None);
    assert_eq!(status_of(&pod), reason(StatusReason::CrashLoopBackOff));
}

#[test]
fn ready_total_counts_main_and_sidecar_containers() {
    let pod = pod(
        Some("Running"),
        &["a", "b", "c"],
        &[("classic", None), ("side", Some("Always"))],
    );
    assert_eq!(pod_display(&pod).ready, ReadyCount { ready: 0, total: 4 });
    assert_eq!(pod_display(&pod).ready.to_string(), "0/4");
}

#[test]
fn ready_counts_started_ready_sidecars() {
    let mut pod = pod(Some("Running"), &["main"], &[("side", Some("Always"))]);
    set_init_statuses(&mut pod, vec![started(running("side", true))]);
    set_main_statuses(&mut pod, vec![running("main", true)]);
    assert_eq!(pod_display(&pod).ready, ReadyCount { ready: 2, total: 2 });
}

#[test]
fn running_container_that_is_not_ready_is_not_counted() {
    let mut pod = pod(Some("Running"), &["a"], &[]);
    set_main_statuses(&mut pod, vec![running("a", false)]);
    let display = pod_display(&pod);
    assert_eq!(display.status, reason(StatusReason::Running));
    assert_eq!(display.ready, ReadyCount { ready: 0, total: 1 });
}

#[test]
fn restarts_sum_sidecar_and_main_after_initialization() {
    let mut pod = pod(
        Some("Running"),
        &["main"],
        &[("classic", None), ("side", Some("Always"))],
    );
    set_init_statuses(
        &mut pod,
        vec![
            with_restarts(terminated("classic", Some("Completed"), 0, None), 5),
            with_restarts(started(running("side", true)), 2),
        ],
    );
    set_main_statuses(&mut pod, vec![with_restarts(running("main", true), 3)]);
    assert_eq!(pod_display(&pod).restarts, 5);
}

#[test]
fn restarts_sum_init_containers_while_initializing() {
    let mut pod = pod(
        Some("Pending"),
        &["main"],
        &[("init-a", None), ("init-b", None)],
    );
    set_init_statuses(
        &mut pod,
        vec![
            with_restarts(terminated("init-a", Some("Completed"), 0, None), 1),
            with_restarts(waiting("init-b", None), 2),
        ],
    );
    set_main_statuses(&mut pod, vec![with_restarts(waiting("main", None), 7)]);
    assert_eq!(pod_display(&pod).restarts, 3);
}

#[test]
fn status_reason_from_api_maps_known_text_and_keeps_unknown() {
    assert_eq!(StatusReason::from_api("OOMKilled"), StatusReason::OomKilled);
    assert_eq!(
        StatusReason::from_api("CrashLoopBackOff"),
        StatusReason::CrashLoopBackOff
    );
    assert_eq!(
        StatusReason::from_api("Foo"),
        StatusReason::Other("Foo".to_owned())
    );
    // Matching is case-sensitive and never produces Signal or ExitCode.
    assert_eq!(
        StatusReason::from_api("running"),
        StatusReason::Other("running".to_owned())
    );
    assert_eq!(
        StatusReason::from_api("Signal:9"),
        StatusReason::Other("Signal:9".to_owned())
    );
}

#[test]
fn status_display_matches_kubectl_text() {
    let cases = [
        (reason(StatusReason::OomKilled), "OOMKilled"),
        (
            PodStatus::Init(InitStatus::Progress {
                first_incomplete: 1,
                total: 3,
            }),
            "Init:1/3",
        ),
        (
            PodStatus::Init(InitStatus::Reason(StatusReason::CrashLoopBackOff)),
            "Init:CrashLoopBackOff",
        ),
        (reason(StatusReason::Signal(9)), "Signal:9"),
        (reason(StatusReason::ExitCode(1)), "ExitCode:1"),
        (PodStatus::NotReady, "NotReady"),
        (PodStatus::Terminating, "Terminating"),
        (reason(StatusReason::Other("Weird".to_owned())), "Weird"),
    ];
    for (status, text) in cases {
        assert_eq!(status.to_string(), text);
    }
}
