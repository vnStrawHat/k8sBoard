use cluster::{
    ContainerKind, ContainerProbes, ContainerSummary, PodStatus, ReadyCount, Termination,
};

use super::*;

fn line(text: &str) -> LogLine {
    LogLine {
        timestamp: None,
        text: text.to_owned(),
    }
}

fn container(name: &str, state: ContainerState, with_run: bool) -> ContainerSummary {
    ContainerSummary {
        name: name.to_owned(),
        image: "app:1".to_owned(),
        kind: ContainerKind::Main,
        state,
        is_ready: false,
        restart_count: 7,
        last_termination: with_run.then_some(Termination {
            reason: None,
            exit_code: 1,
            signal: None,
            started_at: None,
            finished_at: None,
        }),
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: Vec::new(),
        probes: ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
        terminal: cluster::ContainerTerminal::None,
    }
}

fn crash_loop() -> ContainerState {
    ContainerState::Waiting {
        reason: Some(StatusReason::CrashLoopBackOff),
        message: None,
    }
}

fn pod(containers: Vec<ContainerSummary>) -> PodSummary {
    PodSummary {
        namespace: "shop".to_owned(),
        name: "api-0".to_owned(),
        status: PodStatus::Reason(StatusReason::CrashLoopBackOff),
        ready: ReadyCount { ready: 0, total: 1 },
        restarts: 7,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        containers,
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
        is_finished: false,
    }
}

#[test]
fn the_key_names_the_crash_looping_container_and_its_restart_count() {
    let pod = pod(vec![
        container(
            "sidecar",
            ContainerState::Running { started_at: None },
            true,
        ),
        container("app", crash_loop(), true),
    ]);
    assert_eq!(
        last_log_key(&pod),
        Some(LastLogKey {
            namespace: "shop".to_owned(),
            pod: "api-0".to_owned(),
            container: "app".to_owned(),
            restart_count: 7,
        })
    );
}

#[test]
fn a_container_that_never_ran_or_is_not_crash_looping_has_no_key() {
    assert_eq!(
        last_log_key(&pod(vec![container("app", crash_loop(), false)])),
        None
    );
    let running = ContainerState::Running { started_at: None };
    assert_eq!(
        last_log_key(&pod(vec![container("app", running, true)])),
        None
    );
}

#[test]
fn the_newest_non_empty_line_is_quoted_without_control_characters() {
    let lines = [line("old"), line("panic: \x1b[31mboom\x07"), line("  ")];
    assert_eq!(last_line(&lines), Some("panic: [31mboom".to_owned()));
    assert_eq!(last_line(&[line("")]), None);
    assert_eq!(last_line(&[]), None);
}

#[test]
fn a_long_line_is_cut() {
    let long = "x".repeat(250);
    assert_eq!(
        last_line(&[line(&long)]),
        Some(format!("{}…", "x".repeat(200)))
    );
}
