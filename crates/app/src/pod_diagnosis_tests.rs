use cluster::{
    ContainerProbes, ContainerResource, EventType, InvolvedObject, PodCondition, ProbeAction,
    ReadyCount,
};

use super::*;

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("valid timestamp")
}

fn container(name: &str, kind: ContainerKind, state: ContainerState) -> ContainerSummary {
    ContainerSummary {
        name: name.to_owned(),
        image: "registry/app:1".to_owned(),
        kind,
        state,
        is_ready: false,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: Vec::new(),
        probes: ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

fn main_container(name: &str, state: ContainerState) -> ContainerSummary {
    container(name, ContainerKind::Main, state)
}

fn running() -> ContainerState {
    ContainerState::Running { started_at: None }
}

fn ready(mut container: ContainerSummary) -> ContainerSummary {
    container.is_ready = true;
    container
}

fn waiting(reason: StatusReason, message: Option<&str>) -> ContainerState {
    ContainerState::Waiting {
        reason: Some(reason),
        message: message.map(str::to_owned),
    }
}

fn pod(status: PodStatus, containers: Vec<ContainerSummary>) -> PodSummary {
    let total = u32::try_from(containers.len()).unwrap_or_default();
    PodSummary {
        namespace: "shop".to_owned(),
        name: "api-0".to_owned(),
        status,
        ready: ReadyCount { ready: 0, total },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        containers,
        status_message: None,
    }
}

fn running_pod(containers: Vec<ContainerSummary>) -> PodSummary {
    pod(PodStatus::Reason(StatusReason::Running), containers)
}

fn termination(
    reason: Option<StatusReason>,
    exit_code: i32,
    started: Option<i64>,
    finished: Option<i64>,
) -> Termination {
    Termination {
        reason,
        exit_code,
        signal: None,
        started_at: started.map(at),
        finished_at: finished.map(at),
    }
}

fn memory(limit: Option<&str>) -> Vec<ContainerResource> {
    vec![ContainerResource {
        name: "memory".to_owned(),
        request: None,
        limit: limit.map(str::to_owned),
    }]
}

fn http_probe() -> ProbeSummary {
    ProbeSummary {
        action: ProbeAction::HttpGet {
            scheme: "HTTP".to_owned(),
            port: "8080".to_owned(),
            path: "/ready".to_owned(),
        },
        period_seconds: 5,
        failure_threshold: 3,
    }
}

fn unhealthy(container: &str, message: &str, count: u32, last_seen: i64) -> EventSummary {
    EventSummary {
        namespace: "shop".to_owned(),
        name: format!("api-0.{last_seen}"),
        event_type: EventType::Warning,
        reason: "Unhealthy".to_owned(),
        object: InvolvedObject {
            kind: "Pod".to_owned(),
            namespace: Some("shop".to_owned()),
            name: "api-0".to_owned(),
        },
        message: message.to_owned(),
        count,
        first_seen: Some(at(last_seen)),
        last_seen: Some(at(last_seen)),
        source: None,
        container: Some(container.to_owned()),
    }
}

fn diagnose(pod: &PodSummary) -> Option<PodDiagnosis> {
    pod_diagnosis(pod, None, at(10_000))
}

fn text_of(pod: &PodSummary) -> String {
    diagnose(pod).expect("a diagnosis").text
}

fn crash_looping(termination: Option<Termination>) -> ContainerSummary {
    let mut crashing = main_container(
        "api",
        waiting(
            StatusReason::CrashLoopBackOff,
            Some("back-off 5m0s restarting failed container=api"),
        ),
    );
    crashing.last_termination = termination;
    crashing.restart_count = 7;
    crashing
}

#[test]
fn diagnosis_p0_terminating() {
    let mut terminating = pod(
        PodStatus::Terminating,
        vec![main_container(
            "api",
            waiting(StatusReason::CrashLoopBackOff, None),
        )],
    );
    terminating.status_message = Some("gone".to_owned());
    assert_eq!(diagnose(&terminating), None);
    for reason in [StatusReason::Succeeded, StatusReason::Completed] {
        let done = pod(
            PodStatus::Reason(reason),
            vec![main_container("api", waiting(StatusReason::Error, None))],
        );
        assert_eq!(diagnose(&done), None);
    }
}

#[test]
fn diagnosis_p1_unschedulable() {
    let mut pending = pod(PodStatus::Reason(StatusReason::Pending), Vec::new());
    pending.conditions = vec![PodCondition {
        name: "PodScheduled".to_owned(),
        is_true: false,
        reason: Some("Unschedulable".to_owned()),
        message: Some("0/4 nodes are available: 4 Insufficient memory.".to_owned()),
    }];
    let diagnosis = diagnose(&pending).expect("a diagnosis");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(diagnosis.container, None);
    assert_eq!(
        diagnosis.text,
        "Cannot be scheduled: 0/4 nodes are available: 4 Insufficient memory."
    );
    pending.conditions[0].message = None;
    assert_eq!(text_of(&pending), "Cannot be scheduled.");
}

#[test]
fn diagnosis_p2_scheduling_gated() {
    let gated = pod(PodStatus::Reason(StatusReason::SchedulingGated), Vec::new());
    let diagnosis = diagnose(&gated).expect("a diagnosis");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(
        diagnosis.text,
        "Waiting for its scheduling gates to be removed."
    );
}

#[test]
fn diagnosis_p3_status_message() {
    let mut evicted = pod(
        PodStatus::Reason(StatusReason::Evicted),
        vec![main_container("api", waiting(StatusReason::Error, None))],
    );
    evicted.status_message = Some("The node was low on resource: memory.".to_owned());
    let diagnosis = diagnose(&evicted).expect("a diagnosis");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(diagnosis.container, None);
    assert_eq!(
        diagnosis.text,
        "Evicted: The node was low on resource: memory."
    );
}

#[test]
fn diagnosis_c1_image_pull() {
    let cases = [
        StatusReason::ImagePullBackOff,
        StatusReason::ErrImagePull,
        StatusReason::InvalidImageName,
        StatusReason::ErrImageNeverPull,
    ];
    for reason in cases {
        let with_message = running_pod(vec![main_container(
            "api",
            waiting(reason.clone(), Some("manifest unknown")),
        )]);
        assert_eq!(
            text_of(&with_message),
            "Cannot pull image registry/app:1: manifest unknown"
        );
        let without_message =
            running_pod(vec![main_container("api", waiting(reason.clone(), None))]);
        assert_eq!(
            text_of(&without_message),
            format!("Cannot pull image registry/app:1: {reason}")
        );
    }
}

#[test]
fn diagnosis_c2_oom_killed_loop() {
    let oom = termination(Some(StatusReason::OomKilled), 137, Some(0), Some(240));
    let mut crashing = crash_looping(Some(oom.clone()));
    crashing.resources = memory(Some("512Mi"));
    assert_eq!(
        text_of(&running_pod(vec![crashing.clone()])),
        "OOMKilled (exit 137) about 4m after each start: memory hits the 512Mi limit."
    );
    crashing.resources = memory(None);
    assert_eq!(
        text_of(&running_pod(vec![crashing.clone()])),
        "OOMKilled (exit 137) about 4m after each start. No memory limit is set."
    );
    crashing.last_termination = Some(termination(
        Some(StatusReason::OomKilled),
        137,
        None,
        Some(240),
    ));
    assert_eq!(
        text_of(&running_pod(vec![crashing])),
        "OOMKilled (exit 137) on each start. No memory limit is set."
    );
}

#[test]
fn diagnosis_c3_exit_loop() {
    let failed = termination(Some(StatusReason::Error), 1, Some(0), Some(30));
    let crashing = crash_looping(Some(failed));
    assert_eq!(
        text_of(&running_pod(vec![crashing])),
        "Exits with Error (exit 1) about 30s after each start. Restarted 7 times."
    );
    let bare = crash_looping(Some(termination(None, 2, None, None)));
    assert_eq!(
        text_of(&running_pod(vec![bare])),
        "Exits with code 2 on each start. Restarted 7 times."
    );
}

#[test]
fn diagnosis_c4_loop_without_last_termination() {
    let crashing = crash_looping(None);
    assert_eq!(
        text_of(&running_pod(vec![crashing])),
        "CrashLoopBackOff: back-off 5m0s restarting failed container=api"
    );
}

#[test]
fn diagnosis_c5_other_bad_waiting_reasons() {
    let create_error = running_pod(vec![main_container(
        "api",
        waiting(
            StatusReason::CreateContainerConfigError,
            Some("secret \"db\" not found"),
        ),
    )]);
    assert_eq!(
        text_of(&create_error),
        "CreateContainerConfigError: secret \"db\" not found"
    );
    let bare = running_pod(vec![main_container(
        "api",
        waiting(StatusReason::CreateContainerError, None),
    )]);
    assert_eq!(text_of(&bare), "CreateContainerError.");
    // A reason that is not bad is never a problem, and neither is unknown text.
    for reason in [
        StatusReason::ContainerCreating,
        StatusReason::PodInitializing,
        StatusReason::Other("Whatever".to_owned()),
    ] {
        let calm = running_pod(vec![main_container("api", waiting(reason, Some("x")))]);
        assert_eq!(diagnose(&calm), None);
    }
}

#[test]
fn diagnosis_c6_terminated_with_error() {
    let mut oom = main_container(
        "api",
        ContainerState::Terminated(termination(Some(StatusReason::OomKilled), 137, None, None)),
    );
    oom.resources = memory(Some("256Mi"));
    assert_eq!(
        text_of(&running_pod(vec![oom])),
        "Exited with OOMKilled (exit 137). Memory limit 256Mi."
    );
    let plain = main_container(
        "api",
        ContainerState::Terminated(termination(None, 3, None, None)),
    );
    assert_eq!(
        text_of(&running_pod(vec![plain])),
        "Exited with Error (exit 3)."
    );
    let clean = container(
        "init",
        ContainerKind::Init,
        ContainerState::Terminated(termination(Some(StatusReason::Completed), 0, None, None)),
    );
    assert_eq!(diagnose(&running_pod(vec![clean])), None);
}

#[test]
fn diagnosis_c7_startup_probe_not_passed() {
    let mut starting = main_container("api", running());
    starting.probes.startup = Some(http_probe());
    starting.is_started = Some(false);
    let pod = running_pod(vec![starting.clone()]);
    let diagnosis = diagnose(&pod).expect("a diagnosis");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(diagnosis.text, "Startup probe has not passed yet.");

    let events = [unhealthy(
        "api",
        "Startup probe failed: connection refused",
        4,
        9_940,
    )];
    let with_event = pod_diagnosis(&pod, Some(&events), at(10_000)).expect("a diagnosis");
    assert_eq!(
        with_event.text,
        "Startup probe has not passed yet. Startup probe failed: connection refused (×4, 1m ago)"
    );

    // The API reports a startup probe that has not reported yet as `started: null`, meaning false.
    starting.is_started = None;
    assert_eq!(
        text_of(&running_pod(vec![starting.clone()])),
        "Startup probe has not passed yet."
    );
    starting.is_started = Some(true);
    assert_eq!(diagnose(&running_pod(vec![ready(starting)])), None);
}

#[test]
fn diagnosis_c8_not_ready() {
    let mut idle = main_container("api", running());
    assert_eq!(
        diagnose(&running_pod(vec![idle.clone()])).map(|diagnosis| diagnosis.text),
        Some("Running but not ready.".to_owned())
    );

    idle.probes.readiness = Some(http_probe());
    let pod = running_pod(vec![idle.clone()]);
    assert_eq!(
        text_of(&pod),
        "Running but not ready. The readiness probe (HTTP GET :8080/ready · every 5s) has not passed."
    );

    let events = [unhealthy(
        "api",
        "Readiness probe failed: HTTP probe failed with statuscode: 503",
        12,
        9_700,
    )];
    let with_event = pod_diagnosis(&pod, Some(&events), at(10_000)).expect("a diagnosis");
    assert_eq!(with_event.tone, StatusTone::Warn);
    assert_eq!(
        with_event.text,
        "Running but not ready. Readiness probe failed: HTTP probe failed with statuscode: 503 (×12, 5m ago)"
    );

    // An init container that runs is not expected to be ready.
    let init = container("init", ContainerKind::Init, running());
    assert_eq!(diagnose(&running_pod(vec![init])), None);
}

#[test]
fn diagnosis_prefers_bad_over_warn_then_container_order() {
    let not_ready = main_container("first", running());
    let image = main_container("second", waiting(StatusReason::ErrImagePull, None));
    let crash = crash_looping(None);
    let pod = running_pod(vec![not_ready, image, crash]);
    let diagnosis = diagnose(&pod).expect("a diagnosis");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(diagnosis.container.as_deref(), Some("second"));

    let warn_only = running_pod(vec![
        main_container("one", running()),
        main_container("two", running()),
    ]);
    assert_eq!(
        diagnose(&warn_only).and_then(|diagnosis| diagnosis.container),
        Some("one".to_owned())
    );
}

#[test]
fn diagnosis_suffix_counts_other_problem_containers() {
    let healthy = ready(main_container("ok", running()));
    let broken = |name: &str| main_container(name, waiting(StatusReason::ErrImagePull, None));
    let suffix_of = |containers: Vec<ContainerSummary>| {
        let text = text_of(&running_pod(containers));
        text.strip_prefix("Cannot pull image registry/app:1: ErrImagePull")
            .map(str::to_owned)
            .expect("the image text")
    };
    assert_eq!(
        suffix_of(vec![broken("a"), healthy.clone()]),
        ". Other containers are healthy."
    );
    assert_eq!(suffix_of(vec![broken("a")]), "");
    assert_eq!(
        suffix_of(vec![broken("a"), broken("b"), healthy.clone()]),
        ". 1 other container also has a problem."
    );
    assert_eq!(
        suffix_of(vec![broken("a"), broken("b"), broken("c")]),
        ". 2 other containers also have problems."
    );
}

#[test]
fn healthy_and_completed_pods_have_no_diagnosis() {
    let healthy = running_pod(vec![ready(main_container("api", running()))]);
    assert_eq!(diagnose(&healthy), None);
    let creating = pod(
        PodStatus::Reason(StatusReason::Pending),
        vec![main_container(
            "api",
            waiting(StatusReason::ContainerCreating, None),
        )],
    );
    assert_eq!(diagnose(&creating), None);
    let unreported = running_pod(vec![main_container("api", ContainerState::NotReported)]);
    assert_eq!(diagnose(&unreported), None);
}

fn probed(kind: ProbeKind) -> ContainerSummary {
    let mut probed = main_container("api", running());
    match kind {
        ProbeKind::Liveness => probed.probes.liveness = Some(http_probe()),
        ProbeKind::Readiness => probed.probes.readiness = Some(http_probe()),
        ProbeKind::Startup => probed.probes.startup = Some(http_probe()),
    }
    probed
}

fn result(pod: &PodSummary, kind: ProbeKind, events: Option<&[EventSummary]>) -> ProbeResult {
    probe_result(pod, &pod.containers[0], kind, events)
}

#[test]
fn probe_result_table() {
    let no_events: &[EventSummary] = &[];
    let one = |container: ContainerSummary| running_pod(vec![container]);

    // Not set.
    let bare = one(main_container("api", running()));
    assert_eq!(
        result(&bare, ProbeKind::Liveness, None),
        ProbeResult::NotSet
    );

    // Inactive: terminating pod, or a container that is not running.
    let mut live = probed(ProbeKind::Readiness);
    live.is_ready = true;
    let terminating = pod(PodStatus::Terminating, vec![live.clone()]);
    assert_eq!(
        result(&terminating, ProbeKind::Readiness, None),
        ProbeResult::Inactive
    );
    let mut stopped = probed(ProbeKind::Liveness);
    stopped.state = waiting(StatusReason::CrashLoopBackOff, None);
    assert_eq!(
        result(&one(stopped), ProbeKind::Liveness, Some(no_events)),
        ProbeResult::Inactive
    );

    // Waiting for startup: liveness and readiness only.
    let mut starting = probed(ProbeKind::Readiness);
    starting.probes.liveness = Some(http_probe());
    starting.probes.startup = Some(http_probe());
    starting.is_started = Some(false);
    let starting = one(starting);
    assert_eq!(
        result(&starting, ProbeKind::Readiness, Some(no_events)),
        ProbeResult::WaitingForStartup
    );
    assert_eq!(
        result(&starting, ProbeKind::Liveness, Some(no_events)),
        ProbeResult::WaitingForStartup
    );
    assert_eq!(
        result(&starting, ProbeKind::Startup, Some(no_events)),
        ProbeResult::Pending
    );
    // The API reports `started: null` while the startup probe has not passed.
    let mut unreported = starting.containers[0].clone();
    unreported.is_started = None;
    assert_eq!(
        result(&one(unreported), ProbeKind::Readiness, Some(no_events)),
        ProbeResult::WaitingForStartup
    );

    // Readiness follows the ready flag; failures come from the newest event.
    assert_eq!(
        result(&one(live.clone()), ProbeKind::Readiness, None),
        ProbeResult::Passing
    );
    live.is_ready = false;
    let not_ready = one(live);
    assert_eq!(
        result(&not_ready, ProbeKind::Readiness, None),
        ProbeResult::Failing { failures: 0 }
    );
    let events = [unhealthy("api", "Readiness probe failed: x", 3, 100)];
    assert_eq!(
        result(&not_ready, ProbeKind::Readiness, Some(&events)),
        ProbeResult::Failing { failures: 3 }
    );

    // Startup: passed, failing with an event, else pending.
    let mut startup = probed(ProbeKind::Startup);
    startup.is_started = Some(true);
    assert_eq!(
        result(&one(startup.clone()), ProbeKind::Startup, None),
        ProbeResult::Passed
    );
    startup.is_started = Some(false);
    let startup_pod = one(startup);
    let startup_events = [unhealthy("api", "Startup probe failed: x", 5, 100)];
    assert_eq!(
        result(&startup_pod, ProbeKind::Startup, Some(&startup_events)),
        ProbeResult::Failing { failures: 5 }
    );
    assert_eq!(
        result(&startup_pod, ProbeKind::Startup, Some(no_events)),
        ProbeResult::Pending
    );

    // Liveness: no data until the events load, then failing or passing.
    let liveness = one(probed(ProbeKind::Liveness));
    assert_eq!(
        result(&liveness, ProbeKind::Liveness, None),
        ProbeResult::NoData
    );
    assert_eq!(
        result(&liveness, ProbeKind::Liveness, Some(no_events)),
        ProbeResult::Passing
    );
    let liveness_events = [unhealthy("api", "Liveness probe failed: x", 2, 100)];
    assert_eq!(
        result(&liveness, ProbeKind::Liveness, Some(&liveness_events)),
        ProbeResult::Failing { failures: 2 }
    );
}

#[test]
fn probe_failures_match_reason_container_kind_and_run() {
    let mut api = probed(ProbeKind::Readiness);
    api.state = ContainerState::Running {
        started_at: Some(at(1_000)),
    };
    let pod = running_pod(vec![api]);
    let readiness = |events: &[EventSummary]| result(&pod, ProbeKind::Readiness, Some(events));
    let matching = unhealthy("api", "Readiness probe failed: x", 3, 2_000);

    assert_eq!(
        readiness(std::slice::from_ref(&matching)),
        ProbeResult::Failing { failures: 3 }
    );

    let mut other_reason = matching.clone();
    other_reason.reason = "BackOff".to_owned();
    let other_container = unhealthy("sidecar", "Readiness probe failed: x", 9, 2_000);
    let other_kind = unhealthy("api", "Liveness probe failed: x", 9, 2_000);
    let before_start = unhealthy("api", "Readiness probe failed: x", 9, 500);
    for event in [other_reason, other_container, other_kind, before_start] {
        assert_eq!(
            readiness(&[event]),
            ProbeResult::Failing { failures: 0 },
            "a non-matching event must not count"
        );
    }

    // `errored` wording matches too.
    let errored = unhealthy("api", "Readiness probe errored: timeout", 4, 2_000);
    assert_eq!(readiness(&[errored]), ProbeResult::Failing { failures: 4 });
}

#[test]
fn probe_failures_use_newest_event_count() {
    let pod = running_pod(vec![probed(ProbeKind::Readiness)]);
    let older = unhealthy("api", "Readiness probe failed: x", 7, 1_000);
    let newer = unhealthy("api", "Readiness probe failed: x", 3, 2_000);
    for events in [[older.clone(), newer.clone()], [newer, older]] {
        assert_eq!(
            result(&pod, ProbeKind::Readiness, Some(&events)),
            ProbeResult::Failing { failures: 3 }
        );
    }
}

#[test]
fn next_retry_from_back_off_message() {
    let loop_with = |message: Option<&str>, finished: Option<i64>| {
        let mut crashing = main_container("api", waiting(StatusReason::CrashLoopBackOff, message));
        crashing.last_termination = Some(termination(None, 1, None, finished));
        crashing
    };
    let remaining = |container: &ContainerSummary| {
        next_retry(container, at(1_100)).map(|duration| duration.as_secs())
    };
    let message = |duration: &str| format!("back-off {duration} restarting failed container=api");

    assert_eq!(
        remaining(&loop_with(Some(&message("5m0s")), Some(1_000))),
        Some(200)
    );
    assert_eq!(
        remaining(&loop_with(Some(&message("40s")), Some(1_090))),
        Some(30)
    );
    assert_eq!(
        remaining(&loop_with(Some(&message("1h0m0s")), Some(1_000))),
        Some(3_500)
    );
    // The retry time has passed.
    assert_eq!(
        remaining(&loop_with(Some(&message("40s")), Some(1_000))),
        None
    );
    // Text the parser does not know.
    assert_eq!(
        remaining(&loop_with(Some(&message("soon")), Some(1_000))),
        None
    );
    assert_eq!(
        remaining(&loop_with(Some(&message("1.5s")), Some(1_000))),
        None
    );
    assert_eq!(remaining(&loop_with(Some("restarting"), Some(1_000))), None);
    assert_eq!(remaining(&loop_with(None, Some(1_000))), None);
    // Without the end of the last run there is nothing to measure from.
    assert_eq!(remaining(&loop_with(Some(&message("5m0s")), None)), None);
    // Only CrashLoopBackOff has a retry.
    let mut pulling = loop_with(Some(&message("5m0s")), Some(1_000));
    pulling.state = waiting(StatusReason::ImagePullBackOff, Some(&message("5m0s")));
    assert_eq!(remaining(&pulling), None);
}

#[test]
fn diagnosis_suffix_follows_a_sentence_end() {
    // A readiness event ends with `)`, so a full stop is added before the suffix; a text that
    // already ends a sentence gets none.
    let mut idle = main_container("api", running());
    idle.probes.readiness = Some(http_probe());
    let pod = running_pod(vec![idle, ready(main_container("ok", running()))]);
    let events = [unhealthy("api", "Readiness probe failed: 503", 2, 9_940)];
    let diagnosis = pod_diagnosis(&pod, Some(&events), at(10_000)).expect("a diagnosis");
    assert_eq!(
        diagnosis.text,
        "Running but not ready. Readiness probe failed: 503 (×2, 1m ago). Other containers are healthy."
    );
    let flat = running_pod(vec![
        main_container("api", running()),
        ready(main_container("ok", running())),
    ]);
    assert_eq!(
        text_of(&flat),
        "Running but not ready. Other containers are healthy."
    );
}

#[test]
fn probe_failures_with_equal_times_keep_list_order() {
    let pod = running_pod(vec![probed(ProbeKind::Readiness)]);
    let first = unhealthy("api", "Readiness probe failed: x", 7, 2_000);
    let second = unhealthy("api", "Readiness probe failed: x", 3, 2_000);
    assert_eq!(
        result(&pod, ProbeKind::Readiness, Some(&[first, second])),
        ProbeResult::Failing { failures: 7 }
    );
}
