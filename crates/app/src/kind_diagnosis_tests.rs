use cluster::{
    ContainerProbes, ContainerSummary, DaemonSetSummary, DeploymentSummary, JobStatus, JobSummary,
    NodeScheduling, NodeStatus, NodeSystemInfo, ReadyCount, Termination,
};

use super::*;

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("valid timestamp")
}

fn condition(
    name: &str,
    is_true: bool,
    reason: Option<&str>,
    message: Option<&str>,
) -> WorkloadCondition {
    WorkloadCondition {
        name: name.to_owned(),
        is_true,
        reason: reason.map(str::to_owned),
        message: message.map(str::to_owned),
    }
}

fn deployment(desired: u32, ready: u32) -> DeploymentSummary {
    DeploymentSummary {
        namespace: "team-a".to_owned(),
        name: "api".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired,
        ready,
        up_to_date: ready,
        available: ready,
        strategy: String::new(),
        max_surge: None,
        max_unavailable: None,
        progress_deadline_seconds: 600,
        is_paused: false,
        revision: None,
        selector: Vec::new(),
        containers: Vec::new(),
        conditions: Vec::new(),
    }
}

fn daemon_set(desired: u32, current: u32, ready: u32) -> DaemonSetSummary {
    DaemonSetSummary {
        namespace: "kube-system".to_owned(),
        name: "agent".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired,
        current,
        ready,
        up_to_date: current,
        available: ready,
        misscheduled: 0,
        node_selector: Vec::new(),
        update_strategy: String::new(),
        selector: Vec::new(),
        containers: Vec::new(),
    }
}

fn job(status: JobStatus, failed: u32) -> JobSummary {
    JobSummary {
        namespace: "team-a".to_owned(),
        name: "migrate".to_owned(),
        created_at: None,
        labels: Vec::new(),
        status,
        completions: Some(1),
        parallelism: Some(1),
        succeeded: 0,
        failed,
        active: 0,
        backoff_limit: None,
        active_deadline_seconds: None,
        ttl_seconds_after_finished: None,
        started_at: None,
        finished_at: None,
        owner: None,
        conditions: Vec::new(),
        containers: Vec::new(),
    }
}

fn node(name: &str, readiness: NodeReadiness) -> NodeSummary {
    NodeSummary {
        name: name.to_owned(),
        status: NodeStatus {
            readiness,
            scheduling: NodeScheduling::Enabled,
        },
        roles: Vec::new(),
        taints: Vec::new(),
        kubelet_version: "v1.29.5".to_owned(),
        internal_ip: None,
        created_at: None,
        conditions: Vec::new(),
        addresses: Vec::new(),
        system: NodeSystemInfo::default(),
        resources: Vec::new(),
        labels: Vec::new(),
    }
}

/// A healthy pod, ready and running.
fn pod(name: &str, node: Option<&str>) -> PodSummary {
    PodSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: node.map(str::to_owned),
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        containers: Vec::new(),
    }
}

/// An evicted pod: a pod-level cause with a fixed text, `Evicted: {message}`.
fn evicted(name: &str, node: Option<&str>) -> PodSummary {
    let mut pod = pod(name, node);
    pod.status = PodStatus::Reason(StatusReason::Evicted);
    pod.ready = ReadyCount { ready: 0, total: 1 };
    pod.status_message = Some("The node was low on resource: memory.".to_owned());
    pod
}

fn terminated(exit_code: i32, reason: Option<StatusReason>) -> ContainerState {
    ContainerState::Terminated(Termination {
        reason,
        exit_code,
        signal: None,
        started_at: None,
        finished_at: None,
    })
}

fn failed_pod(
    name: &str,
    created: i64,
    exit_code: i32,
    reason: Option<StatusReason>,
) -> PodSummary {
    let mut pod = pod(name, None);
    pod.created_at = Some(at(created));
    pod.status = PodStatus::Reason(StatusReason::Error);
    pod.containers = vec![ContainerSummary {
        name: "main".to_owned(),
        image: "registry/app:1".to_owned(),
        kind: ContainerKind::Main,
        state: terminated(exit_code, reason),
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
    }];
    pod
}

fn run(
    object: KindObject,
    pods: Option<&[PodSummary]>,
    nodes: &[NodeSummary],
) -> Option<KindDiagnosis> {
    let refs: Option<Vec<&PodSummary>> = pods.map(|pods| pods.iter().collect());
    kind_diagnosis(
        &object,
        &DiagnosisInputs {
            pods: refs.as_deref(),
            nodes,
            now: at(1_000),
        },
    )
}

fn pod_key(name: &str) -> Option<ResourceKey> {
    Some(ResourceKey::Pod {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
    })
}

// ---- Deployments ----

#[test]
fn deployment_stalled() {
    let mut stalled = deployment(3, 2);
    stalled.conditions = vec![condition(
        "Progressing",
        false,
        Some("ProgressDeadlineExceeded"),
        None,
    )];
    let object = || KindObject::Deployment(stalled.clone());
    let bare = run(object(), Some(&[pod("api-1", None)]), &[]).expect("a box");
    assert_eq!(bare.tone, StatusTone::Bad);
    assert_eq!(bare.title, "ROLLOUT STALLED");
    assert_eq!(bare.text, "No progress for 600s.");
    assert_eq!(bare.pod, None);
    let mut once = failed_job("BackoffLimitExceeded", None);
    once.failed = 1;
    let single = run(KindObject::Job(once), None, &[]).expect("a box");
    assert_eq!(single.text, "1 attempt failed.");
    // The cause of the first unhealthy pod joins the text and becomes the link.
    let pods = [pod("api-1", None), evicted("api-2", None)];
    let explained = run(object(), Some(&pods), &[]).expect("a box");
    assert_eq!(
        explained.text,
        "No progress for 600s. Pod api-2: Evicted: The node was low on resource: memory."
    );
    assert_eq!(explained.pod, pod_key("api-2"));
    // The condition alone is enough, so it fires while the pods load.
    assert!(run(object(), None, &[]).is_some());
}

#[test]
fn deployment_replica_failure() {
    let mut failing = deployment(3, 0);
    failing.conditions = vec![condition(
        "ReplicaFailure",
        true,
        Some("FailedCreate"),
        Some("exceeded quota: compute"),
    )];
    let diagnosis = run(KindObject::Deployment(failing.clone()), None, &[]).expect("a box");
    assert_eq!(
        (diagnosis.tone, diagnosis.title.as_str()),
        (StatusTone::Bad, "REPLICA FAILURE")
    );
    assert_eq!(diagnosis.text, "FailedCreate: exceeded quota: compute");
    failing.conditions = vec![condition("ReplicaFailure", true, None, None)];
    let diagnosis = run(KindObject::Deployment(failing.clone()), None, &[]).expect("a box");
    assert_eq!(diagnosis.text, "The controller cannot create pods.");
    // A false condition is no failure.
    failing.conditions = vec![condition("ReplicaFailure", false, Some("Old"), None)];
    assert_eq!(run(KindObject::Deployment(failing), None, &[]), None);
}

#[test]
fn deployment_not_ready_names_pod() {
    let pods = [pod("api-1", None), evicted("api-2", None)];
    let partial = run(KindObject::Deployment(deployment(3, 1)), Some(&pods), &[]).expect("a box");
    assert_eq!(partial.tone, StatusTone::Warn);
    assert_eq!(partial.title, "2 OF 3 NOT READY");
    assert_eq!(
        partial.text,
        "Pod api-2: Evicted: The node was low on resource: memory."
    );
    assert_eq!(partial.pod, pod_key("api-2"));
    let none_ready =
        run(KindObject::Deployment(deployment(3, 0)), Some(&pods), &[]).expect("a box");
    assert_eq!(none_ready.tone, StatusTone::Bad);
    assert_eq!(none_ready.title, "3 OF 3 NOT READY");
}

#[test]
fn deployment_rollout_without_unhealthy_pod_has_no_box() {
    // Not ready yet, but every pod is healthy: a normal rollout.
    let pods = [pod("api-1", None), pod("api-2", None)];
    assert_eq!(
        run(KindObject::Deployment(deployment(3, 1)), Some(&pods), &[]),
        None
    );
    // Everything ready needs no box even with an unhealthy extra pod.
    let pods = [evicted("api-old", None)];
    assert_eq!(
        run(KindObject::Deployment(deployment(2, 2)), Some(&pods), &[]),
        None
    );
}

#[test]
fn paused_deployment_has_no_box() {
    let mut paused = deployment(3, 0);
    paused.is_paused = true;
    paused.conditions = vec![condition(
        "Progressing",
        false,
        Some("ProgressDeadlineExceeded"),
        None,
    )];
    let pods = [evicted("api-1", None)];
    assert_eq!(run(KindObject::Deployment(paused), Some(&pods), &[]), None);
    // Scaled to zero is no failure either.
    assert_eq!(
        run(KindObject::Deployment(deployment(0, 0)), Some(&pods), &[]),
        None
    );
}

// ---- DaemonSets ----

#[test]
fn daemon_set_node_missing() {
    let mut stranded = pod("agent-a", Some("wk-03"));
    stranded.ready = ReadyCount { ready: 0, total: 1 };
    let nodes = [
        node("wk-03", NodeReadiness::NotReady),
        node("wk-01", NodeReadiness::Ready),
    ];
    let diagnosis = run(
        KindObject::DaemonSet(daemon_set(2, 2, 1)),
        Some(&[pod("agent-b", Some("wk-01")), stranded]),
        &nodes,
    )
    .expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(diagnosis.title, "1 NODE MISSING");
    assert_eq!(
        diagnosis.text,
        "The pod on wk-03 is not ready because the node is NotReady."
    );
    assert_eq!(diagnosis.pod, pod_key("agent-a"));
}

#[test]
fn daemon_set_nodes_missing_plural() {
    let stranded = |name: &str, node: &str| {
        let mut pod = pod(name, Some(node));
        pod.ready = ReadyCount { ready: 0, total: 1 };
        pod
    };
    let nodes = [
        node("wk-02", NodeReadiness::NotReady),
        node("wk-03", NodeReadiness::NotReady),
    ];
    let diagnosis = run(
        KindObject::DaemonSet(daemon_set(3, 3, 1)),
        Some(&[stranded("agent-a", "wk-02"), stranded("agent-b", "wk-03")]),
        &nodes,
    )
    .expect("a box");
    assert_eq!(diagnosis.title, "2 NODES MISSING");
    assert_eq!(
        diagnosis.text,
        "Pods on wk-02 and 1 more nodes are not ready because their nodes are NotReady."
    );
}

#[test]
fn daemon_set_pod_not_ready() {
    let nodes = [node("wk-01", NodeReadiness::Ready)];
    let pods = [
        pod("agent-a", Some("wk-02")),
        evicted("agent-b", Some("wk-01")),
    ];
    let diagnosis = run(
        KindObject::DaemonSet(daemon_set(3, 3, 2)),
        Some(&pods),
        &nodes,
    )
    .expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(diagnosis.title, "1 OF 3 NOT READY");
    assert_eq!(
        diagnosis.text,
        "Pod agent-b on wk-01: Evicted: The node was low on resource: memory."
    );
    assert_eq!(diagnosis.pod, pod_key("agent-b"));
}

#[test]
fn daemon_set_without_pod_on_nodes() {
    let one = run(KindObject::DaemonSet(daemon_set(5, 4, 4)), Some(&[]), &[]).expect("a box");
    assert_eq!(one.title, "1 NODE WITHOUT A POD");
    assert_eq!(one.text, "5 nodes should run a pod; 4 do.");
    // Condition-only: it fires while the pods load.
    let many = run(KindObject::DaemonSet(daemon_set(5, 2, 2)), None, &[]).expect("a box");
    assert_eq!(many.title, "3 NODES WITHOUT A POD");
    let single = run(KindObject::DaemonSet(daemon_set(1, 0, 0)), None, &[]).expect("a box");
    assert_eq!(single.title, "1 NODE WITHOUT A POD");
    assert_eq!(single.text, "1 node should run a pod; 0 do.");
}

#[test]
fn daemon_set_misscheduled() {
    let mut set = daemon_set(3, 3, 3);
    set.misscheduled = 2;
    let diagnosis = run(KindObject::DaemonSet(set.clone()), None, &[]).expect("a box");
    assert_eq!(diagnosis.title, "MISSCHEDULED");
    assert_eq!(
        diagnosis.text,
        "2 pods run on nodes the DaemonSet no longer targets."
    );
    set.misscheduled = 1;
    let diagnosis = run(KindObject::DaemonSet(set), None, &[]).expect("a box");
    assert_eq!(
        diagnosis.text,
        "1 pod runs on a node the DaemonSet no longer targets."
    );
    // A healthy DaemonSet and one with nothing desired have no box.
    assert_eq!(
        run(KindObject::DaemonSet(daemon_set(3, 3, 3)), Some(&[]), &[]),
        None
    );
    assert_eq!(
        run(KindObject::DaemonSet(daemon_set(0, 0, 0)), Some(&[]), &[]),
        None
    );
}

// ---- Jobs ----

fn failed_job(reason: &str, message: Option<&str>) -> JobSummary {
    let mut job = job(JobStatus::Failed, 7);
    job.conditions = vec![condition("Failed", true, Some(reason), message)];
    job
}

#[test]
fn job_backoff_limit_with_exit_code() {
    let job = failed_job(
        "BackoffLimitExceeded",
        Some("Job has reached the specified backoff limit"),
    );
    let pods = [
        failed_pod("migrate-old", 100, 2, None),
        failed_pod("migrate-new", 200, 137, Some(StatusReason::OomKilled)),
    ];
    let diagnosis = run(KindObject::Job(job.clone()), Some(&pods), &[]).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(diagnosis.title, "BACKOFF LIMIT REACHED");
    assert_eq!(
        diagnosis.text,
        "7 attempts failed. Last pod exited with code 137 (OOMKilled)."
    );
    assert_eq!(diagnosis.pod, pod_key("migrate-new"));
    // Without pods the count still shows.
    let bare = run(KindObject::Job(job), None, &[]).expect("a box");
    assert_eq!(bare.text, "7 attempts failed.");
    assert_eq!(bare.pod, None);
    let mut once = failed_job("BackoffLimitExceeded", None);
    once.failed = 1;
    let single = run(KindObject::Job(once), None, &[]).expect("a box");
    assert_eq!(single.text, "1 attempt failed.");
}

#[test]
fn job_deadline_exceeded() {
    let mut job = failed_job(
        "DeadlineExceeded",
        Some("Job was active longer than specified deadline"),
    );
    job.active_deadline_seconds = Some(300);
    let diagnosis = run(KindObject::Job(job.clone()), None, &[]).expect("a box");
    assert_eq!(diagnosis.title, "DEADLINE EXCEEDED");
    assert_eq!(
        diagnosis.text,
        "The job ran longer than its active deadline of 300s."
    );
    job.active_deadline_seconds = None;
    let diagnosis = run(KindObject::Job(job), None, &[]).expect("a box");
    assert_eq!(
        diagnosis.text,
        "The job ran longer than its active deadline."
    );
}

#[test]
fn job_failed_other_reason() {
    let failed = failed_job("PodFailurePolicy", Some("Container main matched a rule"));
    let diagnosis = run(KindObject::Job(failed), None, &[]).expect("a box");
    assert_eq!(diagnosis.title, "JOB FAILED");
    assert_eq!(
        diagnosis.text,
        "PodFailurePolicy: Container main matched a rule"
    );
    // A Failing job (FailureTarget) reads the same way.
    let mut failing = job(JobStatus::Failing, 1);
    failing.conditions = vec![condition(
        "FailureTarget",
        true,
        Some("PodFailurePolicy"),
        None,
    )];
    let diagnosis = run(KindObject::Job(failing), None, &[]).expect("a box");
    assert_eq!(diagnosis.text, "PodFailurePolicy");
}

#[test]
fn job_retrying() {
    let mut retrying = job(JobStatus::Running, 2);
    let diagnosis = run(KindObject::Job(retrying.clone()), None, &[]).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(diagnosis.title, "2 FAILED ATTEMPTS");
    assert_eq!(
        diagnosis.text,
        "Retrying; the job fails after 7 failed attempts."
    );
    retrying.failed = 1;
    retrying.backoff_limit = Some(2);
    let diagnosis = run(KindObject::Job(retrying), None, &[]).expect("a box");
    assert_eq!(diagnosis.title, "1 FAILED ATTEMPT");
    assert_eq!(
        diagnosis.text,
        "Retrying; the job fails after 3 failed attempts."
    );
    // No failures, a finished job, and a suspended one have no box.
    assert_eq!(
        run(KindObject::Job(job(JobStatus::Running, 0)), None, &[]),
        None
    );
    assert_eq!(
        run(KindObject::Job(job(JobStatus::Complete, 3)), None, &[]),
        None
    );
    assert_eq!(
        run(KindObject::Job(job(JobStatus::Suspended, 3)), None, &[]),
        None
    );
}

#[test]
fn no_box_while_pods_load() {
    // The rules that need pods stay quiet until the pods list is ready.
    assert_eq!(
        run(KindObject::Deployment(deployment(3, 1)), None, &[]),
        None
    );
    let nodes = [node("wk-03", NodeReadiness::NotReady)];
    assert_eq!(
        run(KindObject::DaemonSet(daemon_set(3, 3, 1)), None, &nodes),
        None
    );
    // Kinds without a box never get one.
    assert_eq!(run(KindObject::Plain, Some(&[]), &[]), None);
}

/// A main container that runs but is not ready: a warming-up pod (a Warn cause).
fn warming_up(name: &str) -> PodSummary {
    let mut pod = pod(name, None);
    pod.ready = ReadyCount { ready: 0, total: 1 };
    pod.containers = vec![ContainerSummary {
        name: "main".to_owned(),
        image: "registry/app:1".to_owned(),
        kind: ContainerKind::Main,
        state: ContainerState::Running { started_at: None },
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
    }];
    pod
}

/// A crash-looping main container: a Bad cause that belongs to a container.
fn crash_looping(name: &str) -> PodSummary {
    let mut pod = warming_up(name);
    pod.status = PodStatus::Reason(StatusReason::CrashLoopBackOff);
    pod.containers[0].state = ContainerState::Waiting {
        reason: Some(StatusReason::CrashLoopBackOff),
        message: None,
    };
    pod
}

#[test]
fn warming_up_pod_is_a_normal_rollout() {
    // A pod that runs but is not ready yet has only a Warn cause. It must not raise a box: the
    // rollout is normal, and a long stall is ROLLOUT STALLED.
    let pods = [pod("api-1", None), warming_up("api-2")];
    assert_eq!(
        run(KindObject::Deployment(deployment(3, 1)), Some(&pods), &[]),
        None
    );
    let nodes = [node("wk-01", NodeReadiness::Ready)];
    let pods = [warming_up("agent-a")];
    assert_eq!(
        run(
            KindObject::DaemonSet(daemon_set(3, 3, 2)),
            Some(&pods),
            &nodes
        ),
        None
    );
}

#[test]
fn container_cause_names_the_pod_status() {
    // A container cause reads "is {status}"; a pod-level cause (evicted) does not.
    let pods = [warming_up("api-1"), crash_looping("api-2")];
    let diagnosis = run(KindObject::Deployment(deployment(3, 1)), Some(&pods), &[]).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(
        diagnosis.text,
        "Pod api-2 is CrashLoopBackOff: CrashLoopBackOff."
    );
    assert_eq!(diagnosis.pod, pod_key("api-2"));
}
