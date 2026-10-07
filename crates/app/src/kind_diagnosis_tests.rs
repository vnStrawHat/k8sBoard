use cluster::{
    ClaimRef, ContainerProbes, ContainerSummary, DaemonSetSummary, DeploymentSummary, JobStatus,
    JobSummary, NodeScheduling, NodeStatus, NodeSystemInfo, ReadyCount, Termination, VolumeBackend,
};

use super::*;
use crate::kind_join::EndpointCounts;

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
        last_transition: None,
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
        generation: 1,
        observed_generation: 1,
        revision: None,
        selector: Vec::new(),
        containers: Vec::new(),
        conditions: Vec::new(),
        template_change: None,
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
        node_affinity_keys: Vec::new(),
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
        is_finished: false,
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
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
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
        terminal: cluster::ContainerTerminal::None,
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
            service: None,
            bindings: None,
            tls_secrets: None,
            events: None,
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
    assert_eq!(bare.link, None);
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
    assert_eq!(explained.link, pod_key("api-2"));
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
    assert_eq!(partial.link, pod_key("api-2"));
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
    assert_eq!(diagnosis.link, pod_key("agent-a"));
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
    assert_eq!(diagnosis.link, pod_key("agent-b"));
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
    assert_eq!(diagnosis.link, pod_key("migrate-new"));
    // Without pods the count still shows.
    let bare = run(KindObject::Job(job), None, &[]).expect("a box");
    assert_eq!(bare.text, "7 attempts failed.");
    assert_eq!(bare.link, None);
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
        terminal: cluster::ContainerTerminal::None,
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
    assert_eq!(diagnosis.link, pod_key("api-2"));
}

// ---- Rollout in progress ----

fn progress(deployment: DeploymentSummary, pods: Option<&[PodSummary]>) -> Option<KindDiagnosis> {
    let refs: Option<Vec<&PodSummary>> = pods.map(|pods| pods.iter().collect());
    rollout_progress(
        &deployment,
        &DiagnosisInputs {
            pods: refs.as_deref(),
            nodes: &[],
            service: None,
            bindings: None,
            tls_secrets: None,
            events: None,
            now: at(1_000),
        },
    )
}

fn image_pull_failing(name: &str, created: i64) -> PodSummary {
    let mut pod = warming_up(name);
    pod.status = PodStatus::Reason(StatusReason::ErrImagePull);
    pod.created_at = Some(at(created));
    pod.containers[0].state = ContainerState::Waiting {
        reason: Some(StatusReason::ErrImagePull),
        message: None,
    };
    pod
}

/// A broken release behind old pods: every old pod is ready, so the Deployment is green.
fn broken_release() -> DeploymentSummary {
    let mut broken = deployment(3, 3);
    broken.up_to_date = 1;
    broken.template_change = Some(cluster::FieldWriter {
        manager: "k8sboard".to_owned(),
        at: at(900),
    });
    broken
}

#[test]
fn a_rollout_with_a_failing_new_pod_names_it_while_old_pods_keep_the_deployment_green() {
    let mut old = pod("api-old", None);
    old.created_at = Some(at(100));
    let pods = [old, image_pull_failing("api-new", 950)];
    let diagnosis = progress(broken_release(), Some(&pods)).expect("a progress box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(diagnosis.title, "ROLLOUT IN PROGRESS");
    assert!(
        diagnosis.text.starts_with(
            "1 of 3 pods run the new template; 3 of 3 are available. New pod api-new is ErrImagePull:"
        ),
        "{}",
        diagnosis.text
    );
    assert_eq!(diagnosis.link, pod_key("api-new"));
    // The problem rules of the Deployment have nothing to say: it is green.
    let object = KindObject::Deployment(broken_release());
    assert_eq!(run(object, Some(&pods), &[]), None);
}

#[test]
fn a_failing_pod_older_than_the_template_change_is_not_a_new_pod() {
    let pods = [image_pull_failing("api-old", 100)];
    let diagnosis = progress(broken_release(), Some(&pods)).expect("a progress box");
    assert_eq!(diagnosis.link, None);
    assert!(!diagnosis.text.contains("New pod"), "{}", diagnosis.text);
}

#[test]
fn a_warming_up_pod_is_not_named_and_no_pods_list_is_needed() {
    let pods = [pod("api-1", None), warming_up("api-2")];
    let diagnosis = progress(deployment(3, 1), Some(&pods)).expect("a progress box");
    assert_eq!(diagnosis.link, None);
    let diagnosis = progress(broken_release(), None).expect("a progress box");
    assert_eq!(diagnosis.title, "ROLLOUT IN PROGRESS");
}

#[test]
fn a_finished_scaled_to_zero_or_paused_rollout_raises_no_progress_box() {
    assert_eq!(progress(deployment(3, 3), Some(&[])), None);
    assert_eq!(progress(deployment(0, 0), Some(&[])), None);
    let mut paused = broken_release();
    paused.is_paused = true;
    assert_eq!(progress(paused, Some(&[])), None);
}

// ---- Services ----

fn service() -> ServiceSummary {
    ServiceSummary {
        namespace: "team-a".to_owned(),
        name: "api".to_owned(),
        created_at: None,
        labels: Vec::new(),
        service_type: "ClusterIP".to_owned(),
        cluster_ips: Vec::new(),
        is_headless: false,
        external_addresses: Vec::new(),
        ports: Vec::new(),
        selector: vec!["app=api".to_owned(), "tier=web".to_owned()],
    }
}

fn run_service(health: ServiceHealth, pods: &[PodSummary]) -> Option<KindDiagnosis> {
    let refs: Vec<&PodSummary> = pods.iter().collect();
    kind_diagnosis(
        &KindObject::Service(service()),
        &DiagnosisInputs {
            pods: Some(&refs),
            nodes: &[],
            service: Some(health),
            bindings: None,
            tls_secrets: None,
            events: None,
            now: at(1_000),
        },
    )
}

fn endpoint_health(ready: usize, total: usize) -> ServiceHealth {
    ServiceHealth {
        matching_pods: Some(total.max(1)),
        endpoints: Some(EndpointCounts { ready, total }),
    }
}

#[test]
fn service_no_matching_pods() {
    let health = ServiceHealth {
        matching_pods: Some(0),
        endpoints: None,
    };
    let diagnosis = run_service(health, &[]).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(diagnosis.title, "NO MATCHING PODS");
    assert_eq!(
        diagnosis.text,
        "No pod in team-a has the labels app=api, tier=web."
    );
    assert_eq!(diagnosis.link, None);
}

#[test]
fn service_no_ready_endpoints() {
    let pods = [pod("api-1", None), crash_looping("api-2")];
    let diagnosis = run_service(endpoint_health(0, 3), &pods).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(diagnosis.title, "NO READY ENDPOINTS");
    assert_eq!(
        diagnosis.text,
        "3 endpoints, none ready. Pod api-2: CrashLoopBackOff."
    );
    assert_eq!(diagnosis.link, pod_key("api-2"));
}

#[test]
fn service_no_ready_endpoints_without_an_unhealthy_pod_has_no_pod_link() {
    let diagnosis = run_service(endpoint_health(0, 1), &[pod("api-1", None)]).expect("a box");
    assert_eq!(diagnosis.text, "1 endpoint, none ready.");
    assert_eq!(diagnosis.link, None);
}

#[test]
fn service_with_some_ready_endpoints_has_no_box() {
    assert_eq!(run_service(endpoint_health(1, 3), &[]), None);
}

#[test]
fn service_without_endpoints_has_no_box() {
    // An empty list is the status column's "No endpoints", not a WHY box.
    assert_eq!(run_service(endpoint_health(0, 0), &[]), None);
}

#[test]
fn service_with_unknown_health_has_no_box() {
    assert_eq!(run_service(ServiceHealth::default(), &[]), None);
}

#[test]
fn service_no_ready_endpoints_waits_for_the_pods() {
    let health = endpoint_health(0, 3);
    let diagnosis = kind_diagnosis(
        &KindObject::Service(service()),
        &DiagnosisInputs {
            pods: None,
            nodes: &[],
            service: Some(health),
            bindings: None,
            tls_secrets: None,
            events: None,
            now: at(1_000),
        },
    );
    assert_eq!(diagnosis, None);
}

// ---- PodDisruptionBudgets ----

fn budget(expected: u32, healthy: u32, allowed: u32) -> PodDisruptionBudgetSummary {
    PodDisruptionBudgetSummary {
        namespace: "team-a".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        min_available: Some("2".to_owned()),
        max_unavailable: None,
        selector: None,
        current_healthy: healthy,
        desired_healthy: 2,
        expected_pods: expected,
        disruptions_allowed: allowed,
        unhealthy_pod_eviction_policy: None,
        conditions: Vec::new(),
        is_status_stale: false,
    }
}

/// The WHY box of a budget with no pods loaded: the rules read the budget only.
fn budget_diagnosis(budget: PodDisruptionBudgetSummary) -> Option<KindDiagnosis> {
    kind_diagnosis(
        &KindObject::PodDisruptionBudget(budget),
        &DiagnosisInputs {
            pods: None,
            nodes: &[],
            service: None,
            bindings: None,
            tls_secrets: None,
            events: None,
            now: at(1_000),
        },
    )
}

#[test]
fn pdb_blocks_drain_with_unhealthy_pods() {
    let diagnosis = budget_diagnosis(budget(3, 2, 0)).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(diagnosis.title, "BLOCKS DRAIN");
    assert_eq!(
        diagnosis.text,
        "Only 2 of 3 pods are healthy and minAvailable is 2. Draining any node that runs these \
         pods will wait."
    );
    assert_eq!(diagnosis.link, None);
}

#[test]
fn pdb_blocks_drain_without_room() {
    let mut full = budget(3, 3, 0);
    full.min_available = None;
    full.max_unavailable = Some("0".to_owned());
    let diagnosis = budget_diagnosis(full).expect("a box");
    assert_eq!(
        diagnosis.text,
        "maxUnavailable is 0 and all 3 pods must stay up, so no pod can be evicted. Draining \
         any node that runs these pods will wait until the budget changes."
    );
}

#[test]
fn pdb_allowed_has_no_box() {
    assert_eq!(budget_diagnosis(budget(3, 3, 1)), None);
}

#[test]
fn pdb_no_pods_has_no_box() {
    assert_eq!(budget_diagnosis(budget(0, 0, 0)), None);
}

#[test]
fn pdb_sync_failed_blocks_drain() {
    let mut failed = budget(0, 0, 0);
    failed.conditions = vec![condition(
        "DisruptionAllowed",
        false,
        Some("SyncFailed"),
        Some("found no controller ref"),
    )];
    let diagnosis = budget_diagnosis(failed).expect("a box");
    assert_eq!(
        diagnosis.text,
        "The disruption controller cannot compute this budget (found no controller ref). \
         Evictions of the selected pods are refused, so draining a node that runs them will wait."
    );
}

#[test]
fn pdb_blocks_drain_with_a_single_pod() {
    let diagnosis = budget_diagnosis(budget(1, 1, 0)).expect("a box");
    assert_eq!(
        diagnosis.text,
        "minAvailable is 2 and the only pod must stay up, so it cannot be evicted. Draining the \
         node that runs this pod will wait until the budget changes."
    );
}

// ---- HorizontalPodAutoscalers ----

fn autoscaler(conditions: Vec<WorkloadCondition>) -> HorizontalPodAutoscalerSummary {
    HorizontalPodAutoscalerSummary {
        namespace: "team-a".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        target: cluster::ControllerRef {
            kind: "Deployment".to_owned(),
            name: "web".to_owned(),
        },
        min_replicas: 2,
        max_replicas: 10,
        current_replicas: 10,
        desired_replicas: 10,
        metrics: Vec::new(),
        conditions,
        last_scaled_at: None,
    }
}

fn autoscaler_diagnosis(hpa: HorizontalPodAutoscalerSummary) -> Option<KindDiagnosis> {
    kind_diagnosis(
        &KindObject::HorizontalPodAutoscaler(hpa),
        &DiagnosisInputs {
            pods: None,
            nodes: &[],
            service: None,
            bindings: None,
            tls_secrets: None,
            events: None,
            now: at(1_000),
        },
    )
}

/// The title of the box for one false condition.
fn title_for(name: &str, reason: &str) -> Option<String> {
    autoscaler_diagnosis(autoscaler(vec![condition(
        name,
        false,
        Some(reason),
        Some("detail"),
    )]))
    .map(|diagnosis| diagnosis.title)
}

#[test]
fn hpa_metrics_unavailable_by_reason() {
    for reason in [
        "FailedGetResourceMetric",
        "FailedGetExternalMetric",
        "InvalidMetricSourceType",
    ] {
        assert_eq!(
            title_for("ScalingActive", reason).as_deref(),
            Some("METRICS UNAVAILABLE"),
            "{reason}"
        );
    }
    let diagnosis = autoscaler_diagnosis(autoscaler(vec![condition(
        "ScalingActive",
        false,
        Some("FailedGetResourceMetric"),
        Some("no metrics returned"),
    )]))
    .expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(
        diagnosis.text,
        "FailedGetResourceMetric: no metrics returned"
    );
}

#[test]
fn hpa_cannot_scale_by_reason() {
    for reason in ["FailedGetScale", "FailedUpdateScale"] {
        assert_eq!(
            title_for("AbleToScale", reason).as_deref(),
            Some("CANNOT SCALE"),
            "{reason}"
        );
        // The scale reasons win over the inactive-condition rule.
        assert_eq!(
            title_for("ScalingActive", reason).as_deref(),
            Some("CANNOT SCALE"),
            "{reason}"
        );
    }
    assert_eq!(
        title_for("AbleToScale", "SomethingElse").as_deref(),
        Some("CANNOT SCALE")
    );
}

#[test]
fn hpa_scaling_inactive_other_reason() {
    assert_eq!(
        title_for("ScalingActive", "SomethingElse").as_deref(),
        Some("SCALING INACTIVE")
    );
}

#[test]
fn hpa_scaling_disabled_has_no_box() {
    assert_eq!(title_for("ScalingActive", "ScalingDisabled"), None);
}

#[test]
fn hpa_at_max_replicas() {
    let mut capped = autoscaler(vec![condition(
        "ScalingLimited",
        true,
        Some("TooManyReplicas"),
        None,
    )]);
    capped.metrics = vec![cluster::HpaMetric {
        name: "cpu".to_owned(),
        source: cluster::MetricSource::Resource,
        target: cluster::MetricValue::Utilization(70),
        current: Some(cluster::MetricValue::Utilization(92)),
    }];
    let diagnosis = autoscaler_diagnosis(capped).expect("a box");
    assert_eq!(diagnosis.title, "AT MAX REPLICAS");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(
        diagnosis.text,
        "Running 10 of max 10 replicas and the metrics ask for more. cpu 92% / 70% is above \
         target. Raise maxReplicas or reduce the load."
    );
}

#[test]
fn hpa_scaling_normally_has_no_box() {
    let healthy = autoscaler(vec![
        condition("AbleToScale", true, Some("ReadyForNewScale"), None),
        condition("ScalingActive", true, Some("ValidMetricFound"), None),
        condition("ScalingLimited", false, Some("DesiredWithinRange"), None),
    ]);
    assert_eq!(autoscaler_diagnosis(healthy), None);
}

// ---- ResourceQuotas ----

#[test]
fn quota_at_limit() {
    let quota = |used: &str| ResourceQuotaSummary {
        namespace: "team-a".to_owned(),
        name: "compute".to_owned(),
        created_at: None,
        labels: Vec::new(),
        items: vec![
            cluster::QuotaItem {
                resource: "requests.cpu".to_owned(),
                hard: "4".to_owned(),
                used: Some("1".to_owned()),
            },
            cluster::QuotaItem {
                resource: "pods".to_owned(),
                hard: "10".to_owned(),
                used: Some(used.to_owned()),
            },
        ],
        scopes: Vec::new(),
    };
    let diagnose = |quota| {
        kind_diagnosis(
            &KindObject::ResourceQuota(quota),
            &DiagnosisInputs {
                pods: None,
                nodes: &[],
                service: None,
                bindings: None,
                tls_secrets: None,
                events: None,
                now: at(1_000),
            },
        )
    };
    let diagnosis = diagnose(quota("10")).expect("a box");
    assert_eq!(diagnosis.title, "AT QUOTA");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(
        diagnosis.text,
        "pods is at its limit (10 / 10). New objects that need it are rejected; see Blocked \
         creations."
    );
    assert_eq!(diagnose(quota("9")), None);
}

#[test]
fn quota_status_and_box_name_the_same_item() {
    let item = |resource: &str, hard: &str, used: &str| cluster::QuotaItem {
        resource: resource.to_owned(),
        hard: hard.to_owned(),
        used: Some(used.to_owned()),
    };
    // Pods come first and are exactly at the limit, but CPU is further over it.
    let quota = ResourceQuotaSummary {
        namespace: "team-a".to_owned(),
        name: "compute".to_owned(),
        created_at: None,
        labels: Vec::new(),
        items: vec![item("pods", "10", "10"), item("requests.cpu", "4", "5")],
        scopes: Vec::new(),
    };
    let row = crate::policy_rows::resource_quota_row(&quota);
    assert_eq!(row.status.text.as_ref(), "CPU at quota");
    let diagnosis = kind_diagnosis(
        &row.object,
        &DiagnosisInputs {
            pods: None,
            nodes: &[],
            service: None,
            bindings: None,
            tls_secrets: None,
            events: None,
            now: at(1_000),
        },
    )
    .expect("a box");
    assert!(
        diagnosis.text.starts_with("requests.cpu is at its limit"),
        "{}",
        diagnosis.text
    );
}

fn storage_inputs() -> DiagnosisInputs<'static> {
    DiagnosisInputs {
        pods: None,
        nodes: &[],
        service: None,
        bindings: None,
        tls_secrets: None,
        events: None,
        now: at(1_000),
    }
}

fn claim(phase: &str) -> PersistentVolumeClaimSummary {
    PersistentVolumeClaimSummary {
        namespace: "shop".to_owned(),
        name: "data".to_owned(),
        created_at: None,
        labels: Vec::new(),
        phase: phase.to_owned(),
        is_terminating: false,
        volume: Some("pv-1".to_owned()),
        capacity: None,
        requested: None,
        access_modes: Vec::new(),
        storage_class: None,
        volume_mode: None,
        conditions: Vec::new(),
    }
}

fn volume(phase: &str, reclaim_policy: &str) -> PersistentVolumeSummary {
    PersistentVolumeSummary {
        name: "pv-1".to_owned(),
        created_at: None,
        labels: Vec::new(),
        capacity: None,
        access_modes: Vec::new(),
        reclaim_policy: reclaim_policy.to_owned(),
        phase: phase.to_owned(),
        is_terminating: false,
        claim: Some(ClaimRef {
            namespace: "shop".to_owned(),
            name: "data".to_owned(),
        }),
        storage_class: None,
        volume_mode: None,
        backend: VolumeBackend::Other { kind: "unknown" },
        node_affinity: Vec::new(),
        mount_options: Vec::new(),
        reason: None,
        message: None,
    }
}

#[test]
fn pvc_volume_lost() {
    let diagnosis = kind_diagnosis(
        &KindObject::PersistentVolumeClaim(claim("Lost")),
        &storage_inputs(),
    )
    .expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(diagnosis.title, "VOLUME LOST");
    assert_eq!(
        diagnosis.text,
        "The bound volume pv-1 no longer exists. The data on it is gone or unreachable."
    );
}

#[test]
fn pvc_volume_lost_without_a_volume_name() {
    let mut lost = claim("Lost");
    lost.volume = None;
    let diagnosis =
        kind_diagnosis(&KindObject::PersistentVolumeClaim(lost), &storage_inputs()).expect("a box");
    assert_eq!(
        diagnosis.text,
        "The bound volume no longer exists. The data on it is gone or unreachable."
    );
}

#[test]
fn bound_pvc_has_no_box() {
    let bound = kind_diagnosis(
        &KindObject::PersistentVolumeClaim(claim("Bound")),
        &storage_inputs(),
    );
    assert_eq!(bound, None);
}

#[test]
fn pv_released_retain() {
    let diagnosis = kind_diagnosis(
        &KindObject::PersistentVolume(volume("Released", "Retain")),
        &storage_inputs(),
    )
    .expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(diagnosis.title, "RELEASED");
    assert!(
        diagnosis
            .text
            .starts_with("Claim shop/data was deleted. Reclaim policy Retain keeps")
    );
    assert!(
        diagnosis.text.contains("clear claimRef"),
        "{}",
        diagnosis.text
    );
}

#[test]
fn pv_released_delete() {
    let diagnosis = kind_diagnosis(
        &KindObject::PersistentVolume(volume("Released", "Delete")),
        &storage_inputs(),
    )
    .expect("a box");
    assert_eq!(
        diagnosis.text,
        "Claim shop/data was deleted and reclaim policy is Delete, but the volume still exists. \
         Check the Events tab for reclaim errors."
    );
}

#[test]
fn pv_reclaim_failed() {
    let mut failed = volume("Failed", "Delete");
    failed.reason = Some("VolumeFailedDelete".to_owned());
    failed.message = Some("disk is in use".to_owned());
    let diagnosis = kind_diagnosis(
        &KindObject::PersistentVolume(failed.clone()),
        &storage_inputs(),
    )
    .expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(diagnosis.title, "RECLAIM FAILED");
    assert_eq!(diagnosis.text, "VolumeFailedDelete: disk is in use");
}

#[test]
fn pv_reclaim_failed_with_reason_only() {
    let mut failed = volume("Failed", "Delete");
    failed.reason = Some("VolumeFailedDelete".to_owned());
    let diagnosis =
        kind_diagnosis(&KindObject::PersistentVolume(failed), &storage_inputs()).expect("a box");
    assert_eq!(diagnosis.text, "VolumeFailedDelete");
}

#[test]
fn pv_reclaim_failed_without_detail() {
    let diagnosis = kind_diagnosis(
        &KindObject::PersistentVolume(volume("Failed", "Delete")),
        &storage_inputs(),
    )
    .expect("a box");
    assert_eq!(
        diagnosis.text,
        "The volume could not be reclaimed. Check the Events tab."
    );
}

#[test]
fn bound_pv_has_no_box() {
    for phase in ["Bound", "Available", "Pending"] {
        let diagnosis = kind_diagnosis(
            &KindObject::PersistentVolume(volume(phase, "Retain")),
            &storage_inputs(),
        );
        assert_eq!(diagnosis, None, "{phase}");
    }
}

// ---- Access control ----

mod access {
    use cluster::{BindingSummary, RbacRule, RoleKind, RoleRef, RoleSummary, Subject, SubjectKind};

    use super::*;
    use crate::access_bindings::{BindingIndex, BindingLists};

    fn wildcard_role(namespace: Option<&str>) -> RoleSummary {
        let star = || vec!["*".to_owned()];
        RoleSummary {
            namespace: namespace.map(str::to_owned),
            name: "super".to_owned(),
            created_at: None,
            labels: Vec::new(),
            rules: vec![RbacRule {
                api_groups: star(),
                resources: star(),
                resource_names: Vec::new(),
                verbs: star(),
                non_resource_urls: Vec::new(),
            }],
            aggregation: Vec::new(),
        }
    }

    fn subject(kind: SubjectKind, namespace: Option<&str>, name: &str) -> Subject {
        Subject {
            kind,
            name: name.to_owned(),
            namespace: namespace.map(str::to_owned),
        }
    }

    fn account(namespace: &str, name: &str) -> Subject {
        subject(SubjectKind::ServiceAccount, Some(namespace), name)
    }

    fn group(name: &str) -> Subject {
        subject(SubjectKind::Group, None, name)
    }

    fn binding(namespace: Option<&str>, role: &str, subjects: Vec<Subject>) -> BindingSummary {
        BindingSummary {
            namespace: namespace.map(str::to_owned),
            name: "bind".to_owned(),
            created_at: None,
            labels: Vec::new(),
            role: RoleRef {
                kind: RoleKind::ClusterRole,
                name: role.to_owned(),
            },
            subjects,
        }
    }

    fn review(binding: BindingSummary) -> Option<KindDiagnosis> {
        kind_diagnosis(&KindObject::Binding(binding), &storage_inputs())
    }

    #[test]
    fn cluster_role_very_broad_names_first_service_account() {
        let bindings = [binding(
            None,
            "super",
            vec![
                subject(SubjectKind::User, None, "ana"),
                account("kube-system", "tiller"),
                account("shop", "api"),
            ],
        )];
        let index = BindingIndex::build(&BindingLists {
            role_bindings: &[],
            cluster_role_bindings: &bindings,
        });
        let inputs = DiagnosisInputs {
            bindings: Some(&index),
            ..storage_inputs()
        };
        let role = wildcard_role(None);
        let diagnosis = kind_diagnosis(&KindObject::Role(role), &inputs).expect("a box");
        assert_eq!(diagnosis.tone, StatusTone::Warn);
        assert_eq!(diagnosis.title, "VERY BROAD");
        assert_eq!(
            diagnosis.text,
            "Grants every verb on every resource. Bound to 3 subjects, including sa kube-system/tiller."
        );
    }

    #[test]
    fn cluster_role_very_broad_counts_a_repeated_subject_once() {
        let bindings = [
            binding(None, "super", vec![account("shop", "api")]),
            binding(Some("shop"), "super", vec![account("shop", "api")]),
        ];
        let index = BindingIndex::build(&BindingLists {
            role_bindings: &bindings[1..],
            cluster_role_bindings: &bindings[..1],
        });
        let inputs = DiagnosisInputs {
            bindings: Some(&index),
            ..storage_inputs()
        };
        let diagnosis =
            kind_diagnosis(&KindObject::Role(wildcard_role(None)), &inputs).expect("a box");
        assert_eq!(
            diagnosis.text,
            "Grants every verb on every resource. Bound to 1 subject, including sa shop/api."
        );
    }

    #[test]
    fn cluster_role_very_broad_without_bindings() {
        let diagnosis = kind_diagnosis(&KindObject::Role(wildcard_role(None)), &storage_inputs())
            .expect("a box");
        assert_eq!(diagnosis.text, "Grants every verb on every resource.");
        // A ready companion with one subject reads singular.
        let bindings = [binding(
            None,
            "super",
            vec![subject(SubjectKind::User, None, "ana")],
        )];
        let index = BindingIndex::build(&BindingLists {
            role_bindings: &[],
            cluster_role_bindings: &bindings,
        });
        let inputs = DiagnosisInputs {
            bindings: Some(&index),
            ..storage_inputs()
        };
        let diagnosis =
            kind_diagnosis(&KindObject::Role(wildcard_role(None)), &inputs).expect("a box");
        assert_eq!(
            diagnosis.text,
            "Grants every verb on every resource. Bound to 1 subject."
        );
    }

    #[test]
    fn role_very_broad_names_namespace() {
        let diagnosis = kind_diagnosis(
            &KindObject::Role(wildcard_role(Some("shop"))),
            &storage_inputs(),
        )
        .expect("a box");
        assert_eq!(
            diagnosis.text,
            "Grants every verb on every resource in shop."
        );
        let mut narrow = wildcard_role(Some("shop"));
        narrow.rules[0].verbs = vec!["get".to_owned()];
        assert_eq!(
            kind_diagnosis(&KindObject::Role(narrow), &storage_inputs()),
            None
        );
    }

    #[test]
    fn cluster_binding_review_single_and_many() {
        let one = review(binding(
            None,
            "cluster-admin",
            vec![account("kube-system", "tiller")],
        ))
        .expect("a box");
        assert_eq!(one.title, "REVIEW");
        assert_eq!(one.tone, StatusTone::Warn);
        assert_eq!(
            one.text,
            "Service account kube-system/tiller has full access to the cluster. Consider a namespaced Role instead."
        );
        let many = review(binding(
            None,
            "cluster-admin",
            vec![account("a", "x"), account("b", "y"), account("c", "z")],
        ))
        .expect("a box");
        assert_eq!(
            many.text,
            "3 service accounts, including a/x, have full access to the cluster."
        );
    }

    #[test]
    fn role_binding_review_names_namespace() {
        let one = review(binding(
            Some("shop"),
            "cluster-admin",
            vec![account("shop", "api")],
        ))
        .expect("a box");
        assert_eq!(
            one.text,
            "Service account shop/api has full access to namespace shop. Consider a narrower Role."
        );
        let many = review(binding(
            Some("shop"),
            "cluster-admin",
            vec![account("shop", "api"), account("shop", "web")],
        ))
        .expect("a box");
        assert_eq!(
            many.text,
            "2 service accounts, including shop/api, have full access to namespace shop."
        );
    }

    #[test]
    fn review_for_broad_groups() {
        let cases = [
            (
                "system:authenticated",
                StatusTone::Bad,
                "Group system:authenticated gives every signed-in user and service account full access to the cluster.",
            ),
            (
                "system:unauthenticated",
                StatusTone::Bad,
                "Group system:unauthenticated gives anonymous requests full access to the cluster.",
            ),
            (
                "system:serviceaccounts",
                StatusTone::Warn,
                "Group system:serviceaccounts gives every service account full access to the cluster.",
            ),
            (
                "system:serviceaccounts:shop",
                StatusTone::Warn,
                "Group system:serviceaccounts:shop gives every service account in shop full access to the cluster.",
            ),
        ];
        for (name, tone, text) in cases {
            let diagnosis =
                review(binding(None, "cluster-admin", vec![group(name)])).expect("a box");
            assert_eq!(diagnosis.tone, tone, "{name}");
            assert_eq!(diagnosis.text, text, "{name}");
        }
        // A group is named before a service account, and the worst group wins.
        let mixed = review(binding(
            None,
            "cluster-admin",
            vec![
                account("a", "x"),
                group("system:serviceaccounts"),
                group("system:authenticated"),
            ],
        ))
        .expect("a box");
        assert_eq!(mixed.tone, StatusTone::Bad);
        assert!(
            mixed.text.starts_with("Group system:authenticated"),
            "{}",
            mixed.text
        );
        let namespaced = review(binding(
            Some("shop"),
            "cluster-admin",
            vec![group("system:serviceaccounts:shop")],
        ))
        .expect("a box");
        assert!(
            namespaced.text.ends_with("full access to namespace shop."),
            "{}",
            namespaced.text
        );
    }

    #[test]
    fn view_binding_has_no_box() {
        assert_eq!(review(binding(None, "view", vec![account("a", "x")])), None);
        assert_eq!(
            review(binding(
                None,
                "cluster-admin",
                vec![subject(SubjectKind::User, None, "ana")]
            )),
            None
        );
    }
}

// ---- Service accounts ----

mod account {
    use cluster::{BindingSummary, RoleKind, RoleRef, ServiceAccountSummary, Subject, SubjectKind};

    use super::*;
    use crate::access_bindings::{BindingIndex, BindingLists};

    fn account() -> ServiceAccountSummary {
        ServiceAccountSummary {
            namespace: "shop".to_owned(),
            name: "api".to_owned(),
            created_at: None,
            labels: Vec::new(),
            secrets: Vec::new(),
            image_pull_secrets: Vec::new(),
            automount_token: None,
            cloud_identities: Vec::new(),
        }
    }

    fn admin_binding(namespace: Option<&str>, name: &str) -> BindingSummary {
        BindingSummary {
            namespace: namespace.map(str::to_owned),
            name: name.to_owned(),
            created_at: None,
            labels: Vec::new(),
            role: RoleRef {
                kind: RoleKind::ClusterRole,
                name: "cluster-admin".to_owned(),
            },
            subjects: vec![Subject {
                kind: SubjectKind::ServiceAccount,
                name: "api".to_owned(),
                namespace: Some("shop".to_owned()),
            }],
        }
    }

    fn diagnose(
        role_bindings: &[BindingSummary],
        cluster_role_bindings: &[BindingSummary],
    ) -> Option<KindDiagnosis> {
        let index = BindingIndex::build(&BindingLists {
            role_bindings,
            cluster_role_bindings,
        });
        let inputs = DiagnosisInputs {
            bindings: Some(&index),
            ..storage_inputs()
        };
        kind_diagnosis(&KindObject::ServiceAccount(account()), &inputs)
    }

    #[test]
    fn service_account_cluster_admin_through_cluster_binding() {
        let diagnosis = diagnose(
            &[admin_binding(Some("shop"), "local")],
            &[admin_binding(None, "root")],
        )
        .expect("a box");
        assert_eq!(diagnosis.tone, StatusTone::Warn);
        assert_eq!(diagnosis.title, "CLUSTER ADMIN");
        // The cluster-wide binding is named, not the namespace one.
        assert_eq!(
            diagnosis.text,
            "This service account has full access to the cluster through clusterrolebinding/root."
        );
    }

    #[test]
    fn service_account_admin_of_namespace() {
        let diagnosis = diagnose(&[admin_binding(Some("shop"), "local")], &[]).expect("a box");
        assert_eq!(
            diagnosis.text,
            "This service account has full access to namespace shop through rolebinding/local."
        );
    }

    #[test]
    fn service_account_admin_through_authenticated() {
        let mut open = admin_binding(None, "open-door");
        open.subjects = vec![Subject {
            kind: SubjectKind::Group,
            name: "system:authenticated".to_owned(),
            namespace: None,
        }];
        let diagnosis = diagnose(&[], &[open]).expect("a box");
        assert_eq!(
            diagnosis.text,
            "This service account has full access to the cluster through clusterrolebinding/open-door."
        );
    }

    #[test]
    fn service_account_without_cluster_admin_has_no_box() {
        let mut view = admin_binding(None, "view-all");
        view.role.name = "view".to_owned();
        assert_eq!(diagnose(&[], &[view]), None);
        // The box waits for the bindings.
        assert_eq!(
            kind_diagnosis(&KindObject::ServiceAccount(account()), &storage_inputs()),
            None
        );
    }
}

// ---- Secrets ----

const DAY: i64 = 24 * 3600;

fn tls_secret(details: SecretDetails) -> KindObject {
    KindObject::Secret(cluster::SecretSummary {
        namespace: "team-a".to_owned(),
        name: "shop-tls".to_owned(),
        created_at: None,
        labels: Vec::new(),
        secret_type: "kubernetes.io/tls".to_owned(),
        keys: Vec::new(),
        details,
        is_immutable: false,
        is_owned: false,
    })
}

fn leaf(not_before: i64, not_after: i64) -> SecretDetails {
    SecretDetails::Certificate {
        chain: vec![cluster::CertificateInfo {
            subject: "CN=shop".to_owned(),
            issuer: "CN=ca".to_owned(),
            alt_names: Vec::new(),
            not_before: at(not_before),
            not_after: at(not_after),
        }],
    }
}

/// The box of a secret at `now` seconds, with no list loaded: the CERTIFICATE rules read only
/// the secret.
fn secret_box(object: KindObject, now: i64) -> Option<KindDiagnosis> {
    kind_diagnosis(
        &object,
        &DiagnosisInputs {
            pods: None,
            nodes: &[],
            service: None,
            bindings: None,
            tls_secrets: None,
            events: None,
            now: at(now),
        },
    )
}

#[test]
fn secret_certificate_expired_box() {
    // 1970-04-11 is day 100; the certificate ended on day 97.
    let diagnosis = secret_box(tls_secret(leaf(0, 97 * DAY)), 100 * DAY).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(diagnosis.title, "CERTIFICATE");
    assert_eq!(diagnosis.text, "Expired Apr 8, 1970 (3d ago).");
    assert_eq!(diagnosis.link, None);
}

#[test]
fn secret_certificate_expiring_box() {
    let diagnosis = secret_box(tls_secret(leaf(0, 106 * DAY)), 100 * DAY).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(diagnosis.text, "Expires Apr 17, 1970 (in 6d).");
}

#[test]
fn secret_certificate_not_yet_valid_box() {
    let diagnosis = secret_box(tls_secret(leaf(110 * DAY, 400 * DAY)), 100 * DAY).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(diagnosis.text, "Not valid until Apr 21, 1970.");
}

#[test]
fn secret_not_parsed_and_missing_boxes() {
    let unparsed = tls_secret(SecretDetails::NoCertificate(
        cluster::CertificateIssue::Unparsed,
    ));
    let diagnosis = secret_box(unparsed, 0).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(
        diagnosis.text,
        "tls.crt could not be parsed as an X.509 certificate."
    );
    let missing = tls_secret(SecretDetails::NoCertificate(
        cluster::CertificateIssue::Missing,
    ));
    let diagnosis = secret_box(missing, 0).expect("a box");
    assert_eq!(diagnosis.text, "The secret has no tls.crt.");
}

#[test]
fn valid_certificate_has_no_box() {
    assert_eq!(secret_box(tls_secret(leaf(0, 400 * DAY)), 100 * DAY), None);
}

#[test]
fn expiry_box_follows_the_leaf_not_an_early_intermediate() {
    let SecretDetails::Certificate { mut chain } = leaf(0, 400 * DAY) else {
        panic!("a certificate");
    };
    chain.push(cluster::CertificateInfo {
        subject: "CN=ca".to_owned(),
        issuer: "CN=root".to_owned(),
        alt_names: Vec::new(),
        not_before: at(0),
        not_after: at(101 * DAY),
    });
    let object = tls_secret(SecretDetails::Certificate { chain });
    assert_eq!(secret_box(object, 100 * DAY), None);
}

#[test]
fn opaque_secret_has_no_box() {
    assert_eq!(secret_box(tls_secret(SecretDetails::None), 0), None);
    let registries = SecretDetails::Registries(vec!["registry.example.test".to_owned()]);
    assert_eq!(secret_box(tls_secret(registries), 0), None);
}

// ---- Ingress certificate ----

fn tls_ingress(secrets: &[&str]) -> KindObject {
    KindObject::Ingress(cluster::IngressSummary {
        namespace: "team-a".to_owned(),
        name: "shop".to_owned(),
        created_at: None,
        labels: Vec::new(),
        class: None,
        hosts: Vec::new(),
        addresses: Vec::new(),
        rules: Vec::new(),
        default_backend: None,
        default_service: None,
        tls: secrets
            .iter()
            .map(|name| cluster::IngressTls {
                hosts: Vec::new(),
                secret_name: Some((*name).to_owned()),
            })
            .collect(),
    })
}

fn named_secret(name: &str, details: SecretDetails) -> cluster::SecretSummary {
    cluster::SecretSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        secret_type: "kubernetes.io/tls".to_owned(),
        keys: Vec::new(),
        details,
        is_immutable: false,
        is_owned: false,
    }
}

fn ingress_box(
    object: &KindObject,
    secrets: Option<&[cluster::SecretSummary]>,
    now: i64,
) -> Option<KindDiagnosis> {
    kind_diagnosis(
        object,
        &DiagnosisInputs {
            pods: None,
            nodes: &[],
            service: None,
            bindings: None,
            tls_secrets: secrets,
            events: None,
            now: at(now),
        },
    )
}

fn secret_key_of(name: &str) -> Option<ResourceKey> {
    ResourceKey::of_object("Secret", Some("team-a"), name)
}

#[test]
fn ingress_certificate_box_worst_secret() {
    let secrets = [
        named_secret("fine", leaf(0, 400 * DAY)),
        named_secret("soon", leaf(0, 106 * DAY)),
        named_secret("old", leaf(0, 97 * DAY)),
    ];
    let object = tls_ingress(&["fine", "soon", "old"]);
    let diagnosis = ingress_box(&object, Some(&secrets), 100 * DAY).expect("a box");
    // Expired comes before expiring.
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(diagnosis.title, "CERTIFICATE");
    assert_eq!(diagnosis.text, "old: Expired Apr 8, 1970 (3d ago).");
    assert_eq!(diagnosis.link, secret_key_of("old"));
    // Without the expired one, the expiring one shows.
    let object = tls_ingress(&["fine", "soon"]);
    let diagnosis = ingress_box(&object, Some(&secrets), 100 * DAY).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(diagnosis.text, "soon: Expires Apr 17, 1970 (in 6d).");
    assert_eq!(diagnosis.link, secret_key_of("soon"));
    // Only fine certificates: no box.
    let object = tls_ingress(&["fine"]);
    assert_eq!(ingress_box(&object, Some(&secrets), 100 * DAY), None);
}

#[test]
fn ingress_certificate_box_picks_the_earlier_of_two_expiring() {
    let secrets = [
        named_secret("later", leaf(0, 110 * DAY)),
        named_secret("sooner", leaf(0, 103 * DAY)),
    ];
    let object = tls_ingress(&["later", "sooner"]);
    let diagnosis = ingress_box(&object, Some(&secrets), 100 * DAY).expect("a box");
    assert_eq!(diagnosis.link, secret_key_of("sooner"));
}

#[test]
fn ingress_missing_tls_secret_box() {
    let secrets = [named_secret("fine", leaf(0, 400 * DAY))];
    let object = tls_ingress(&["fine", "gone"]);
    let diagnosis = ingress_box(&object, Some(&secrets), 100 * DAY).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(diagnosis.text, "No TLS secret gone in team-a.");
    assert_eq!(diagnosis.link, secret_key_of("gone"));
}

#[test]
fn ingress_box_waits_for_companion() {
    let object = tls_ingress(&["shop-tls"]);
    assert_eq!(ingress_box(&object, None, 100 * DAY), None);
    // An ingress without TLS needs no box even with the list loaded.
    assert_eq!(ingress_box(&tls_ingress(&[]), Some(&[]), 100 * DAY), None);
}

fn claim_event(event_type: EventType, reason: &str, message: &str, seen: i64) -> EventSummary {
    EventSummary {
        namespace: "shop".to_owned(),
        name: format!("data.{seen}"),
        event_type,
        reason: reason.to_owned(),
        object: cluster::InvolvedObject {
            kind: "PersistentVolumeClaim".to_owned(),
            namespace: Some("shop".to_owned()),
            name: "data".to_owned(),
        },
        message: message.to_owned(),
        count: 1,
        first_seen: Some(at(seen)),
        last_seen: Some(at(seen)),
        source: None,
        container: None,
    }
}

const NO_CLASS_MESSAGE: &str = "storageclass.storage.k8s.io \"fast-ssd\" not found";

#[test]
fn pending_claim_box_quotes_the_newest_warning_event() {
    let events = [
        claim_event(
            EventType::Warning,
            "ProvisioningFailed",
            "older failure",
            100,
        ),
        claim_event(
            EventType::Warning,
            "ProvisioningFailed",
            NO_CLASS_MESSAGE,
            200,
        ),
        claim_event(EventType::Normal, "WaitForFirstConsumer", "waiting", 300),
    ];
    let inputs = DiagnosisInputs {
        events: Some(&events),
        ..storage_inputs()
    };
    let diagnosis = kind_diagnosis(
        &KindObject::PersistentVolumeClaim(claim("Pending")),
        &inputs,
    )
    .expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(diagnosis.title, "PENDING");
    assert_eq!(
        diagnosis.text,
        format!("ProvisioningFailed: {NO_CLASS_MESSAGE}")
    );
}

#[test]
fn pending_claim_box_needs_a_warning_event() {
    let object = KindObject::PersistentVolumeClaim(claim("Pending"));
    // A claim that waits for its first consumer has only Normal events.
    let normal = [claim_event(
        EventType::Normal,
        "WaitForFirstConsumer",
        "waiting",
        1,
    )];
    let inputs = DiagnosisInputs {
        events: Some(&normal),
        ..storage_inputs()
    };
    assert_eq!(kind_diagnosis(&object, &inputs), None);
    assert_eq!(kind_diagnosis(&object, &storage_inputs()), None);
}

#[test]
fn a_class_the_events_call_missing_is_named_missing() {
    let events = [claim_event(
        EventType::Warning,
        "ProvisioningFailed",
        NO_CLASS_MESSAGE,
        200,
    )];
    let mut pending = claim("Pending");
    assert_eq!(missing_storage_class(&pending, &events), None);
    pending.storage_class = Some("fast-ssd".to_owned());
    assert_eq!(missing_storage_class(&pending, &events), Some("fast-ssd"));
    pending.storage_class = Some("standard".to_owned());
    assert_eq!(missing_storage_class(&pending, &events), None);
}

fn old_ingress(class: Option<&str>, addresses: &[&str]) -> KindObject {
    let KindObject::Ingress(mut ingress) = tls_ingress(&[]) else {
        unreachable!("tls_ingress builds an ingress");
    };
    ingress.created_at = Some(at(0));
    ingress.class = class.map(str::to_owned);
    ingress.addresses = addresses.iter().map(|text| (*text).to_owned()).collect();
    KindObject::Ingress(ingress)
}

#[test]
fn ingress_without_an_address_names_the_missing_controller() {
    let classless = ingress_box(&old_ingress(None, &[]), None, 600).expect("a box");
    assert_eq!(classless.tone, StatusTone::Warn);
    assert_eq!(classless.title, "NO ADDRESS");
    assert!(
        classless
            .text
            .starts_with("No address: no ingress controller has picked it up (no ingressClassName"),
        "{}",
        classless.text
    );
    let classed = ingress_box(&old_ingress(Some("nginx"), &[]), None, 600).expect("a box");
    assert_eq!(
        classed.text,
        "No address: no ingress controller has picked it up (class nginx: no controller reports it)."
    );
}

#[test]
fn ingress_address_box_waits_for_the_grace_and_an_empty_status() {
    assert_eq!(ingress_box(&old_ingress(None, &[]), None, 300), None);
    assert_eq!(
        ingress_box(&old_ingress(None, &["10.0.0.5"]), None, 600),
        None
    );
}

// ---- Helm releases ----

fn helm_release(status: HelmStatus, description: Option<&str>) -> HelmReleaseSummary {
    HelmReleaseSummary {
        namespace: "shop".to_owned(),
        name: "api".to_owned(),
        revision: 5,
        status,
        chart: None,
        updated_at: Some(at(1_000)),
        description: description.map(str::to_owned),
        deployed_revision: None,
    }
}

fn helm_box(release: &HelmReleaseSummary) -> Option<KindDiagnosis> {
    helm_release_diagnosis(release, at(1_000 + 7_200))
}

#[test]
fn upgrade_failed_box_names_deployed_revision() {
    let mut release = helm_release(
        HelmStatus::Failed,
        Some("Upgrade \"api\" failed: timed out"),
    );
    release.deployed_revision = Some(4);
    let diagnosis = helm_box(&release).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Bad);
    assert_eq!(diagnosis.title, "UPGRADE FAILED");
    assert_eq!(
        diagnosis.text,
        "Upgrade \"api\" failed: timed out Revision 4 is still deployed."
    );
    release.deployed_revision = None;
    assert_eq!(
        helm_box(&release).expect("a box").text,
        "Upgrade \"api\" failed: timed out"
    );
}

#[test]
fn rollback_failed_by_description_prefix() {
    let release = helm_release(
        HelmStatus::Failed,
        Some("Rollback \"api\" failed: no revision"),
    );
    let diagnosis = helm_box(&release).expect("a box");
    assert_eq!(diagnosis.title, "ROLLBACK FAILED");
    assert_eq!(diagnosis.text, "Rollback \"api\" failed: no revision");
}

#[test]
fn install_failed_for_other_descriptions() {
    for description in [
        "Release \"api\" failed: boom",
        "failed pre-install: timed out",
    ] {
        let diagnosis =
            helm_box(&helm_release(HelmStatus::Failed, Some(description))).expect("a box");
        assert_eq!(diagnosis.title, "INSTALL FAILED");
        assert_eq!(diagnosis.text, description);
    }
}

#[test]
fn failed_without_description_has_fallback_text() {
    let diagnosis = helm_box(&helm_release(HelmStatus::Failed, None)).expect("a box");
    assert_eq!(diagnosis.title, "INSTALL FAILED");
    assert_eq!(diagnosis.text, "Helm gave no reason.");
}

#[test]
fn pending_release_box_warns() {
    for (status, title) in [
        (HelmStatus::PendingInstall, "PENDING INSTALL"),
        (HelmStatus::PendingUpgrade, "PENDING UPGRADE"),
        (HelmStatus::PendingRollback, "PENDING ROLLBACK"),
    ] {
        let diagnosis = helm_box(&helm_release(status, None)).expect("a box");
        assert_eq!(diagnosis.tone, StatusTone::Warn);
        assert_eq!(diagnosis.title, title);
        assert_eq!(
            diagnosis.text,
            "Helm started this operation 2h ago and has not finished. A pending release blocks the next helm upgrade."
        );
    }
    let uninstalling = helm_box(&helm_release(HelmStatus::Uninstalling, None)).expect("a box");
    assert_eq!(uninstalling.title, "UNINSTALLING");
    assert_eq!(
        uninstalling.text,
        "Helm started uninstalling this release 2h ago."
    );
    let unknown =
        helm_box(&helm_release(HelmStatus::Unknown("odd".to_owned()), None)).expect("a box");
    assert_eq!(unknown.title, "UNKNOWN STATUS");
    assert_eq!(unknown.text, "Helm reports status \"odd\".");
}

#[test]
fn deployed_release_has_no_box() {
    for status in [
        HelmStatus::Deployed,
        HelmStatus::Uninstalled,
        HelmStatus::Superseded,
    ] {
        assert_eq!(
            helm_box(&helm_release(status, Some("Install complete"))),
            None
        );
    }
}

fn object_condition(
    name: &str,
    status: ConditionStatus,
    reason: Option<&str>,
    message: Option<&str>,
) -> cluster::ObjectCondition {
    cluster::ObjectCondition {
        name: name.to_owned(),
        status,
        reason: reason.map(str::to_owned),
        message: message.map(str::to_owned),
        changed_at: None,
    }
}

fn custom_box(conditions: Vec<cluster::ObjectCondition>) -> Option<KindDiagnosis> {
    custom_object_diagnosis(&CustomObjectSummary {
        namespace: Some("ingress".to_owned()),
        name: "tls-shop-example".to_owned(),
        created_at: None,
        labels: Vec::new(),
        columns: Vec::new(),
        conditions,
        phase: None,
    })
}

#[test]
fn custom_box_reports_ready_false() {
    let with_message = custom_box(vec![object_condition(
        "Ready",
        ConditionStatus::False,
        Some("Pending"),
        Some("Issuing certificate as Secret does not exist"),
    )])
    .expect("a box");
    assert_eq!(with_message.tone, StatusTone::Bad);
    assert_eq!(with_message.title, "NOT READY");
    assert_eq!(
        with_message.text,
        "Issuing certificate as Secret does not exist"
    );
    let without = custom_box(vec![object_condition(
        "Ready",
        ConditionStatus::False,
        Some("Pending"),
        None,
    )])
    .expect("a box");
    assert_eq!(without.text, "Ready is False: Pending");
    let bare = custom_box(vec![object_condition(
        "Available",
        ConditionStatus::False,
        None,
        None,
    )])
    .expect("a box");
    assert_eq!(
        (bare.title.as_str(), bare.text.as_str()),
        ("UNAVAILABLE", "Available is False")
    );
    let unknown = custom_box(vec![object_condition(
        "Ready",
        ConditionStatus::Unknown,
        None,
        Some("waiting"),
    )])
    .expect("a box");
    assert_eq!(
        (unknown.tone, unknown.text.as_str()),
        (StatusTone::Warn, "waiting")
    );
}

#[test]
fn custom_box_reports_failing_condition() {
    let found = custom_box(vec![
        object_condition("Ready", ConditionStatus::True, None, None),
        object_condition(
            "Issuing",
            ConditionStatus::False,
            Some("Failed"),
            Some("Last renewal attempt failed: ACME challenge returned 404"),
        ),
    ])
    .expect("a box");
    assert_eq!(found.tone, StatusTone::Warn);
    assert_eq!(found.title, "ISSUING FAILING");
    assert_eq!(
        found.text,
        "Issuing: Last renewal attempt failed: ACME challenge returned 404"
    );
    // The reason stands in when there is no message.
    let by_reason = custom_box(vec![object_condition(
        "Synced",
        ConditionStatus::False,
        Some("ReconcileError"),
        None,
    )])
    .expect("a box");
    assert_eq!(by_reason.text, "Synced: ReconcileError");
}

#[test]
fn custom_box_absent_when_healthy() {
    assert_eq!(custom_box(Vec::new()), None);
    assert_eq!(
        custom_box(vec![
            object_condition("Ready", ConditionStatus::True, None, None),
            object_condition("Synced", ConditionStatus::True, Some("Synced"), None),
        ]),
        None
    );
    // Ready False wins over a failing condition.
    let both = custom_box(vec![
        object_condition("Failing", ConditionStatus::False, Some("Failed"), None),
        object_condition("Ready", ConditionStatus::False, None, Some("not ready")),
    ])
    .expect("a box");
    assert_eq!(both.tone, StatusTone::Bad);
}

fn stuck_namespace(
    phase: cluster::NamespacePhase,
    since: Option<i64>,
    conditions: &[(&str, Option<&str>)],
) -> cluster::NamespaceSummary {
    cluster::NamespaceSummary {
        name: "team-a".to_owned(),
        phase,
        labels: Vec::new(),
        created_at: None,
        deleting_since: since.map(at),
        deletion_conditions: conditions
            .iter()
            .map(|(name, message)| cluster::NamespaceDeletionCondition {
                name: (*name).to_owned(),
                reason: None,
                message: message.map(str::to_owned),
            })
            .collect(),
    }
}

#[test]
fn stuck_box_after_five_minutes() {
    let namespace = stuck_namespace(
        cluster::NamespacePhase::Terminating,
        Some(1_000),
        &[
            (
                "NamespaceContentRemaining",
                Some("Some resources are remaining: pods. has 2 resource instances"),
            ),
            ("NamespaceFinalizersRemaining", Some("finalizers remain")),
        ],
    );
    // Exactly five minutes is not yet stuck.
    assert_eq!(namespace_diagnosis(&namespace, at(1_000 + 300)), None);
    let found = namespace_diagnosis(&namespace, at(1_000 + 301)).expect("a box");
    assert_eq!(found.title, "STUCK");
    assert_eq!(found.tone, StatusTone::Warn);
    assert_eq!(
        found.text,
        "Terminating for 5m. Some resources are remaining: pods. has 2 resource instances finalizers remain"
    );
}

#[test]
fn stuck_box_is_bad_with_failure_conditions() {
    let namespace = stuck_namespace(
        cluster::NamespacePhase::Terminating,
        Some(0),
        &[(
            "NamespaceDeletionDiscoveryFailure",
            Some("Discovery failed"),
        )],
    );
    let found = namespace_diagnosis(&namespace, at(7_200)).expect("a box");
    assert_eq!(found.tone, StatusTone::Bad);
    assert_eq!(found.text, "Terminating for 2h. Discovery failed");
    // No message at all still says why there is none.
    let silent = stuck_namespace(cluster::NamespacePhase::Terminating, Some(0), &[]);
    let found = namespace_diagnosis(&silent, at(7_200)).expect("a box");
    assert_eq!(
        found.text,
        "Terminating for 2h. The namespace reports no reason."
    );
}

#[test]
fn no_box_while_recently_terminating() {
    let recent = stuck_namespace(cluster::NamespacePhase::Terminating, Some(1_000), &[]);
    assert_eq!(namespace_diagnosis(&recent, at(1_060)), None);
    let active = stuck_namespace(cluster::NamespacePhase::Active, None, &[]);
    assert_eq!(namespace_diagnosis(&active, at(100_000)), None);
    // Terminating without a deletion time cannot be timed.
    let untimed = stuck_namespace(cluster::NamespacePhase::Terminating, None, &[]);
    assert_eq!(namespace_diagnosis(&untimed, at(100_000)), None);
}

// ---- CronJobs ----

fn cron_job_ran_at_ten() -> cluster::CronJobSummary {
    let ran_at: Timestamp = "2024-10-04T10:00:00Z".parse().expect("timestamp");
    cluster::CronJobSummary {
        namespace: "team-a".to_owned(),
        name: "reconcile".to_owned(),
        created_at: None,
        labels: Vec::new(),
        schedule: "*/5 * * * *".to_owned(),
        time_zone: None,
        timetable: cluster::CronSchedule::parse("*/5 * * * *", None),
        is_suspended: false,
        concurrency_policy: "Forbid".to_owned(),
        starting_deadline_seconds: Some(60),
        successful_history_limit: None,
        failed_history_limit: None,
        active_jobs: Vec::new(),
        last_schedule_at: Some(ran_at),
        last_success_at: Some(ran_at),
        containers: Vec::new(),
    }
}

#[test]
fn cron_job_healthy_has_no_box() {
    let now = "2024-10-04T10:02:00Z".parse().expect("timestamp");
    assert!(cron_job_diagnosis(&cron_job_ran_at_ten(), now).is_none());
}

#[test]
fn cron_job_suspended_names_resume_and_trigger_now() {
    let mut cron = cron_job_ran_at_ten();
    cron.is_suspended = true;
    let now = "2024-10-04T12:00:00Z".parse().expect("timestamp");
    let diagnosis = cron_job_diagnosis(&cron, now).expect("a box");
    assert_eq!(diagnosis.tone, StatusTone::Warn);
    assert_eq!(diagnosis.title, "SUSPENDED");
    assert!(diagnosis.text.contains("Resume") && diagnosis.text.contains("Trigger now"));
}

#[test]
fn cron_job_missed_run_names_the_overdue_run() {
    let now = "2024-10-04T10:20:00Z".parse().expect("timestamp");
    let diagnosis = cron_job_diagnosis(&cron_job_ran_at_ten(), now).expect("a box");
    assert_eq!(diagnosis.title, "SCHEDULE MISSED");
    assert_eq!(
        diagnosis.text,
        concat!(
            "No job started for the run due 15m ago, and its starting deadline of 60s has ",
            "passed. The CronJob controller may be down or too busy, or the deadline too ",
            "short. Trigger now to run it once."
        )
    );
}

#[test]
fn cron_job_failed_last_run_points_to_recent_jobs() {
    let mut cron = cron_job_ran_at_ten();
    cron.last_success_at = None;
    let now = "2024-10-04T10:05:30Z".parse().expect("timestamp");
    let diagnosis = cron_job_diagnosis(&cron, now).expect("a box");
    assert_eq!(diagnosis.title, "LAST RUN FAILED");
    assert!(diagnosis.text.contains("Recent jobs"));
    assert!(diagnosis.text.contains("Trigger now"));
}
