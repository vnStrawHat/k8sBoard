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
