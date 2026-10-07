use cluster::{
    EventType, InvolvedObject, NamespacePhase, NodeCondition, NodeReadiness, NodeScheduling,
    NodeStatus, NodeSystemInfo,
};

use super::*;

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// 12:00 UTC.
fn now() -> Timestamp {
    at("2024-05-01T12:00:00Z")
}

fn event(kind: &str, namespace: Option<&str>, name: &str, last_seen: Option<&str>) -> EventSummary {
    EventSummary {
        namespace: namespace.unwrap_or("default").to_owned(),
        name: format!("{name}.1"),
        event_type: EventType::Normal,
        reason: "ScalingReplicaSet".to_owned(),
        object: InvolvedObject {
            kind: kind.to_owned(),
            namespace: namespace.map(str::to_owned),
            name: name.to_owned(),
        },
        message: "Scaled up replica set api-7d9f8c to 3".to_owned(),
        count: 1,
        first_seen: None,
        last_seen: last_seen.map(at),
        source: Some("deployment-controller".to_owned()),
        container: None,
    }
}

fn node(name: &str, created: &str) -> NodeSummary {
    NodeSummary {
        name: name.to_owned(),
        status: NodeStatus {
            readiness: NodeReadiness::Ready,
            scheduling: NodeScheduling::Enabled,
        },
        roles: Vec::new(),
        taints: Vec::new(),
        kubelet_version: "v1.29.5".to_owned(),
        internal_ip: None,
        created_at: Some(at(created)),
        conditions: Vec::new(),
        addresses: Vec::new(),
        system: NodeSystemInfo::default(),
        resources: Vec::new(),
        labels: Vec::new(),
    }
}

fn with_ready(mut node: NodeSummary, status: ConditionStatus, changed: &str) -> NodeSummary {
    node.conditions.push(NodeCondition {
        name: "Ready".to_owned(),
        status,
        reason: None,
        message: None,
        changed_at: Some(at(changed)),
        last_heartbeat_at: None,
    });
    node
}

fn namespace(name: &str, created: &str) -> NamespaceSummary {
    NamespaceSummary {
        name: name.to_owned(),
        phase: NamespacePhase::Active,
        labels: Vec::new(),
        created_at: Some(at(created)),
        deleting_since: None,
        deletion_conditions: Vec::new(),
    }
}

fn inputs<'a>() -> ChangeInputs<'a> {
    ChangeInputs {
        rollouts: None,
        rescales: None,
        nodes: None,
        namespaces: None,
        replica_sets: None,
        deployments: None,
        window: ChangeWindow::FifteenMinutes,
        now: now(),
    }
}

const RECENT: &str = "2024-05-01T11:55:00Z";
const OLD: &str = "2024-05-01T11:30:00Z";
const LONG_AGO: &str = "2024-01-01T00:00:00Z";

#[test]
fn rollout_event_becomes_deployment_entry() {
    let events = [event("Deployment", Some("payments"), "api", Some(RECENT))];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        ..inputs()
    });
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, ChangeKind::Deployment);
    assert_eq!(entries[0].object, "payments/api");
    assert_eq!(entries[0].text, "Scaled up replica set api-7d9f8c to 3");
    assert_eq!(entries[0].at, at(RECENT));
}

#[test]
fn rescale_event_reads_hpa() {
    let events = [event(
        "HorizontalPodAutoscaler",
        Some("payments"),
        "api",
        Some(RECENT),
    )];
    let entries = recent_changes(&ChangeInputs {
        rescales: Some(&events),
        ..inputs()
    });
    assert_eq!(entries[0].kind, ChangeKind::Autoscaler);
    assert_eq!(entries[0].kind.label(), "HPA");
}

#[test]
fn window_cuts_by_last_seen() {
    let events = [
        event("Deployment", Some("a"), "new", Some(RECENT)),
        event("Deployment", Some("a"), "old", Some(OLD)),
    ];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        ..inputs()
    });
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].object, "a/new");
}

#[test]
fn one_hour_window_keeps_older_events() {
    let events = [event("Deployment", Some("a"), "old", Some(OLD))];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        window: ChangeWindow::OneHour,
        ..inputs()
    });
    assert_eq!(entries.len(), 1);
}

#[test]
fn event_without_last_seen_is_skipped() {
    let events = [event("Deployment", Some("a"), "api", None)];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        ..inputs()
    });
    assert!(entries.is_empty());
}

#[test]
fn actor_is_source_before_on() {
    assert_eq!(
        actor_of("kubelet on ip-10-0-1-23").as_deref(),
        Some("kubelet")
    );
    assert_eq!(
        actor_of("horizontal-pod-autoscaler").as_deref(),
        Some("horizontal-pod-autoscaler")
    );
    assert_eq!(actor_of(""), None);
}

#[test]
fn node_ready_transition_in_window() {
    let nodes = [with_ready(
        node("n1", LONG_AGO),
        ConditionStatus::False,
        RECENT,
    )];
    let entries = recent_changes(&ChangeInputs {
        nodes: Some(&nodes),
        ..inputs()
    });
    assert_eq!(entries[0].text, "became NotReady");
    assert_eq!(entries[0].actor.as_deref(), Some("kubelet"));
    let ready = [with_ready(
        node("n1", LONG_AGO),
        ConditionStatus::True,
        RECENT,
    )];
    let entries = recent_changes(&ChangeInputs {
        nodes: Some(&ready),
        ..inputs()
    });
    assert_eq!(entries[0].text, "became Ready");
}

#[test]
fn node_unknown_reads_stopped_reporting() {
    let nodes = [with_ready(
        node("n1", LONG_AGO),
        ConditionStatus::Unknown,
        RECENT,
    )];
    let entries = recent_changes(&ChangeInputs {
        nodes: Some(&nodes),
        ..inputs()
    });
    assert_eq!(entries[0].text, "stopped reporting (Unknown)");
    assert_eq!(entries[0].actor.as_deref(), Some("node-controller"));
}

#[test]
fn joined_node_hides_its_ready_transition() {
    let nodes = [with_ready(
        node("n1", RECENT),
        ConditionStatus::True,
        "2024-05-01T11:56:00Z",
    )];
    let entries = recent_changes(&ChangeInputs {
        nodes: Some(&nodes),
        ..inputs()
    });
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text, "joined the cluster");
    assert_eq!(entries[0].actor, None);
}

#[test]
fn namespace_created_in_window() {
    let namespaces = [namespace("fresh", RECENT), namespace("ancient", LONG_AGO)];
    let entries = recent_changes(&ChangeInputs {
        namespaces: Some(&namespaces),
        ..inputs()
    });
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, ChangeKind::Namespace);
    assert_eq!(entries[0].text, "created");
}

#[test]
fn newest_first_ties_by_object() {
    let events = [
        event("Deployment", Some("a"), "zeta", Some(RECENT)),
        event("Deployment", Some("a"), "alpha", Some(RECENT)),
        event(
            "Deployment",
            Some("a"),
            "latest",
            Some("2024-05-01T11:59:00Z"),
        ),
    ];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        ..inputs()
    });
    let objects: Vec<&str> = entries.iter().map(|entry| entry.object.as_str()).collect();
    assert_eq!(objects, ["a/latest", "a/alpha", "a/zeta"]);
}

#[test]
fn targets_resolve_through_resource_key() {
    let events = [event("Deployment", Some("payments"), "api", Some(RECENT))];
    let nodes = [with_ready(
        node("n1", LONG_AGO),
        ConditionStatus::False,
        RECENT,
    )];
    let namespaces = [namespace("fresh", RECENT)];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        nodes: Some(&nodes),
        namespaces: Some(&namespaces),
        ..inputs()
    });
    let target = |kind| {
        entries
            .iter()
            .find(|entry| entry.kind == kind)
            .and_then(|entry| entry.target.clone())
    };
    assert_eq!(
        target(ChangeKind::Node),
        Some(ResourceKey::Node {
            name: "n1".to_owned()
        })
    );
    assert!(matches!(
        target(ChangeKind::Deployment),
        Some(ResourceKey::Kind { name, .. }) if name == "api"
    ));
    assert!(matches!(
        target(ChangeKind::Namespace),
        Some(ResourceKey::Kind { name, .. }) if name == "fresh"
    ));
    // An object without a screen has no target.
    let unknown = [event("Widget", Some("a"), "w", Some(RECENT))];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&unknown),
        ..inputs()
    });
    assert_eq!(entries[0].target, None);
}

#[test]
fn count_kept_for_aggregated_events() {
    let mut aggregated = event("Deployment", Some("a"), "api", Some(RECENT));
    aggregated.count = 5;
    let events = [aggregated];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        ..inputs()
    });
    assert_eq!(entries[0].count, 5);
}

#[test]
fn none_inputs_contribute_nothing() {
    assert!(recent_changes(&inputs()).is_empty());
}

#[test]
fn window_excludes_exact_start() {
    let events = [
        event(
            "Deployment",
            Some("a"),
            "edge",
            Some("2024-05-01T11:45:00Z"),
        ),
        event(
            "Deployment",
            Some("a"),
            "inside",
            Some("2024-05-01T11:45:01Z"),
        ),
    ];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        ..inputs()
    });
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].object, "a/inside");
}

#[test]
fn event_slightly_ahead_of_the_clock_is_recent() {
    let events = [event(
        "Deployment",
        Some("a"),
        "ahead",
        Some("2024-05-01T12:00:03Z"),
    )];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        ..inputs()
    });
    assert_eq!(entries.len(), 1);
}

// ---- Spec 0041: the field manager of a rollout ----

fn deployment_written(manager: &str, written: &str) -> KindObject {
    let mut deployment = crate::workload_actions::workload_actions_tests::deployment("api");
    deployment.namespace = "payments".to_owned();
    deployment.template_change = Some(cluster::FieldWriter {
        manager: manager.to_owned(),
        at: at(written),
    });
    KindObject::Deployment(deployment)
}

/// The one rollout entry of `api` seen at `RECENT` (11:55:00Z), with the feed `deployments`.
fn rollout_entry(deployments: Option<&[KindObject]>) -> ChangeEntry {
    let events = [event("Deployment", Some("payments"), "api", Some(RECENT))];
    let mut entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        deployments,
        ..inputs()
    });
    assert_eq!(entries.len(), 1);
    entries.remove(0)
}

#[test]
fn rollout_actor_is_field_manager_in_window() {
    // 20 s before the event's last occurrence.
    let feed = [deployment_written("ci-bot", "2024-05-01T11:54:40Z")];
    let entry = rollout_entry(Some(&feed));
    assert_eq!(entry.actor.as_deref(), Some("ci-bot"));
    assert_eq!(entry.actor_source, ActorSource::FieldManager);
}

#[test]
fn field_manager_actor_tooltip_says_probably() {
    let feed = [deployment_written("ci-bot", "2024-05-01T11:54:40Z")];
    let tooltip = rollout_entry(Some(&feed)).tooltip();
    assert!(
        tooltip.ends_with("probably ci-bot · last pod-template writer (field manager)"),
        "{tooltip}"
    );
}

#[test]
fn rollout_actor_falls_back_outside_window() {
    // A write 2 minutes after the event cannot have caused it; one 40 minutes before is too old.
    for written in ["2024-05-01T11:57:00Z", "2024-05-01T11:15:00Z"] {
        let feed = [deployment_written("ci-bot", written)];
        let entry = rollout_entry(Some(&feed));
        assert_eq!(
            entry.actor.as_deref(),
            Some("deployment-controller"),
            "{written}"
        );
        assert_eq!(entry.actor_source, ActorSource::EventSource);
        assert!(
            entry.tooltip().ends_with("· event source"),
            "{}",
            entry.tooltip()
        );
    }
    // The edges of the window count: 30 minutes before and 60 seconds after.
    for written in ["2024-05-01T11:25:00Z", "2024-05-01T11:56:00Z"] {
        let feed = [deployment_written("ci-bot", written)];
        assert_eq!(
            rollout_entry(Some(&feed)).actor_source,
            ActorSource::FieldManager,
            "{written}"
        );
    }
}

#[test]
fn rollout_actor_without_feed_is_event_source() {
    let entry = rollout_entry(None);
    assert_eq!(entry.actor.as_deref(), Some("deployment-controller"));
    assert_eq!(entry.actor_source, ActorSource::EventSource);
    // A Deployment the feed does not hold, or one without a template writer, is the same.
    let other = [deployment_written("ci-bot", "2024-05-01T11:54:40Z")];
    let mut elsewhere = event("Deployment", Some("billing"), "api", Some(RECENT));
    elsewhere.source = Some("deployment-controller".to_owned());
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&[elsewhere]),
        deployments: Some(&other),
        ..inputs()
    });
    assert_eq!(entries[0].actor_source, ActorSource::EventSource);
    let mut unwritten = crate::workload_actions::workload_actions_tests::deployment("api");
    unwritten.namespace = "payments".to_owned();
    let feed = [KindObject::Deployment(unwritten)];
    assert_eq!(
        rollout_entry(Some(&feed)).actor_source,
        ActorSource::EventSource
    );
}

#[test]
fn other_rows_keep_their_actor() {
    let feed = [deployment_written("ci-bot", "2024-05-01T11:54:40Z")];
    let hpa = [event(
        "HorizontalPodAutoscaler",
        Some("payments"),
        "api",
        Some(RECENT),
    )];
    let nodes = [with_ready(
        node("wk-1", LONG_AGO),
        ConditionStatus::False,
        RECENT,
    )];
    let namespaces = [namespace("new-team", RECENT)];
    let entries = recent_changes(&ChangeInputs {
        rescales: Some(&hpa),
        nodes: Some(&nodes),
        namespaces: Some(&namespaces),
        deployments: Some(&feed),
        ..inputs()
    });
    let actor = |kind| {
        let entry = entries
            .iter()
            .find(|entry| entry.kind == kind)
            .expect("a row");
        (entry.actor.clone(), entry.actor_source, entry.tooltip())
    };
    let (hpa_actor, hpa_source, hpa_tooltip) = actor(ChangeKind::Autoscaler);
    assert_eq!(hpa_actor.as_deref(), Some("deployment-controller"));
    assert_eq!(hpa_source, ActorSource::EventSource);
    assert!(!hpa_tooltip.contains("event source"), "{hpa_tooltip}");
    assert_eq!(actor(ChangeKind::Node).0.as_deref(), Some("kubelet"));
    assert_eq!(actor(ChangeKind::Namespace).0, None);
}

#[test]
fn event_entry_reads_named_replica_set() {
    let named = |message: &str| {
        let mut rollout = event("Deployment", Some("payments"), "api", Some(RECENT));
        rollout.message = message.to_owned();
        recent_changes(&ChangeInputs {
            rollouts: Some(&[rollout]),
            ..inputs()
        })
        .remove(0)
        .replica_set
    };
    assert_eq!(
        named("Scaled down replica set api-7d9f8c to 0").as_deref(),
        Some("api-7d9f8c")
    );
    assert_eq!(
        named("Scaled up replica set api-7d9f8c to 3 from 2").as_deref(),
        Some("api-7d9f8c")
    );
    assert_eq!(named("Deployment api paused"), None);
    // Arbitrary text after the words is not a name.
    assert_eq!(named("Scaled up replica set Not_A_Name to 3"), None);
    // Only Deployment rows carry one.
    let mut hpa = event(
        "HorizontalPodAutoscaler",
        Some("payments"),
        "api",
        Some(RECENT),
    );
    hpa.message = "Scaled up replica set api-7d9f8c to 3".to_owned();
    let entries = recent_changes(&ChangeInputs {
        rescales: Some(&[hpa]),
        ..inputs()
    });
    assert_eq!(entries[0].replica_set, None);
}

// ---- The 24 h range: rollouts from ReplicaSet creation times ----

fn replica_set(
    name: &str,
    owner: Option<&str>,
    revision: Option<&str>,
    cause: Option<&str>,
    created: &str,
) -> ReplicaSetSummary {
    ReplicaSetSummary {
        namespace: "payments".to_owned(),
        name: name.to_owned(),
        created_at: Some(at(created)),
        labels: Vec::new(),
        desired: 3,
        current: 3,
        ready: 3,
        owner: owner.map(|owner| cluster::ControllerRef {
            kind: "Deployment".to_owned(),
            name: owner.to_owned(),
        }),
        revision: revision.map(str::to_owned),
        change_cause: cause.map(str::to_owned),
        selector: Vec::new(),
        containers: vec![cluster::TemplateContainer {
            name: "api".to_owned(),
            image: "repo.example.com/api:2.14.0".to_owned(),
            ports: Vec::new(),
            resources: Vec::new(),
        }],
    }
}

fn day_inputs<'a>() -> ChangeInputs<'a> {
    ChangeInputs {
        window: ChangeWindow::TwentyFourHours,
        ..inputs()
    }
}

/// Eight hours before `now()`: past the 1 h range, inside the 24 h one.
const EIGHT_HOURS_AGO: &str = "2024-05-01T04:00:00Z";

#[test]
fn the_24_hour_range_lists_a_rollout_with_its_revision_tag_and_cause() {
    let sets = [replica_set(
        "api-7d9f8c",
        Some("api"),
        Some("8"),
        Some("release test"),
        EIGHT_HOURS_AGO,
    )];
    let entries = recent_changes(&ChangeInputs {
        replica_sets: Some(&sets),
        ..day_inputs()
    });
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, ChangeKind::Deployment);
    assert_eq!(entries[0].object, "payments/api");
    assert_eq!(entries[0].text, "rev 8 · 2.14.0 · release test");
    assert_eq!(entries[0].at, at(EIGHT_HOURS_AGO));
    assert_eq!(entries[0].replica_set.as_deref(), Some("api-7d9f8c"));
    assert!(entries[0].target.is_some());
}

#[test]
fn the_shorter_ranges_do_not_read_replica_sets() {
    let sets = [replica_set(
        "api-7d9f8c",
        Some("api"),
        Some("8"),
        None,
        "2024-05-01T11:55:00Z",
    )];
    for window in [ChangeWindow::FifteenMinutes, ChangeWindow::OneHour] {
        let entries = recent_changes(&ChangeInputs {
            replica_sets: Some(&sets),
            window,
            ..inputs()
        });
        assert!(entries.is_empty(), "{window:?}");
    }
}

#[test]
fn a_replica_set_without_a_deployment_owner_or_a_revision_is_no_rollout() {
    let sets = [
        replica_set("a-1", None, Some("1"), None, EIGHT_HOURS_AGO),
        replica_set("b-1", Some("b"), None, None, EIGHT_HOURS_AGO),
        replica_set("c-1", Some("c"), Some("1"), None, LONG_AGO),
    ];
    let entries = recent_changes(&ChangeInputs {
        replica_sets: Some(&sets),
        ..day_inputs()
    });
    assert!(entries.is_empty());
}

#[test]
fn a_rollout_row_replaces_the_scale_up_event_of_its_replica_set() {
    let sets = [replica_set(
        "api-7d9f8c",
        Some("api"),
        Some("8"),
        None,
        RECENT,
    )];
    let mut scale_down = event("Deployment", Some("payments"), "api", Some(RECENT));
    scale_down.message = "Scaled down replica set api-6b5f77 to 2".to_owned();
    let events = [
        // The event of the same ReplicaSet: the rollout row says it better.
        event("Deployment", Some("payments"), "api", Some(RECENT)),
        scale_down,
    ];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        replica_sets: Some(&sets),
        ..day_inputs()
    });
    let texts: Vec<&str> = entries.iter().map(|entry| entry.text.as_str()).collect();
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert!(texts.contains(&"rev 8 · 2.14.0"));
    assert!(texts.contains(&"Scaled down replica set api-6b5f77 to 2"));
}

#[test]
fn the_24_hour_range_keeps_the_events_that_name_no_listed_replica_set() {
    let events = [event("Deployment", Some("payments"), "api", Some(RECENT))];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        ..day_inputs()
    });
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text, "Scaled up replica set api-7d9f8c to 3");
}

#[test]
fn the_24_hour_range_is_the_third_option_and_spans_a_day() {
    assert_eq!(
        ChangeWindow::ALL.map(ChangeWindow::label),
        ["Last 15 min", "Last 1 h", "Last 24 h"]
    );
    assert_eq!(
        ChangeWindow::TwentyFourHours.span(),
        SignedDuration::from_hours(24)
    );
    assert!(ChangeWindow::TwentyFourHours.reads_replica_sets());
    assert!(!ChangeWindow::OneHour.reads_replica_sets());
}

#[test]
fn the_24_hour_range_adds_the_date_to_a_change_from_another_day() {
    let utc = TimeZone::UTC;
    let clock = |window| ChangeClock {
        zone: &utc,
        now: now(),
        window,
    };
    let yesterday = at("2024-04-30T22:10:00Z");
    let today = at("2024-05-01T04:00:00Z");
    let day = clock(ChangeWindow::TwentyFourHours);
    assert_eq!(day.label(yesterday), "04-30 22:10");
    assert_eq!(day.label(today), "04:00");
    // The shorter ranges never reach another day on a normal clock, and keep their narrow column.
    let hour = clock(ChangeWindow::OneHour);
    assert_eq!(hour.label(today), "04:00");
    assert!(day.width() > hour.width());
}

#[test]
fn a_scale_up_long_after_the_creation_is_a_rollback_and_stays() {
    // The rolled-back ReplicaSet is eight hours old; only the event has the time of the rollback.
    let sets = [replica_set(
        "api-7d9f8c",
        Some("api"),
        Some("9"),
        None,
        EIGHT_HOURS_AGO,
    )];
    let events = [event("Deployment", Some("payments"), "api", Some(RECENT))];
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(&events),
        replica_sets: Some(&sets),
        ..day_inputs()
    });
    let texts: Vec<&str> = entries.iter().map(|entry| entry.text.as_str()).collect();
    assert_eq!(
        texts,
        ["Scaled up replica set api-7d9f8c to 3", "rev 9 · 2.14.0"]
    );
}
