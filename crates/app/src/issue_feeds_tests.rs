use cluster::{ClusterError, EventType, InvolvedObject};

use super::*;
use crate::cluster_session::scope_multiplicity;

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

// ---- condition feeds ----

use cluster::{AccessCheck, AccessDecision, AccessReport, AccessReview, DeploymentSummary};

fn report_denying(denied: &[AccessCheck]) -> AccessState {
    let reviews = AccessCheck::ALL
        .into_iter()
        .map(|check| AccessReview {
            check,
            decision: if denied.contains(&check) {
                AccessDecision::Denied { reason: None }
            } else {
                AccessDecision::Allowed
            },
        })
        .collect();
    AccessState::Known(AccessReport { reviews })
}

fn scope_of(names: &[&str]) -> NamespaceScope {
    NamespaceScope::of_namespaces(names.iter().map(|name| (*name).to_owned()))
}

fn plan_for(scope: &NamespaceScope, access: &AccessState) -> Vec<FeedPlan> {
    condition_plan(scope, access, CertificateWatch::Watch)
        .into_iter()
        .map(|(_, plan)| plan)
        .collect()
}

#[test]
fn condition_plan_uses_scope_up_to_two_namespaces() {
    let access = report_denying(&[]);
    for scope in [NamespaceScope::All, scope_of(&["a"]), scope_of(&["a", "b"])] {
        let plans = plan_for(&scope, &access);
        assert_eq!(plans.len(), CONDITION_KINDS.len());
        assert!(plans.iter().all(|plan| *plan
            == FeedPlan::Start {
                watch_scope: scope.clone()
            }));
    }
}

#[test]
fn condition_plan_uses_all_scope_above_two() {
    let access = report_denying(&[]);
    let plans = plan_for(&scope_of(&["a", "b", "c"]), &access);
    assert!(plans.iter().all(|plan| *plan
        == FeedPlan::Start {
            watch_scope: NamespaceScope::All
        }));
}

#[test]
fn condition_plan_waits_for_review() {
    let checking = AccessState::Checking {
        _task: gpui_kit::Task::ready(()),
    };
    assert!(
        plan_for(&scope_of(&["a"]), &checking)
            .iter()
            .all(|plan| *plan == FeedPlan::Wait)
    );
    // A review that failed starts the feeds: each shows its own error.
    let plans = plan_for(&scope_of(&["a"]), &AccessState::Unknown);
    assert!(
        plans
            .iter()
            .all(|plan| matches!(plan, FeedPlan::Start { .. }))
    );
}

#[test]
fn condition_plan_off_when_denied() {
    let access = report_denying(&[AccessCheck::ListSecrets, AccessCheck::ListJobs]);
    let plans: Vec<_> = condition_plan(&scope_of(&["a", "b"]), &access, CertificateWatch::Watch);
    let plan_of = |kind| {
        plans
            .iter()
            .find(|(listed, _)| *listed == kind)
            .map(|(_, plan)| plan.clone())
            .expect("planned")
    };
    // A denied TLS secret list is the secrets check.
    assert_eq!(
        plan_of(ResourceKind::Secrets),
        FeedPlan::Off("not permitted: list secrets".to_owned())
    );
    assert_eq!(
        plan_of(ResourceKind::Jobs),
        FeedPlan::Off("not permitted: list jobs".to_owned())
    );
    assert!(matches!(
        plan_of(ResourceKind::Deployments),
        FeedPlan::Start { .. }
    ));
    // Above two namespaces the denial may be of one namespace only: the cluster-wide watch finds out.
    let wide = condition_plan(
        &scope_of(&["a", "b", "c"]),
        &access,
        CertificateWatch::Watch,
    );
    assert!(
        wide.iter()
            .all(|(_, plan)| matches!(plan, FeedPlan::Start { .. }))
    );
}

#[test]
fn tls_watch_off_turns_the_secrets_feed_off() {
    let plans = condition_plan(
        &scope_of(&["a"]),
        &report_denying(&[]),
        CertificateWatch::Skip,
    );
    for (kind, plan) in plans {
        if kind == ResourceKind::Secrets {
            assert_eq!(plan, FeedPlan::Off("off in Settings".to_owned()));
        } else {
            assert!(matches!(plan, FeedPlan::Start { .. }), "{kind:?}");
        }
    }
    // The setting wins over a review that is still running.
    let checking = AccessState::Checking {
        _task: gpui_kit::Task::ready(()),
    };
    let waiting = condition_plan(&scope_of(&["a"]), &checking, CertificateWatch::Skip);
    assert!(waiting.iter().any(|(kind, plan)| {
        *kind == ResourceKind::Secrets && matches!(plan, FeedPlan::Off(_))
    }));
}

#[test]
fn tls_watch_off_is_named_in_coverage() {
    let reason = condition_plan(
        &scope_of(&["a"]),
        &report_denying(&[]),
        CertificateWatch::Skip,
    )
    .into_iter()
    .find_map(|(kind, plan)| match plan {
        FeedPlan::Off(reason) if kind == ResourceKind::Secrets => Some(reason),
        _ => None,
    });
    let feed = ConditionFeed::idle(ResourceKind::Secrets, reason);
    let coverage = Coverage {
        feeds: vec![(IssueFeed::Kind(ResourceKind::Secrets), feed.state())],
    };
    let note = coverage.note();
    assert!(
        note.as_deref()
            .is_some_and(|text| text.contains("Not checked: certificates (off in Settings).")),
        "{note:?}"
    );
}

#[gpui_kit::test]
fn certificate_watch_follows_the_saved_choice(cx: &mut gpui_kit::TestAppContext) {
    use crate::settings::{GeneralSettings, Settings};
    use crate::settings_store::{LoadedSettings, WriteMode};
    cx.update(|cx| {
        assert_eq!(CertificateWatch::of(cx), CertificateWatch::Watch);
        let settings = Settings {
            general: GeneralSettings {
                watch_tls_secrets: false,
                ..GeneralSettings::default()
            },
            ..Settings::default()
        };
        AppSettings::install(
            LoadedSettings {
                settings,
                writes: WriteMode::Disabled,
                notice: None,
            },
            cx,
        );
        assert_eq!(CertificateWatch::of(cx), CertificateWatch::Skip);
    });
}

fn deployment_in(namespace: &str) -> DeploymentSummary {
    DeploymentSummary {
        namespace: namespace.to_owned(),
        name: "api".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 1,
        ready: 1,
        up_to_date: 1,
        available: 1,
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

#[test]
fn all_scope_feed_drops_rows_outside_scope() {
    let snapshot = || {
        WatchUpdate::Snapshot(vec![
            deployment_in("a"),
            deployment_in("z"),
            deployment_in("b"),
        ])
    };
    let wrapped = |keep: Option<Vec<String>>| {
        let updates = futures::stream::iter([snapshot()]);
        let stream = objects(
            updates,
            keep,
            |item| &item.namespace,
            KindObject::Deployment,
        );
        futures::executor::block_on(stream.collect::<Vec<_>>())
    };
    let namespaces = |updates: Vec<WatchUpdate<KindObject>>| -> Vec<String> {
        match updates.into_iter().next() {
            Some(WatchUpdate::Snapshot(items)) => items
                .into_iter()
                .filter_map(|item| match item {
                    KindObject::Deployment(deployment) => Some(deployment.namespace),
                    _ => None,
                })
                .collect(),
            _ => panic!("a snapshot"),
        }
    };
    assert_eq!(
        namespaces(wrapped(Some(vec![
            "a".to_owned(),
            "b".to_owned(),
            "c".to_owned()
        ]))),
        ["a", "b"]
    );
    assert_eq!(namespaces(wrapped(None)), ["a", "z", "b"]);
}

fn forbidden() -> WatchUpdate<KindObject> {
    WatchUpdate::Failed(ClusterError::Forbidden {
        context: "ctx".to_owned(),
        action: "watching deployments",
        message: "no".to_owned(),
    })
}

#[test]
fn forbidden_all_scope_feed_turns_off() {
    let mut feed = ConditionFeed::idle(ResourceKind::Deployments, None);
    feed.watch_scope = Some(NamespaceScope::All);
    feed.apply(forbidden());
    assert_eq!(
        feed.state(),
        FeedState::Off("not permitted cluster-wide".to_owned())
    );
    assert_eq!(feed.watches(), 0);
    // A watch of the session's own namespaces keeps retrying, so its failure shows as a list's.
    let mut narrow = ConditionFeed::idle(ResourceKind::Deployments, None);
    narrow.watch_scope = Some(scope_of(&["a"]));
    narrow.apply(forbidden());
    assert!(
        matches!(narrow.state(), FeedState::Off(reason) if reason != "not permitted cluster-wide")
    );
}

#[test]
fn rollouts_label_once_in_the_note() {
    let off = |kind| {
        (
            IssueFeed::Kind(kind),
            FeedState::Off("not permitted".to_owned()),
        )
    };
    let coverage = Coverage {
        feeds: vec![
            off(ResourceKind::Deployments),
            off(ResourceKind::DaemonSets),
        ],
    };
    assert_eq!(
        coverage.note().as_deref(),
        Some("Not checked: rollouts (not permitted).")
    );
}

// ---- the watch count ----

use gpui_kit::{AppContext as _, TestAppContext};

/// A view that owns subscriptions and does nothing with their updates.
struct Probe;

fn idle_subscription(runtime: &ClusterRuntime, cx: &mut Context<Probe>) -> WatchSubscription {
    runtime.subscribe_silent(
        futures::stream::pending::<()>(),
        cx,
        |_: &mut Probe, (), _| {},
        |_, _| {},
    )
}

/// Feeds as `restart_conditions` leaves them for `scope`: a running watch for every planned
/// start, an idle entry for the rest.
fn planned_feeds(
    runtime: &ClusterRuntime,
    scope: &NamespaceScope,
    cx: &mut Context<Probe>,
) -> IssueFeeds {
    let conditions = condition_plan(scope, &AccessState::Unknown, CertificateWatch::Watch)
        .into_iter()
        .map(|(kind, plan)| match plan {
            FeedPlan::Start { watch_scope } => ConditionFeed {
                watch_scope: Some(watch_scope),
                subscription: Some(idle_subscription(runtime, cx)),
                ..ConditionFeed::idle(kind, None)
            },
            FeedPlan::Wait | FeedPlan::Off(_) => ConditionFeed::idle(kind, None),
        })
        .collect();
    IssueFeeds {
        events: WarningEvents::default(),
        events_watch: Some(idle_subscription(runtime, cx)),
        events_restart: None,
        conditions,
    }
}

#[gpui_kit::test]
fn watch_count_follows_the_condition_plan(cx: &mut TestAppContext) {
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a runtime");
    let runtime = ClusterRuntime::new(tokio.handle().clone());
    // (namespaces, expected): N events and 8 kinds over N namespaces up to two, else one
    // cluster-wide watch per kind.
    for (names, expected) in [(1, 9), (2, 18), (3, 11)] {
        let scope = NamespaceScope::of_namespaces((0..names).map(|index| format!("n{index}")));
        let multiplicity = scope_multiplicity(&scope);
        let probe = cx.update(|cx| {
            cx.new(|cx| {
                let feeds = planned_feeds(&runtime, &scope, cx);
                assert_eq!(feeds.watch_count(multiplicity), expected, "N = {names}");
                Probe
            })
        });
        drop(probe);
    }
    // TLS watch off: the Secrets feed is one watch fewer.
    let skipped = cx.update(|cx| {
        cx.new(|cx| {
            let scope = NamespaceScope::All;
            let conditions = condition_plan(&scope, &AccessState::Unknown, CertificateWatch::Skip)
                .into_iter()
                .map(|(kind, plan)| match plan {
                    FeedPlan::Start { watch_scope } => ConditionFeed {
                        watch_scope: Some(watch_scope),
                        subscription: Some(idle_subscription(&runtime, cx)),
                        ..ConditionFeed::idle(kind, None)
                    },
                    FeedPlan::Wait | FeedPlan::Off(_) => ConditionFeed::idle(kind, None),
                })
                .collect();
            let feeds = IssueFeeds {
                events: WarningEvents::default(),
                events_watch: None,
                events_restart: None,
                conditions,
            };
            assert_eq!(feeds.watch_count(1), 7);
            Probe
        })
    });
    drop(skipped);
    // A restart that waits for its delay runs no events watch.
    let waiting = cx.update(|cx| {
        cx.new(|cx| {
            let mut feeds = planned_feeds(&runtime, &NamespaceScope::All, cx);
            feeds.events_watch = None;
            assert_eq!(feeds.watch_count(1), 8);
            Probe
        })
    });
    drop(waiting);
}

#[test]
fn live_objects_exist_only_for_a_loaded_feed_that_runs() {
    let mut feed = ConditionFeed::idle(ResourceKind::Deployments, None);
    assert!(feed.live_objects().is_none(), "loading");
    feed.apply(WatchUpdate::Snapshot(vec![KindObject::Plain]));
    assert_eq!(feed.live_objects(), Some(&[KindObject::Plain][..]));
    let off = ConditionFeed::idle(ResourceKind::Deployments, Some("not permitted".to_owned()));
    assert!(off.live_objects().is_none(), "off");
    let mut failed = ConditionFeed::idle(ResourceKind::Deployments, None);
    failed.apply(forbidden());
    assert!(failed.live_objects().is_none(), "failed");
}

#[test]
fn the_palette_searches_only_the_live_feeds() {
    let mut live = ConditionFeed::idle(ResourceKind::Deployments, None);
    live.apply(WatchUpdate::Snapshot(vec![KindObject::Plain]));
    let feeds = IssueFeeds {
        events: WarningEvents::default(),
        events_watch: None,
        events_restart: None,
        conditions: vec![
            live,
            // Waiting for its access review.
            ConditionFeed::idle(ResourceKind::Jobs, None),
            ConditionFeed::idle(ResourceKind::Secrets, Some("off in Settings".to_owned())),
        ],
    };
    let searched: Vec<ResourceKind> = crate::palette_search::live_feed_objects(&feeds)
        .iter()
        .map(|feed| feed.kind)
        .collect();
    assert_eq!(searched, [ResourceKind::Deployments]);
}
