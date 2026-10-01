use cluster::StatusReason;

use super::*;

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

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
fn last_state_text_lists_reason_exit_signal_and_age() {
    let mut last = termination(137);
    last.reason = Some(StatusReason::OomKilled);
    last.signal = Some(9);
    last.finished_at = Some(at(1_000));
    assert_eq!(
        last_state_text(&last, at(1_000 + 3 * 3_600)),
        "OOMKilled · exit 137 · signal 9 · ended 3h ago"
    );
    let bare = Termination {
        reason: None,
        exit_code: 1,
        signal: None,
        started_at: None,
        finished_at: None,
    };
    assert_eq!(last_state_text(&bare, at(0)), "Terminated · exit 1");
}

#[test]
fn state_text_running_includes_started_age() {
    let started = ContainerState::Running {
        started_at: Some(at(0)),
    };
    let up = container("m", ContainerKind::Main, started, true);
    let label = container_state_label(&up);
    assert_eq!(state_text(&up, &label, at(120)), "Running · started 2m ago");
    let waiting = container(
        "w",
        ContainerKind::Main,
        ContainerState::Waiting { reason: None },
        false,
    );
    let label = container_state_label(&waiting);
    assert_eq!(state_text(&waiting, &label, at(120)), "Waiting");
}
