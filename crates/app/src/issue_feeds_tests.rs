use cluster::{ClusterError, EventType, InvolvedObject};

use super::*;

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

fn event(kind: &str, namespace: &str, name: &str, reason: &str, last_seen: i64) -> EventSummary {
    EventSummary {
        namespace: namespace.to_owned(),
        name: format!("{name}.{reason}"),
        event_type: EventType::Warning,
        reason: reason.to_owned(),
        object: InvolvedObject {
            kind: kind.to_owned(),
            namespace: Some(namespace.to_owned()),
            name: name.to_owned(),
        },
        message: String::new(),
        count: 1,
        first_seen: Some(at(last_seen)),
        last_seen: Some(at(last_seen)),
        source: None,
        container: None,
    }
}

fn reasons(events: &[EventSummary]) -> Vec<&str> {
    events.iter().map(|event| event.reason.as_str()).collect()
}

#[test]
fn warning_events_slice_per_object() {
    let mut feed = WarningEvents::default();
    feed.apply(WatchUpdate::Snapshot(vec![
        event("Pod", "shop", "web-1", "Unhealthy", 10),
        event("Node", "default", "node-a", "NodeNotReady", 50),
        event("Pod", "shop", "web-0", "BackOff", 20),
        event("Pod", "shop", "web-1", "Failed", 30),
        event("Pod", "admin", "web-1", "Evicted", 40),
    ]));
    // Newest first within one object.
    let web_1 = feed.of(&IssueObject::pod("shop", "web-1")).expect("ready");
    assert_eq!(reasons(web_1), ["Failed", "Unhealthy"]);
    let web_0 = feed.of(&IssueObject::pod("shop", "web-0")).expect("ready");
    assert_eq!(reasons(web_0), ["BackOff"]);
    // The same name in another namespace is another object.
    let other = feed.of(&IssueObject::pod("admin", "web-1")).expect("ready");
    assert_eq!(reasons(other), ["Evicted"]);
    // A ready feed with no events for an object says so.
    assert_eq!(feed.of(&IssueObject::pod("shop", "web-9")), Some(&[][..]));
    assert_eq!(feed.recent().count(), 5);
}

#[test]
fn warning_events_of_none_until_ready() {
    let mut feed = WarningEvents::default();
    assert_eq!(feed.of(&IssueObject::pod("shop", "web-0")), None);
    assert_eq!(feed.state(), FeedState::Loading);
    feed.apply(WatchUpdate::Failed(ClusterError::TimedOut {
        context: "ctx".to_owned(),
        action: "watching events",
    }));
    assert_eq!(feed.of(&IssueObject::pod("shop", "web-0")), None);
    assert!(matches!(feed.state(), FeedState::Off(_)));
    feed.apply(WatchUpdate::Snapshot(Vec::new()));
    assert_eq!(feed.of(&IssueObject::pod("shop", "web-0")), Some(&[][..]));
    assert_eq!(feed.state(), FeedState::Live);
}

#[test]
fn snapshot_replaces_the_index() {
    let mut feed = WarningEvents::default();
    feed.apply(WatchUpdate::Snapshot(vec![event(
        "Pod", "shop", "web-0", "BackOff", 10,
    )]));
    feed.apply(WatchUpdate::Snapshot(vec![event(
        "Pod", "shop", "web-1", "BackOff", 10,
    )]));
    assert_eq!(feed.of(&IssueObject::pod("shop", "web-0")), Some(&[][..]));
    assert_eq!(
        feed.of(&IssueObject::pod("shop", "web-1")).map(<[_]>::len),
        Some(1)
    );
}

fn coverage(feeds: &[(IssueFeed, FeedState)]) -> Coverage {
    Coverage {
        feeds: feeds.to_vec(),
    }
}

#[test]
fn coverage_note_groups_by_state() {
    let coverage = coverage(&[
        (IssueFeed::Pods, FeedState::Live),
        (
            IssueFeed::PodMetrics,
            FeedState::Off("not permitted: list pods.metrics.k8s.io".to_owned()),
        ),
        (IssueFeed::WarningEvents, FeedState::Loading),
        (IssueFeed::NodeMetrics, FeedState::Loading),
        (
            IssueFeed::VolumeUsage,
            FeedState::Limited("10 of 42 nodes polled".to_owned()),
        ),
    ]);
    assert!(coverage.is_partial());
    assert_eq!(
        coverage.note().as_deref(),
        Some(
            "Not checked: pod metrics (not permitted: list pods.metrics.k8s.io). \
             Loading: warning events, node metrics. \
             Volume usage: 10 of 42 nodes polled."
        )
    );
}

#[test]
fn coverage_full_has_no_note() {
    let live = coverage(&[
        (IssueFeed::Pods, FeedState::Live),
        (IssueFeed::Nodes, FeedState::Live),
    ]);
    assert!(!live.is_partial());
    assert_eq!(live.note(), None);
    assert!(!Coverage::default().is_partial());
}

#[test]
fn limited_feed_is_not_partial() {
    let limited = coverage(&[
        (IssueFeed::Pods, FeedState::Live),
        (
            IssueFeed::VolumeUsage,
            FeedState::Limited("3 of 12 nodes polled".to_owned()),
        ),
    ]);
    assert!(!limited.is_partial());
    // The limit is still worth saying.
    assert_eq!(
        limited.note().as_deref(),
        Some("Volume usage: 3 of 12 nodes polled.")
    );
}

#[test]
fn core_feed_states_from_lists_and_metrics() {
    let ready = LiveList::Ready {
        items: vec![1],
        interruption: None,
    };
    let failed = LiveList::<u32>::Failed {
        message: "forbidden".to_owned(),
    };
    assert_eq!(list_state(&LiveList::<u32>::Loading), FeedState::Loading);
    assert_eq!(list_state(&ready), FeedState::Live);
    assert_eq!(list_state(&failed), FeedState::Off("forbidden".to_owned()));
    // An interruption keeps the stale data, so the list still counts as live.
    let interrupted = LiveList::Ready {
        items: vec![1],
        interruption: Some("timeout".to_owned()),
    };
    assert_eq!(list_state(&interrupted), FeedState::Live);

    let off = |reason: &str| FeedState::Off(reason.to_owned());
    assert_eq!(metrics_state(&FeedStatus::Live), FeedState::Live);
    assert_eq!(
        metrics_state(&FeedStatus::Interrupted("slow".to_owned())),
        FeedState::Live
    );
    assert_eq!(metrics_state(&FeedStatus::Checking), FeedState::Loading);
    assert_eq!(metrics_state(&FeedStatus::Waiting), FeedState::Loading);
    assert_eq!(
        metrics_state(&FeedStatus::Unavailable("denied".to_owned())),
        off("denied")
    );
    assert_eq!(
        metrics_state(&FeedStatus::Failed("down".to_owned())),
        off("down")
    );
}

#[test]
fn volume_usage_is_limited_when_fewer_nodes_are_polled() {
    assert_eq!(
        volume_usage_state(&FeedStatus::Live, 10, 10),
        FeedState::Live
    );
    assert_eq!(
        volume_usage_state(&FeedStatus::Live, 0, 42),
        FeedState::Limited("0 of 42 nodes polled".to_owned())
    );
    // With no node to poll, no round ever comes: waiting is limited, not loading.
    assert_eq!(
        volume_usage_state(&FeedStatus::Waiting, 0, 42),
        FeedState::Limited("0 of 42 nodes polled".to_owned())
    );
    // Every node polled and the first round pending is still loading.
    assert_eq!(
        volume_usage_state(&FeedStatus::Waiting, 4, 4),
        FeedState::Loading
    );
    // A review that has not finished, or a denial, is not a limit.
    assert_eq!(
        volume_usage_state(&FeedStatus::Checking, 0, 42),
        FeedState::Loading
    );
    assert_eq!(
        volume_usage_state(&FeedStatus::Unavailable("denied".to_owned()), 0, 42),
        FeedState::Off("denied".to_owned())
    );
}
