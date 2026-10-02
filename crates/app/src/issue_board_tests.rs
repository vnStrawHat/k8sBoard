use cluster::{
    ContainerKind, ContainerProbes, ContainerState, ContainerSummary, ControllerRef, EventSummary,
    EventType, InvolvedObject, NodeCondition, NodeReadiness, NodeScheduling, NodeStatus,
    NodeSummary, NodeSystemInfo, PodCondition, PodStatus, PodSummary, ReadyCount, StatusReason,
    Termination, WatchUpdate,
};

use super::*;
use crate::issue::{IssueAction, IssueRule};
use crate::issue_feeds::{FeedState, IssueFeed};
use crate::resource_kind::ResourceKind;

const NOW: i64 = 1_000_000;

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("valid timestamp")
}

fn ago(seconds: i64) -> Timestamp {
    at(NOW - seconds)
}

fn container(name: &str, state: ContainerState) -> ContainerSummary {
    ContainerSummary {
        name: name.to_owned(),
        image: "registry/app:1".to_owned(),
        kind: ContainerKind::Main,
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

fn pod_of(namespace: &str, name: &str, controller: Option<(&str, &str)>) -> PodSummary {
    PodSummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: Some(ago(86_400)),
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: controller.map(|(kind, name)| ControllerRef {
            kind: kind.to_owned(),
            name: name.to_owned(),
        }),
        conditions: Vec::new(),
        containers: vec![ContainerSummary {
            is_ready: true,
            ..container("api", ContainerState::Running { started_at: None })
        }],
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
    }
}

fn not_ready_since(seconds_ago: i64) -> PodCondition {
    PodCondition {
        name: "Ready".to_owned(),
        is_true: false,
        reason: None,
        message: None,
        changed_at: Some(ago(seconds_ago)),
    }
}

/// A pod of the `api` Deployment whose container crash loops; `restarts` tells pods apart in the
/// cause text.
fn crashing_pod(name: &str, ready_ago: i64, restarts: u32) -> PodSummary {
    let mut pod = pod_of("shop", name, Some(("ReplicaSet", "api-7d9f8c")));
    pod.conditions = vec![not_ready_since(ready_ago)];
    pod.containers = vec![ContainerSummary {
        restart_count: restarts,
        last_termination: Some(Termination {
            reason: Some(StatusReason::Error),
            exit_code: 1,
            signal: None,
            started_at: Some(ago(ready_ago + 60)),
            finished_at: Some(ago(ready_ago)),
        }),
        ..container(
            "api",
            ContainerState::Waiting {
                reason: Some(StatusReason::CrashLoopBackOff),
                message: None,
            },
        )
    }];
    pod
}

/// A pod of the `api` Deployment that runs but is not ready.
fn unready_pod(name: &str, ready_ago: i64) -> PodSummary {
    let mut pod = pod_of("shop", name, Some(("ReplicaSet", "api-7d9f8c")));
    pod.conditions = vec![not_ready_since(ready_ago)];
    pod.containers[0].is_ready = false;
    pod
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
        conditions: vec![NodeCondition {
            name: "Ready".to_owned(),
            status: cluster::ConditionStatus::False,
            reason: None,
            message: None,
            changed_at: Some(ago(7_200)),
        }],
        addresses: Vec::new(),
        system: NodeSystemInfo::default(),
        resources: Vec::new(),
        labels: Vec::new(),
    }
}

fn burst(kind: &str, name: &str) -> EventSummary {
    EventSummary {
        namespace: "shop".to_owned(),
        name: format!("{name}.burst"),
        event_type: EventType::Warning,
        reason: "BackOff".to_owned(),
        object: InvolvedObject {
            kind: kind.to_owned(),
            namespace: Some("shop".to_owned()),
            name: name.to_owned(),
        },
        message: "Back-off restarting failed container".to_owned(),
        count: 20,
        first_seen: Some(ago(600)),
        last_seen: Some(ago(30)),
        source: None,
        container: None,
    }
}

fn feed(events: Vec<EventSummary>) -> WarningEvents {
    let mut feed = WarningEvents::default();
    feed.apply(WatchUpdate::Snapshot(events));
    feed
}

fn inputs_at<'a>(pods: &'a [PodSummary], nodes: &'a [NodeSummary], now: i64) -> IssueInputs<'a> {
    IssueInputs {
        pods: Some(pods),
        nodes: Some(nodes),
        scope: &NamespaceScope::All,
        namespaces: None,
        events: None,
        objects: &[],
        pod_usage: None,
        node_usage: None,
        kubelet: None,
        is_job_feed_live: false,
        now: at(now),
    }
}

fn inputs<'a>(pods: &'a [PodSummary]) -> IssueInputs<'a> {
    inputs_at(pods, &[], NOW)
}

fn state_of<T>(list: Option<T>) -> FeedState {
    list.map_or(FeedState::Loading, |_| FeedState::Live)
}

/// The coverage a session would report for `inputs`: a list that is there is live, one that is
/// not is still loading.
fn coverage_of(inputs: &IssueInputs) -> Coverage {
    Coverage {
        feeds: vec![
            (IssueFeed::Pods, state_of(inputs.pods)),
            (IssueFeed::Nodes, state_of(inputs.nodes)),
        ],
    }
}

fn refresh(board: &mut IssueBoard, inputs: &IssueInputs) -> IssueChange {
    board.refresh(inputs, coverage_of(inputs))
}

/// The issues one run of `inputs` produces.
fn issues_of(inputs: &IssueInputs) -> Vec<Issue> {
    let mut board = IssueBoard::default();
    refresh(&mut board, inputs);
    board.issues
}

/// `coverage_of(inputs)` with one more feed that is still loading.
fn with_loading(inputs: &IssueInputs, feed: IssueFeed) -> Coverage {
    let mut coverage = coverage_of(inputs);
    coverage.feeds.push((feed, FeedState::Loading));
    coverage
}

#[test]
fn pods_of_one_deployment_group_with_count() {
    let pods = [
        crashing_pod("api-7d9f8c-a", 600, 3),
        crashing_pod("api-7d9f8c-b", 600, 3),
        crashing_pod("api-7d9f8c-c", 600, 3),
    ];
    let issues = issues_of(&inputs(&pods));
    let [issue] = issues.as_slice() else {
        panic!("one issue, got {issues:?}");
    };
    let deployment = IssueObject::new("Deployment", Some("shop"), "api");
    assert_eq!(issue.count, 3);
    assert_eq!(issue.shown, deployment);
    assert_eq!(
        issue.key,
        IssueKey {
            rule: IssueRule::PodCrash,
            object: deployment,
        }
    );
    assert_eq!(issue.severity, IssueSeverity::Critical);
    assert!(issue.target.is_some(), "a Deployment has a screen");
}

#[test]
fn group_of_one_shows_the_pod() {
    let pods = [crashing_pod("api-7d9f8c-a", 600, 3)];
    let issues = issues_of(&inputs(&pods));
    let [issue] = issues.as_slice() else {
        panic!("one issue, got {issues:?}");
    };
    assert_eq!(issue.count, 1);
    assert_eq!(issue.shown, IssueObject::pod("shop", "api-7d9f8c-a"));
    // The key still names the workload, so the age survives the group growing to two pods.
    assert_eq!(
        issue.key.object,
        IssueObject::new("Deployment", Some("shop"), "api")
    );
    assert_eq!(issue.target, issue.shown.target());
}

#[test]
fn group_without_a_screen_shows_the_representative_pod() {
    let mut pods = [crashing_pod("a-0", 600, 1), crashing_pod("a-1", 300, 1)];
    for pod in &mut pods {
        pod.controller = Some(ControllerRef {
            kind: "Widget".to_owned(),
            name: "w".to_owned(),
        });
    }
    let issues = issues_of(&inputs(&pods));
    let [issue] = issues.as_slice() else {
        panic!("one issue, got {issues:?}");
    };
    assert_eq!(issue.count, 2);
    assert_eq!(issue.shown, IssueObject::pod("shop", "a-0"));
}

#[test]
fn group_representative_is_oldest() {
    let pods = [
        crashing_pod("api-7d9f8c-new", 60, 2),
        crashing_pod("api-7d9f8c-old", 3_600, 9),
        crashing_pod("api-7d9f8c-mid", 600, 5),
    ];
    let issues = issues_of(&inputs(&pods));
    let [issue] = issues.as_slice() else {
        panic!("one issue, got {issues:?}");
    };
    assert_eq!(issue.since, ago(3_600));
    assert!(issue.cause.contains("Restarted 9 times"), "{}", issue.cause);
    assert_eq!(
        issue.action,
        IssueAction::ViewLogs {
            container: Some("api".to_owned())
        }
    );
}

#[test]
fn one_issue_per_object_keeps_rule_order() {
    // Two pods of one Deployment hit different rules: the Deployment shows its first rule.
    let pods = [
        unready_pod("api-7d9f8c-b", 3_600),
        crashing_pod("api-7d9f8c-a", 600, 3),
    ];
    let issues = issues_of(&inputs(&pods));
    let [issue] = issues.as_slice() else {
        panic!("one issue, got {issues:?}");
    };
    assert_eq!(issue.key.rule, IssueRule::PodCrash);
    assert_eq!(issue.count, 1);
}

#[test]
fn event_on_grouped_pod_is_dropped() {
    let pods = [
        crashing_pod("api-7d9f8c-a", 600, 3),
        crashing_pod("api-7d9f8c-b", 600, 3),
    ];
    let events = feed(vec![burst("Pod", "api-7d9f8c-b")]);
    let issues = issues_of(&IssueInputs {
        events: Some(&events),
        ..inputs(&pods)
    });
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].key.rule, IssueRule::PodCrash);
}

#[test]
fn event_for_deleted_pod_is_dropped() {
    let pods = [pod_of("shop", "web-0", None)];
    let events = feed(vec![burst("Pod", "gone-0"), burst("Pod", "web-0")]);
    let issues = issues_of(&IssueInputs {
        events: Some(&events),
        ..inputs(&pods)
    });
    let names: Vec<&str> = issues
        .iter()
        .map(|issue| issue.shown.name.as_str())
        .collect();
    assert_eq!(names, ["web-0"]);
    // Without a loaded pods list nothing says the pod is gone.
    let unknown = issues_of(&IssueInputs {
        pods: None,
        events: Some(&events),
        ..inputs(&pods)
    });
    assert_eq!(unknown.len(), 2);
}

#[test]
fn event_on_a_deployment_pod_group_is_dropped_by_the_workload() {
    let pods = [crashing_pod("api-7d9f8c-a", 600, 3)];
    // FailedCreate on the ReplicaSet is reported as its Deployment, which the pod group claimed.
    let mut created = burst("ReplicaSet", "api-7d9f8c");
    created.reason = "FailedCreate".to_owned();
    let events = feed(vec![created]);
    let issues = issues_of(&IssueInputs {
        events: Some(&events),
        ..inputs(&pods)
    });
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].key.rule, IssueRule::PodCrash);
}

#[test]
fn missing_feed_skips_its_rules() {
    let pods = [
        crashing_pod("api-7d9f8c-a", 600, 3),
        pod_of("shop", "web-0", None),
    ];
    let nodes = [node("node-a", NodeReadiness::NotReady)];
    let events = feed(vec![burst("Pod", "web-0")]);
    let rules = |inputs: &IssueInputs| -> Vec<IssueRule> {
        issues_of(inputs)
            .iter()
            .map(|issue| issue.key.rule)
            .collect()
    };
    let full = IssueInputs {
        events: Some(&events),
        ..inputs_at(&pods, &nodes, NOW)
    };
    assert_eq!(
        rules(&full),
        [
            IssueRule::NodeNotReady,
            IssueRule::PodCrash,
            IssueRule::EventBurst
        ]
    );
    let without_pods = IssueInputs { pods: None, ..full };
    // The event about web-0 stays: with no pods list nothing says the pod is gone.
    assert_eq!(
        rules(&without_pods),
        [IssueRule::NodeNotReady, IssueRule::EventBurst]
    );
    let without_nodes = IssueInputs {
        nodes: None,
        ..inputs_at(&pods, &nodes, NOW)
    };
    assert_eq!(rules(&without_nodes), [IssueRule::PodCrash]);
    let without_events = inputs_at(&pods, &nodes, NOW);
    assert_eq!(
        rules(&without_events),
        [IssueRule::NodeNotReady, IssueRule::PodCrash]
    );
}

#[test]
fn issues_sort_by_severity_then_age() {
    let mut pods = [
        unready_pod("old-warning", 7_200),
        crashing_pod("new-critical", 60, 1),
        crashing_pod("old-critical", 3_600, 1),
    ];
    for pod in &mut pods {
        // Distinct workloads, so nothing groups.
        pod.controller = Some(ControllerRef {
            kind: "StatefulSet".to_owned(),
            name: pod.name.clone(),
        });
    }
    let issues = issues_of(&inputs(&pods));
    let order: Vec<&str> = issues
        .iter()
        .map(|issue| issue.shown.name.as_str())
        .collect();
    assert_eq!(order, ["old-critical", "new-critical", "old-warning"]);
}

#[test]
fn since_prefers_onset_over_first_seen() {
    let pods = [crashing_pod("api-7d9f8c-a", 900, 3)];
    let mut board = IssueBoard::default();
    // The board saw the pod only a minute ago, but the API says it broke 15 minutes before.
    refresh(&mut board, &inputs_at(&pods, &[], NOW - 60));
    refresh(&mut board, &inputs(&pods));
    assert_eq!(board.issues[0].since, ago(900));
}

/// The pod of a Deployment that is unschedulable, with or without an onset.
fn pending_pod(changed_ago: Option<i64>) -> PodSummary {
    let mut pod = pod_of("shop", "api-7d9f8c-a", Some(("ReplicaSet", "api-7d9f8c")));
    pod.status = PodStatus::Reason(StatusReason::Pending);
    pod.created_at = None;
    pod.containers.clear();
    pod.conditions = vec![PodCondition {
        name: "PodScheduled".to_owned(),
        is_true: false,
        reason: Some("Unschedulable".to_owned()),
        message: None,
        changed_at: changed_ago.map(ago),
    }];
    pod
}

#[test]
fn grace_holds_findings_back() {
    let grace = crate::issue_rules::UNSCHEDULABLE_GRACE.as_secs();
    // (onset age in seconds, whether the issue shows)
    let cases = [
        (Some(grace - 1), false),
        (Some(grace), true),
        (Some(grace + 1), true),
    ];
    for (onset_ago, is_shown) in cases {
        let pods = [pending_pod(onset_ago)];
        let shown = !issues_of(&inputs(&pods)).is_empty();
        assert_eq!(shown, is_shown, "onset {onset_ago:?} s ago");
    }
    // Without an onset the age counts from first seen.
    let pods = [pending_pod(None)];
    let mut board = IssueBoard::default();
    refresh(&mut board, &inputs_at(&pods, &[], NOW));
    assert!(board.issues.is_empty(), "just seen");
    refresh(&mut board, &inputs_at(&pods, &[], NOW + grace - 1));
    assert!(board.issues.is_empty(), "one second short");
    refresh(&mut board, &inputs_at(&pods, &[], NOW + grace));
    assert_eq!(board.issues.len(), 1);
    assert_eq!(board.issues[0].since, at(NOW));
}

#[test]
fn held_finding_records_first_seen_and_takes_its_slot() {
    let pods = [pending_pod(Some(10))];
    let events = feed(vec![burst("Pod", "api-7d9f8c-a")]);
    let mut board = IssueBoard::default();
    refresh(
        &mut board,
        &IssueInputs {
            events: Some(&events),
            ..inputs(&pods)
        },
    );
    // The unschedulable pod is held, and the event about it does not take its place.
    assert!(board.issues.is_empty());
    let key = IssueKey {
        rule: IssueRule::PodUnschedulable,
        object: IssueObject::new("Deployment", Some("shop"), "api"),
    };
    assert_eq!(board.first_seen.get(&key), Some(&at(NOW)));
}

#[test]
fn first_seen_survives_refresh_and_is_pruned() {
    let pods = [pending_pod(None)];
    let mut board = IssueBoard::default();
    refresh(&mut board, &inputs_at(&pods, &[], NOW));
    refresh(&mut board, &inputs_at(&pods, &[], NOW + 30));
    let seen = |board: &IssueBoard| board.first_seen.values().copied().collect::<Vec<_>>();
    assert_eq!(seen(&board), [at(NOW)]);
    // The pod is fine now: its key is forgotten, and a new problem starts again.
    let healthy = [pod_of(
        "shop",
        "api-7d9f8c-a",
        Some(("ReplicaSet", "api-7d9f8c")),
    )];
    refresh(&mut board, &inputs_at(&healthy, &[], NOW + 60));
    assert!(board.first_seen.is_empty());
    refresh(&mut board, &inputs_at(&pods, &[], NOW + 90));
    assert_eq!(seen(&board), [at(NOW + 90)]);
}

#[test]
fn refresh_reports_change_only_when_different() {
    let pods = [crashing_pod("api-7d9f8c-a", 600, 3)];
    let mut board = IssueBoard::default();
    assert_eq!(
        refresh(&mut board, &inputs(&pods)),
        IssueChange::Shape,
        "the first run finds the issue"
    );
    assert_eq!(
        refresh(&mut board, &inputs_at(&pods, &[], NOW + 5)),
        IssueChange::Unchanged
    );
    // A pod that went away, or a new one, changes the shape.
    let other = [crashing_pod("api-7d9f8c-b", 600, 3)];
    assert_eq!(refresh(&mut board, &inputs(&other)), IssueChange::Shape);
    // Coverage alone is a change too.
    let run = inputs(&other);
    let partial = with_loading(&run, IssueFeed::PodMetrics);
    assert_eq!(board.refresh(&run, partial.clone()), IssueChange::Shape);
    assert_eq!(board.refresh(&run, partial), IssueChange::Unchanged);
}

#[test]
fn an_age_that_moved_is_a_text_only_change() {
    // The cause of a pod stuck in ContainerCreating says how long it has been stuck.
    let mut stuck = pod_of("shop", "web-0", None);
    stuck.status = PodStatus::Reason(StatusReason::ContainerCreating);
    stuck.containers.clear();
    stuck.created_at = Some(ago(660));
    let pods = [stuck];
    let mut board = IssueBoard::default();
    assert_eq!(refresh(&mut board, &inputs(&pods)), IssueChange::Shape);
    let first = board.issues[0].cause.clone();
    let later = refresh(&mut board, &inputs_at(&pods, &[], NOW + 120));
    assert_eq!(later, IssueChange::TextOnly);
    assert_ne!(board.issues[0].cause, first);
    // The restart count in a crash loop's cause is text too.
    let crashing = [crashing_pod("api-7d9f8c-a", 600, 3)];
    refresh(&mut board, &inputs(&crashing));
    let counted = [crashing_pod("api-7d9f8c-a", 600, 4)];
    assert_eq!(
        refresh(&mut board, &inputs(&counted)),
        IssueChange::TextOnly
    );
}

#[test]
fn run_due_dirty_or_time_refresh() {
    let mut board = IssueBoard::default();
    assert_eq!(board.run_due(at(NOW)), Some(RunReason::Dirty));
    refresh(&mut board, &inputs(&[]));
    assert_eq!(board.run_due(at(NOW + 1)), None);
    assert_eq!(board.run_due(at(NOW + 29)), None);
    assert_eq!(board.run_due(at(NOW + 30)), Some(RunReason::TimeRefresh));
    board.mark_dirty();
    assert_eq!(board.run_due(at(NOW + 1)), Some(RunReason::Dirty));
}

#[test]
fn summary_none_until_pods_and_nodes_ready() {
    let mut board = IssueBoard::default();
    assert_eq!(board.summary(), None);
    let pods = [crashing_pod("api-7d9f8c-a", 600, 3)];
    let only_pods = IssueInputs {
        nodes: None,
        ..inputs(&pods)
    };
    refresh(&mut board, &only_pods);
    assert_eq!(board.summary(), None, "nodes have not loaded");
    assert_eq!(board.count_for(Screen::Pods), None);
    refresh(&mut board, &inputs(&pods));
    assert_eq!(
        board.summary(),
        Some(IssueSummary {
            total: 1,
            critical: 1,
            is_partial: false
        })
    );
    let run = inputs(&pods);
    board.refresh(&run, with_loading(&run, IssueFeed::PodMetrics));
    assert!(board.summary().is_some_and(|summary| summary.is_partial));
}

#[test]
fn a_failed_core_list_is_a_gap_not_endless_checking() {
    let pods = [crashing_pod("api-7d9f8c-a", 600, 3)];
    let run = IssueInputs {
        nodes: None,
        ..inputs(&pods)
    };
    // The nodes list failed: it is off in the coverage, and what pods say still counts.
    let mut coverage = coverage_of(&run);
    coverage.feeds[1].1 = FeedState::Off("forbidden".to_owned());
    let mut board = IssueBoard::default();
    board.refresh(&run, coverage);
    assert_eq!(
        board.summary(),
        Some(IssueSummary {
            total: 1,
            critical: 1,
            is_partial: true
        })
    );
}

#[test]
fn summary_worst_follows_the_critical_count() {
    let summary = |critical| IssueSummary {
        total: 3,
        critical,
        is_partial: false,
    };
    assert_eq!(summary(1).worst(), IssueSeverity::Critical);
    assert_eq!(summary(0).worst(), IssueSeverity::Warning);
}

#[test]
fn first_seen_kept_while_pods_reload() {
    let pods = [pending_pod(None)];
    let mut board = IssueBoard::default();
    refresh(&mut board, &inputs_at(&pods, &[], NOW));
    // A scope change reloads the pods list: for a while the board has no pods at all.
    let reloading = IssueInputs {
        pods: None,
        ..inputs_at(&pods, &[], NOW + 30)
    };
    refresh(&mut board, &reloading);
    assert!(board.issues.is_empty());
    let grace = crate::issue_rules::UNSCHEDULABLE_GRACE.as_secs();
    refresh(&mut board, &inputs_at(&pods, &[], NOW + grace));
    // The age counts from the first sighting, not from the reload.
    assert_eq!(board.issues.len(), 1);
    assert_eq!(board.issues[0].since, at(NOW));
}

#[test]
fn first_seen_kept_while_events_reload() {
    let mut created = burst("ReplicaSet", "api-7d9f8c");
    created.reason = "FailedCreate".to_owned();
    // An event without a first-seen time has no onset, so its age is the board's first sighting.
    created.first_seen = None;
    let events = feed(vec![created]);
    let with_events = |now| IssueInputs {
        events: Some(&events),
        ..inputs_at(&[], &[], now)
    };
    let mut board = IssueBoard::default();
    refresh(&mut board, &with_events(NOW));
    assert_eq!(board.issues[0].since, at(NOW));
    // The events watch restarts after a scope change.
    refresh(&mut board, &inputs_at(&[], &[], NOW + 30));
    assert!(board.issues.is_empty());
    refresh(&mut board, &with_events(NOW + 60));
    assert_eq!(board.issues[0].since, at(NOW));
}

#[test]
fn first_seen_is_dropped_when_the_feed_is_loaded_and_the_problem_is_gone() {
    let pods = [pending_pod(None)];
    let mut board = IssueBoard::default();
    refresh(&mut board, &inputs_at(&pods, &[], NOW));
    refresh(&mut board, &inputs_at(&[], &[], NOW + 30));
    assert!(board.first_seen.is_empty());
}

#[test]
fn count_for_screen_uses_reveal_target() {
    let mut standalone = [
        crashing_pod("a-0", 600, 1),
        unready_pod("b-0", 600),
        crashing_pod("c-0", 600, 1),
    ];
    for pod in &mut standalone {
        pod.controller = None;
    }
    let nodes = [node("node-a", NodeReadiness::NotReady)];
    let mut board = IssueBoard::default();
    refresh(&mut board, &inputs_at(&standalone, &nodes, NOW));
    // Two crashing pods (Critical) and one unready pod (Warning) all reveal on Pods.
    assert_eq!(
        board.count_for(Screen::Pods),
        Some((3, IssueSeverity::Critical))
    );
    assert_eq!(
        board.count_for(Screen::Nodes),
        Some((1, IssueSeverity::Critical))
    );
    let deployments = Screen::Kind(ResourceKind::Deployments);
    assert_eq!(board.count_for(deployments), None);
    // A workload of two pods reveals on its kind's screen instead.
    let pods = [
        crashing_pod("api-7d9f8c-a", 600, 1),
        crashing_pod("api-7d9f8c-b", 600, 1),
    ];
    refresh(&mut board, &inputs(&pods));
    assert_eq!(board.count_for(Screen::Pods), None);
    assert_eq!(
        board.count_for(deployments),
        Some((1, IssueSeverity::Critical))
    );
}

/// 1,000 pods with 50 problems, 2,000 events, and 1,000 Deployments. Run with
/// `cargo test --release -p k8sboard issue_evaluation_budget -- --ignored`; it fails above the 4 ms
/// budget of the spec and names the time then.
#[test]
#[ignore = "measures the release build"]
fn issue_evaluation_budget() {
    let pods: Vec<PodSummary> = (0..1_000)
        .map(|index| {
            let name = format!("web-{index:04}");
            let mut pod = if index % 20 == 0 {
                crashing_pod(&name, 600, 3)
            } else {
                pod_of("shop", &name, Some(("ReplicaSet", "web-7d9f8c")))
            };
            pod.name = name;
            pod
        })
        .collect();
    // Most of a real cluster's warnings are single events; 50 pods have a burst.
    let events: Vec<EventSummary> = (0..2_000)
        .map(|index| {
            let mut event = burst("Pod", &format!("web-{:04}", index % 1_000));
            if index >= 50 {
                event.count = 1;
            }
            event
        })
        .collect();
    let feed = feed(events);
    let nodes = [node("node-a", NodeReadiness::Ready)];
    // A thousand Deployments, one in fifty stalled.
    let deployments: Vec<KindObject> = (0..1_000)
        .map(|index| {
            let KindObject::Deployment(mut deployment) = stalled_api() else {
                unreachable!("stalled_api builds a Deployment");
            };
            deployment.name = format!("web-{index:04}");
            if index % 50 != 0 {
                deployment.conditions.clear();
            }
            KindObject::Deployment(deployment)
        })
        .collect();
    let feeds = [(ResourceKind::Deployments, &deployments[..])];
    let run = IssueInputs {
        events: Some(&feed),
        objects: &feeds,
        ..inputs_at(&pods, &nodes, NOW)
    };
    let mut board = IssueBoard::default();
    let started = std::time::Instant::now();
    refresh(&mut board, &run);
    let elapsed = started.elapsed();
    assert!(!board.issues.is_empty());
    assert!(
        elapsed <= std::time::Duration::from_millis(4),
        "{} issues took {elapsed:?}",
        board.issues.len()
    );
}

// ---- condition feeds ----

fn stalled_api() -> KindObject {
    KindObject::Deployment(cluster::DeploymentSummary {
        namespace: "shop".to_owned(),
        name: "api".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 3,
        ready: 1,
        up_to_date: 1,
        available: 1,
        strategy: String::new(),
        max_surge: None,
        max_unavailable: None,
        progress_deadline_seconds: 600,
        is_paused: false,
        revision: None,
        selector: Vec::new(),
        containers: Vec::new(),
        conditions: vec![cluster::WorkloadCondition {
            name: "Progressing".to_owned(),
            is_true: false,
            reason: Some("ProgressDeadlineExceeded".to_owned()),
            message: None,
        }],
    })
}

#[test]
fn pod_group_hides_rollout_stalled_of_same_deployment() {
    let pods = [crashing_pod("api-7d9f8c-a", 600, 3)];
    let objects = [stalled_api()];
    let feeds = [(ResourceKind::Deployments, &objects[..])];
    let issues = issues_of(&IssueInputs {
        objects: &feeds,
        ..inputs(&pods)
    });
    let [issue] = issues.as_slice() else {
        panic!("one issue, got {issues:?}");
    };
    // The crash loop is the cause; the stalled rollout restates it for the same Deployment.
    assert_eq!(issue.key.rule, IssueRule::PodCrash);
}

#[test]
fn rollout_stalled_without_pod_problems_shows() {
    let pods = [pod_of(
        "shop",
        "api-7d9f8c-a",
        Some(("ReplicaSet", "api-7d9f8c")),
    )];
    let objects = [stalled_api()];
    let feeds = [(ResourceKind::Deployments, &objects[..])];
    let issues = issues_of(&IssueInputs {
        objects: &feeds,
        ..inputs(&pods)
    });
    let [issue] = issues.as_slice() else {
        panic!("one issue, got {issues:?}");
    };
    assert_eq!(issue.key.rule, IssueRule::KindRollout);
    assert_eq!(issue.severity, IssueSeverity::Critical);
    assert_eq!(issue.reason, "Rollout stalled");
    assert_eq!(issue.target, issue.shown.target());
}

#[test]
fn first_seen_kept_while_a_condition_feed_reloads() {
    let objects = [stalled_api()];
    let feeds = [(ResourceKind::Deployments, &objects[..])];
    let with_feed = |now| IssueInputs {
        objects: &feeds,
        ..inputs_at(&[], &[], now)
    };
    let mut board = IssueBoard::default();
    refresh(&mut board, &with_feed(NOW));
    assert_eq!(board.issues[0].since, at(NOW));
    // The feed restarts after a scope change; the other lists are up.
    refresh(&mut board, &inputs_at(&[], &[], NOW + 30));
    assert!(board.issues.is_empty());
    refresh(&mut board, &with_feed(NOW + 60));
    assert_eq!(board.issues[0].since, at(NOW));
}
