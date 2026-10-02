use cluster::{ReadyCount, Termination};

use super::*;

fn container(kind: ContainerKind, state: ContainerState, is_ready: bool) -> ContainerSummary {
    ContainerSummary {
        name: "c".to_owned(),
        image: "img".to_owned(),
        kind,
        state,
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

fn running() -> ContainerState {
    ContainerState::Running { started_at: None }
}

fn termination(reason: Option<StatusReason>, exit_code: i32) -> Termination {
    Termination {
        reason,
        exit_code,
        signal: None,
        started_at: None,
        finished_at: None,
    }
}

fn pod(status: PodStatus, ready: u32, containers: Vec<ContainerSummary>) -> PodSummary {
    let total = u32::try_from(containers.len()).unwrap_or_default();
    PodSummary {
        namespace: "ns".to_owned(),
        name: "pod".to_owned(),
        status,
        ready: ReadyCount { ready, total },
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

fn reason_pod(reason: StatusReason) -> PodSummary {
    pod(PodStatus::Reason(reason), 0, Vec::new())
}

#[test]
fn pod_running_all_ready_is_ok() {
    let all_ready = pod(
        PodStatus::Reason(StatusReason::Running),
        1,
        vec![container(ContainerKind::Main, running(), true)],
    );
    let label = pod_status_label(&all_ready);
    assert_eq!(label.text, "Running");
    assert_eq!(label.tone, StatusTone::Ok);
}

#[test]
fn pod_readiness_failed_when_running_and_not_all_ready() {
    let finished_init = ContainerState::Terminated(termination(Some(StatusReason::Completed), 0));
    let not_ready = pod(
        PodStatus::Reason(StatusReason::Running),
        0,
        vec![
            container(ContainerKind::Init, finished_init, false),
            container(ContainerKind::Main, running(), false),
        ],
    );
    let label = pod_status_label(&not_ready);
    assert_eq!(label.text, "Readiness failed");
    assert_eq!(label.tone, StatusTone::Warn);
}

#[test]
fn pod_running_not_ready_with_waiting_container_keeps_status_text() {
    let waiting = pod(
        PodStatus::Reason(StatusReason::Running),
        0,
        vec![
            container(ContainerKind::Main, running(), false),
            container(
                ContainerKind::Main,
                ContainerState::Waiting {
                    reason: None,
                    message: None,
                },
                false,
            ),
        ],
    );
    let label = pod_status_label(&waiting);
    assert_eq!(label.text, "Running");
    assert_eq!(label.tone, StatusTone::Ok);
}

#[test]
fn pod_bad_reasons_are_bad() {
    let bad = [
        StatusReason::CrashLoopBackOff,
        StatusReason::ImagePullBackOff,
        StatusReason::ErrImagePull,
        StatusReason::CreateContainerConfigError,
        StatusReason::InvalidImageName,
        StatusReason::ErrImageNeverPull,
        StatusReason::CreateContainerError,
        StatusReason::OomKilled,
        StatusReason::Error,
        StatusReason::ContainerCannotRun,
        StatusReason::Failed,
        StatusReason::Evicted,
        StatusReason::Signal(9),
        StatusReason::ExitCode(2),
    ];
    for reason in bad {
        let label = pod_status_label(&reason_pod(reason.clone()));
        assert_eq!(label.tone, StatusTone::Bad, "{reason}");
        assert_eq!(label.text, reason.to_string());
    }
}

#[test]
fn pod_init_reason_takes_reason_tone() {
    let init_bad = pod(
        PodStatus::Init(InitStatus::Reason(StatusReason::CrashLoopBackOff)),
        0,
        Vec::new(),
    );
    let label = pod_status_label(&init_bad);
    assert_eq!(label.text, "Init:CrashLoopBackOff");
    assert_eq!(label.tone, StatusTone::Bad);

    let progress = pod(
        PodStatus::Init(InitStatus::Progress {
            first_incomplete: 1,
            total: 2,
        }),
        0,
        Vec::new(),
    );
    let label = pod_status_label(&progress);
    assert_eq!(label.text, "Init:1/2");
    assert_eq!(label.tone, StatusTone::Info);
}

#[test]
fn pod_completed_is_done_and_terminating_is_info() {
    assert_eq!(
        pod_status_label(&reason_pod(StatusReason::Completed)).tone,
        StatusTone::Done
    );
    assert_eq!(
        pod_status_label(&reason_pod(StatusReason::Succeeded)).tone,
        StatusTone::Done
    );
    let terminating = pod(PodStatus::Terminating, 0, Vec::new());
    assert_eq!(pod_status_label(&terminating).tone, StatusTone::Info);
}

#[test]
fn node_ready_scheduling_disabled_is_warn_with_suffix() {
    let label = node_status_label(NodeStatus {
        readiness: NodeReadiness::Ready,
        scheduling: NodeScheduling::Disabled,
    });
    assert_eq!(label.text, "Ready · SchedulingDisabled");
    assert_eq!(label.tone, StatusTone::Warn);
}

#[test]
fn node_not_ready_is_bad() {
    let cordoned = node_status_label(NodeStatus {
        readiness: NodeReadiness::NotReady,
        scheduling: NodeScheduling::Disabled,
    });
    assert_eq!(cordoned.text, "NotReady · SchedulingDisabled");
    assert_eq!(cordoned.tone, StatusTone::Bad);
    let enabled = node_status_label(NodeStatus {
        readiness: NodeReadiness::NotReady,
        scheduling: NodeScheduling::Enabled,
    });
    assert_eq!(enabled.text, "NotReady");
    assert_eq!(enabled.tone, StatusTone::Bad);
}

#[test]
fn container_terminated_exit_zero_is_done_nonzero_is_bad() {
    let done = container(
        ContainerKind::Init,
        ContainerState::Terminated(termination(Some(StatusReason::Completed), 0)),
        false,
    );
    assert_eq!(container_state_label(&done).tone, StatusTone::Done);
    let failed = container(
        ContainerKind::Main,
        ContainerState::Terminated(termination(Some(StatusReason::Error), 1)),
        false,
    );
    let label = container_state_label(&failed);
    assert_eq!(label.text, "Error");
    assert_eq!(label.tone, StatusTone::Bad);
}

#[test]
fn light_theme_text_is_darker_than_the_fill_colour() {
    let green = gpui_kit::hsla(0.38, 0.6, 0.5, 1.);
    let foreground = gpui_kit::hsla(0., 0., 0.05, 1.);
    let text = readable_on_light(green, foreground, LIGHT_THEME_TONE_SHARE);
    assert!(text.l < green.l);
    assert!(text.l > foreground.l);
    // Amber keeps less of its hue, so it ends darker than the same fill at the common share.
    let amber = gpui_kit::hsla(0.12, 0.8, 0.5, 1.);
    let common = readable_on_light(amber, foreground, LIGHT_THEME_TONE_SHARE);
    let warn = readable_on_light(amber, foreground, LIGHT_THEME_WARN_SHARE);
    assert!(warn.l < common.l);
}

#[test]
fn new_reasons_are_bad() {
    for reason in [
        StatusReason::InvalidImageName,
        StatusReason::ErrImageNeverPull,
        StatusReason::CreateContainerError,
    ] {
        assert!(is_bad_reason(&reason), "{reason}");
        let waiting = container(
            ContainerKind::Main,
            ContainerState::Waiting {
                reason: Some(reason.clone()),
                message: None,
            },
            false,
        );
        assert_eq!(
            container_state_label(&waiting).tone,
            StatusTone::Bad,
            "{reason}"
        );
    }
}

#[test]
fn node_condition_tone_table() {
    let tone = |name: &str, status| {
        node_condition_tone(&NodeCondition {
            name: name.to_owned(),
            status,
            reason: None,
            message: None,
            changed_at: None,
        })
    };
    use ConditionStatus::{False, True, Unknown};
    assert_eq!(tone("Ready", True), StatusTone::Ok);
    assert_eq!(tone("Ready", False), StatusTone::Bad);
    assert_eq!(tone("Ready", Unknown), StatusTone::Warn);
    for problem in [
        "MemoryPressure",
        "DiskPressure",
        "PIDPressure",
        "KernelDeadlock",
    ] {
        assert_eq!(tone(problem, True), StatusTone::Bad, "{problem}");
        assert_eq!(tone(problem, False), StatusTone::Ok, "{problem}");
        assert_eq!(tone(problem, Unknown), StatusTone::Warn, "{problem}");
    }
}

#[test]
fn condition_status_text_names_each_status() {
    use ConditionStatus::{False, True, Unknown};
    assert_eq!(condition_status_text(True), "True");
    assert_eq!(condition_status_text(False), "False");
    assert_eq!(condition_status_text(Unknown), "Unknown");
}

#[test]
fn chart_text_color_moves_toward_the_foreground_on_both_themes() {
    let pale_blue = gpui_kit::hsla(0.58, 0.8, 0.8, 1.);
    let black = gpui_kit::hsla(0., 0., 0.05, 1.);
    assert!(chart_text_color(pale_blue, black, false).l < pale_blue.l);
    let dark_blue = gpui_kit::hsla(0.62, 0.7, 0.3, 1.);
    let white = gpui_kit::hsla(0., 0., 0.95, 1.);
    assert!(chart_text_color(dark_blue, white, true).l > dark_blue.l);
}
