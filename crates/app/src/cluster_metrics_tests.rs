use cluster::{AccessReport, AccessReview};

use super::*;

fn access(namespace: Option<&str>, reason: Option<&str>, is_allowed: bool) -> NamespaceAccess {
    NamespaceAccess {
        namespace: namespace.map(str::to_owned),
        decision: if is_allowed {
            AccessDecision::Allowed
        } else {
            AccessDecision::Denied {
                reason: reason.map(str::to_owned),
            }
        },
    }
}

fn several(names: &[&str]) -> NamespaceScope {
    NamespaceScope::of_namespaces(names.iter().map(|name| (*name).to_owned()))
}

#[test]
fn pods_gate_waits_while_the_review_runs() {
    assert_eq!(pods_gate(None, &NamespaceScope::All), PodsGate::Wait);
}

#[test]
fn pods_gate_polls_everything_when_every_namespace_is_allowed() {
    let review = Ok(vec![
        access(Some("a"), None, true),
        access(Some("b"), None, true),
    ]);
    let scope = several(&["a", "b"]);
    assert_eq!(
        pods_gate(Some(&review), &scope),
        PodsGate::Poll {
            scope: scope.clone(),
            note: None
        }
    );
    let all = Ok(vec![access(None, None, true)]);
    assert_eq!(
        pods_gate(Some(&all), &NamespaceScope::All),
        PodsGate::Poll {
            scope: NamespaceScope::All,
            note: None
        }
    );
}

#[test]
fn gate_polls_allowed_namespaces_only() {
    let review = Ok(vec![
        access(Some("a"), None, true),
        access(Some("b"), Some("forbidden"), false),
        access(Some("c"), None, true),
    ]);
    assert_eq!(
        pods_gate(Some(&review), &several(&["a", "b", "c"])),
        PodsGate::Poll {
            scope: several(&["a", "c"]),
            note: Some("no access in b".to_owned()),
        }
    );
    let one_left = Ok(vec![
        access(Some("a"), None, true),
        access(Some("b"), None, false),
    ]);
    assert_eq!(
        pods_gate(Some(&one_left), &several(&["a", "b"])),
        PodsGate::Poll {
            scope: NamespaceScope::Named("a".to_owned()),
            note: Some("no access in b".to_owned()),
        }
    );
}

#[test]
fn pods_gate_is_off_when_no_namespace_is_allowed() {
    let review = Ok(vec![
        access(Some("a"), None, false),
        access(Some("b"), Some("no rule for b"), false),
    ]);
    let PodsGate::Off(reason) = pods_gate(Some(&review), &several(&["a", "b"])) else {
        panic!("every namespace is denied");
    };
    assert_eq!(
        reason,
        "not allowed to list pods.metrics.k8s.io in a, b: no rule for b"
    );
    let all = Ok(vec![access(None, None, false)]);
    assert_eq!(
        pods_gate(Some(&all), &NamespaceScope::All),
        PodsGate::Off("not allowed to list pods.metrics.k8s.io in all namespaces".to_owned())
    );
}

#[test]
fn pods_gate_polls_after_a_failed_review() {
    let failed = Err("review failed".to_owned());
    assert_eq!(
        pods_gate(Some(&failed), &several(&["a", "b"])),
        PodsGate::Poll {
            scope: several(&["a", "b"]),
            note: None
        }
    );
}

fn report(check: AccessCheck, decision: AccessDecision) -> AccessState {
    AccessState::Known(AccessReport {
        reviews: vec![AccessReview { check, decision }],
    })
}

#[test]
fn nodes_gate_follows_the_session_review() {
    let check = AccessCheck::ListNodeMetrics;
    let checking = AccessState::Checking {
        _task: Task::ready(()),
    };
    assert_eq!(nodes_gate(&checking, check), NodesGate::Wait);
    let denied = report(
        check,
        AccessDecision::Denied {
            reason: Some("no rule".to_owned()),
        },
    );
    assert_eq!(
        nodes_gate(&denied, check),
        NodesGate::Off("not allowed to list nodes.metrics.k8s.io: no rule".to_owned())
    );
    let denied_silently = report(check, AccessDecision::Denied { reason: None });
    assert_eq!(
        nodes_gate(&denied_silently, check),
        NodesGate::Off("not allowed to list nodes.metrics.k8s.io".to_owned())
    );
    assert_eq!(
        nodes_gate(&report(check, AccessDecision::Allowed), check),
        NodesGate::Poll
    );
    assert_eq!(nodes_gate(&AccessState::Unknown, check), NodesGate::Poll);
    // Another check's denial does not matter.
    let other = report(
        AccessCheck::ListPods,
        AccessDecision::Denied { reason: None },
    );
    assert_eq!(nodes_gate(&other, check), NodesGate::Poll);
}

fn api_error(code: u16) -> ClusterError {
    ClusterError::Api {
        context: "ctx".to_owned(),
        action: "listing pod metrics",
        code,
        message: "boom".to_owned(),
    }
}

#[test]
fn poll_error_text_names_missing_or_broken_metrics_server() {
    assert_eq!(
        poll_error_text(&api_error(404)),
        "metrics-server is not installed: the cluster does not serve metrics.k8s.io"
    );
    let broken = poll_error_text(&api_error(503));
    assert!(
        broken.starts_with("metrics.k8s.io does not answer: "),
        "{broken}"
    );
    assert!(broken.contains("HTTP 503"), "{broken}");
    let other = poll_error_text(&api_error(500));
    assert!(other.contains("HTTP 500"), "{other}");
    assert!(!other.contains("not installed"), "{other}");
}

#[test]
fn metrics_settle_after_min_ticks_or_unavailable() {
    assert!(!is_metrics_settled(&FeedStatus::Checking, 0, 1));
    assert!(!is_metrics_settled(&FeedStatus::Waiting, 0, 1));
    assert!(is_metrics_settled(&FeedStatus::Live, 1, 1));
    assert!(!is_metrics_settled(&FeedStatus::Live, 1, 2));
    assert!(is_metrics_settled(
        &FeedStatus::Unavailable("no".to_owned()),
        0,
        2
    ));
    assert!(is_metrics_settled(
        &FeedStatus::Failed("no".to_owned()),
        0,
        2
    ));
    assert!(is_metrics_settled(
        &FeedStatus::Interrupted("no".to_owned()),
        0,
        2
    ));
}

fn pod_feed() -> MetricsFeed<PodUsageHistory> {
    MetricsFeed::new("pod metrics")
}

#[test]
fn feed_starts_checking_and_a_stopped_feed_keeps_checking() {
    let mut feed = pod_feed();
    assert_eq!(feed.status, FeedStatus::Checking);
    feed.turn_off("denied".to_owned());
    assert_eq!(feed.status, FeedStatus::Unavailable("denied".to_owned()));
    feed.wait();
    assert_eq!(feed.status, FeedStatus::Checking);
}

#[test]
fn failed_poll_is_failed_before_any_sample_and_interrupted_after() {
    let mut feed = pod_feed();
    feed.receive(WatchUpdate::Failed(api_error(404)), &[]);
    assert!(
        matches!(feed.status, FeedStatus::Failed(_)),
        "{:?}",
        feed.status
    );
    feed.receive(WatchUpdate::Snapshot(Vec::new()), &[]);
    assert_eq!(feed.status, FeedStatus::Live);
    assert_eq!(feed.history.tick_count(), 1);
    feed.receive(WatchUpdate::Failed(api_error(503)), &[]);
    assert!(
        matches!(feed.status, FeedStatus::Interrupted(_)),
        "{:?}",
        feed.status
    );
    assert_eq!(feed.history.tick_count(), 1, "a failed poll adds no tick");
}

#[test]
fn closed_stream_marks_the_feed_failed() {
    let mut feed: MetricsFeed<NodeUsageHistory> = MetricsFeed::new("node metrics");
    feed.mark_stopped();
    assert_eq!(
        feed.status,
        FeedStatus::Failed("metrics polling stopped unexpectedly".to_owned())
    );
}

#[test]
fn kubelet_gate_reads_the_node_proxy_review() {
    let denied = report(
        AccessCheck::GetNodeProxy,
        AccessDecision::Denied {
            reason: Some("no rule".to_owned()),
        },
    );
    let NodesGate::Off(reason) = nodes_gate(&denied, AccessCheck::GetNodeProxy) else {
        panic!("a denied review turns the kubelet feed off");
    };
    assert!(
        reason.starts_with("not allowed to get nodes/proxy"),
        "{reason}"
    );
    assert!(reason.ends_with(": no rule"), "{reason}");
    // A denial of the metrics API does not stop the kubelet feed.
    let metrics_denied = report(
        AccessCheck::ListNodeMetrics,
        AccessDecision::Denied { reason: None },
    );
    assert_eq!(
        nodes_gate(&metrics_denied, AccessCheck::GetNodeProxy),
        NodesGate::Poll
    );
}
