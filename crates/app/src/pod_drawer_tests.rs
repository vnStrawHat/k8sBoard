use cluster::{PodCondition, StatusReason, Termination};

use super::*;

fn container(
    name: &str,
    kind: ContainerKind,
    state: ContainerState,
    is_ready: bool,
) -> ContainerSummary {
    ContainerSummary {
        name: name.to_owned(),
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

fn termination(exit_code: i32) -> Termination {
    Termination {
        reason: Some(StatusReason::Error),
        exit_code,
        signal: None,
        started_at: None,
        finished_at: None,
    }
}

#[test]
fn default_container_is_first_main_that_is_not_ready() {
    let containers = [
        container("init", ContainerKind::Init, running(), false),
        container("ready", ContainerKind::Main, running(), true),
        container("broken", ContainerKind::Main, running(), false),
    ];
    assert_eq!(default_container(&containers), Some(2));
}

#[test]
fn default_container_falls_back_to_first_main_then_first() {
    let all_ready = [
        container("init", ContainerKind::Init, running(), true),
        container("a", ContainerKind::Main, running(), true),
        container("b", ContainerKind::Main, running(), true),
    ];
    assert_eq!(default_container(&all_ready), Some(1));
    let only_init = [container("init", ContainerKind::Init, running(), true)];
    assert_eq!(default_container(&only_init), Some(0));
    assert_eq!(default_container(&[]), None);
}

#[test]
fn group_titles_count_progress_per_kind() {
    let done = ContainerState::Terminated(termination(0));
    let failed = ContainerState::Terminated(termination(1));
    let init_done = container("a", ContainerKind::Init, done, false);
    let init_failed = container("b", ContainerKind::Init, failed, false);
    assert_eq!(
        group_title(ContainerKind::Init, &[&init_done, &init_failed]),
        "Init · ran in order 1/2"
    );
    let sidecar = container("s", ContainerKind::Sidecar, running(), true);
    assert_eq!(
        group_title(ContainerKind::Sidecar, &[&sidecar]),
        "Sidecars 1"
    );
    let ready = container("m", ContainerKind::Main, running(), true);
    let not_ready = container("n", ContainerKind::Main, running(), false);
    assert_eq!(
        group_title(ContainerKind::Main, &[&ready, &not_ready]),
        "Containers 1/2"
    );
}

#[test]
fn condition_tooltip_joins_reason_and_message() {
    let condition = |reason: Option<&str>, message: Option<&str>| PodCondition {
        name: "Ready".to_owned(),
        is_true: false,
        reason: reason.map(str::to_owned),
        message: message.map(str::to_owned),
    };
    assert_eq!(
        condition_tooltip(&condition(
            Some("ContainersNotReady"),
            Some("containers with unready status: [api]")
        ))
        .as_deref(),
        Some("ContainersNotReady: containers with unready status: [api]")
    );
    assert_eq!(
        condition_tooltip(&condition(Some("Unschedulable"), None)).as_deref(),
        Some("Unschedulable")
    );
    assert_eq!(
        condition_tooltip(&condition(None, Some("no nodes"))).as_deref(),
        Some("no nodes")
    );
    assert_eq!(condition_tooltip(&condition(None, None)), None);
}
