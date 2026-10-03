use cluster::{ByteAmount, ContainerResource, CpuAmount, PodCondition, StatusReason, Termination};

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
        changed_at: None,
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

fn resource(name: &str, request: Option<&str>, limit: Option<&str>) -> ContainerResource {
    ContainerResource {
        name: name.to_owned(),
        request: request.map(str::to_owned),
        limit: limit.map(str::to_owned),
    }
}

fn usage(millicores: u64, mebibytes: u64) -> ResourceUsage {
    ResourceUsage {
        cpu: CpuAmount::from_nanocores(millicores * 1_000_000),
        memory: ByteAmount::from_bytes(mebibytes << 20),
    }
}

#[test]
fn container_usage_row_shows_usage_of_limit() {
    let row = container_usage_row(
        &resource("memory", Some("256Mi"), Some("512Mi")),
        Some(usage(0, 498)),
    )
    .expect("a usage row");
    assert_eq!(row.value, "498 of 512Mi");
    assert_eq!(row.tone, Some(StatusTone::Bad));
    assert_eq!(row.note.as_deref(), Some("request 256Mi"));
    let bar = row.bar.expect("a bar");
    assert!((bar.fill - 0.972).abs() < 0.001, "{}", bar.fill);
    assert_eq!(bar.marker, Some(0.5));
    assert_eq!(bar.tone, Some(StatusTone::Bad));

    let cpu = container_usage_row(&resource("cpu", None, Some("1")), Some(usage(310, 0)))
        .expect("a usage row");
    assert_eq!(cpu.value, "310m of 1 core");
    assert_eq!(cpu.tone, None);
    assert_eq!(cpu.note, None);
    assert_eq!(cpu.bar.expect("a bar").marker, None);
}

#[test]
fn container_usage_row_without_limit_has_no_bar() {
    let row = container_usage_row(&resource("cpu", Some("250m"), None), Some(usage(310, 0)))
        .expect("a usage row");
    assert_eq!(row.value, "310m used");
    assert_eq!(row.bar, None);
    assert_eq!(row.tone, None);
    assert_eq!(row.note.as_deref(), Some("request 250m · no limit"));
    let bare = container_usage_row(&resource("memory", None, None), Some(usage(0, 498)))
        .expect("a usage row");
    assert_eq!(bare.note.as_deref(), Some("no request · no limit"));
    let zero = container_usage_row(&resource("cpu", None, Some("0")), Some(usage(10, 0)))
        .expect("a usage row");
    assert_eq!(zero.bar, None);
}

#[test]
fn container_usage_row_keeps_the_text_without_usage_or_for_other_resources() {
    let memory = resource("memory", Some("1Mi"), Some("2Mi"));
    assert_eq!(container_usage_row(&memory, None), None);
    let storage = resource("ephemeral-storage", Some("1Gi"), None);
    assert_eq!(container_usage_row(&storage, Some(usage(1, 1))), None);
}

#[test]
fn container_display_order_groups_init_sidecar_main() {
    let containers = [
        container("main-a", ContainerKind::Main, running(), true),
        container("side", ContainerKind::Sidecar, running(), true),
        container("init", ContainerKind::Init, running(), true),
        container("main-b", ContainerKind::Main, running(), true),
        container("init-b", ContainerKind::Init, running(), true),
    ];
    // Init, Sidecar, Main; spec order inside each group.
    assert_eq!(container_display_order(&containers), [2, 4, 1, 0, 3]);
    assert!(container_display_order(&[]).is_empty());
}
