use cluster::EventType;

use super::*;

fn subject(kind: &str, namespace: Option<&str>, name: &str) -> InvolvedObject {
    InvolvedObject {
        kind: kind.to_owned(),
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
    }
}

fn event() -> EventSummary {
    EventSummary {
        namespace: "team-a".to_owned(),
        name: "api-0.1".to_owned(),
        event_type: EventType::Normal,
        reason: "Pulled".to_owned(),
        object: subject("Pod", Some("team-a"), "api-0"),
        container: None,
        message: "pulled".to_owned(),
        count: 1,
        first_seen: None,
        last_seen: None,
        source: None,
    }
}

fn ready(items: Vec<EventSummary>) -> LiveList<EventSummary> {
    LiveList::Ready {
        items,
        interruption: None,
    }
}

#[test]
fn event_subject_maps_pods_nodes_and_kinds() {
    let pod = ResourceKey::Pod {
        namespace: "team-a".to_owned(),
        name: "api-0".to_owned(),
    };
    assert_eq!(
        event_subject(&pod),
        Some(subject("Pod", Some("team-a"), "api-0"))
    );
    let node = ResourceKey::Node {
        name: "node-1".to_owned(),
    };
    assert_eq!(event_subject(&node), Some(subject("Node", None, "node-1")));
    let kind = |kind, namespace: Option<&str>, name: &str| ResourceKey::Kind {
        kind,
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
    };
    assert_eq!(
        event_subject(&kind(ResourceKind::Deployments, Some("team-a"), "api")),
        Some(subject("Deployment", Some("team-a"), "api"))
    );
    assert_eq!(
        event_subject(&kind(ResourceKind::Namespaces, None, "team-a")),
        Some(subject("Namespace", None, "team-a"))
    );
    assert_eq!(
        event_subject(&kind(ResourceKind::Events, Some("team-a"), "api-0.1")),
        None
    );
}

#[test]
fn subject_change_keeps_stops_and_starts() {
    let a = subject("Pod", Some("ns"), "a");
    let b = subject("Pod", Some("ns"), "b");
    assert_eq!(
        subject_change(Some(&a), Some(a.clone())),
        SubjectChange::Keep
    );
    assert_eq!(
        subject_change::<InvolvedObject>(None, None),
        SubjectChange::Keep
    );
    assert_eq!(subject_change(Some(&a), None), SubjectChange::Stop);
    assert_eq!(
        subject_change(Some(&a), Some(b.clone())),
        SubjectChange::Start(b.clone())
    );
    assert_eq!(
        subject_change(None, Some(b.clone())),
        SubjectChange::Start(b)
    );
}

#[test]
fn event_note_by_list_state() {
    assert_eq!(event_note(None), Some("Loading events…"));
    assert_eq!(
        event_note(Some(&LiveList::Loading)),
        Some("Loading events…")
    );
    let failed = LiveList::Failed {
        message: "denied".to_owned(),
    };
    assert_eq!(event_note(Some(&failed)), Some("Events are unavailable"));
    assert_eq!(
        event_note(Some(&ready(Vec::new()))),
        Some("No recent events")
    );
    assert_eq!(event_note(Some(&ready(vec![event()]))), None);
}

#[test]
fn events_title_counts_only_ready_lists() {
    assert_eq!(events_title(None), "Events");
    assert_eq!(events_title(Some(&LiveList::Loading)), "Events");
    let failed = LiveList::Failed {
        message: "denied".to_owned(),
    };
    assert_eq!(events_title(Some(&failed)), "Events");
    assert_eq!(
        events_title(Some(&ready(vec![event(), event()]))),
        "Events 2"
    );
    assert_eq!(events_title(Some(&ready(Vec::new()))), "Events 0");
}

#[test]
fn subject_change_is_generic() {
    let a = "a".to_owned();
    let b = "b".to_owned();
    assert_eq!(
        subject_change(Some(&a), Some(a.clone())),
        SubjectChange::Keep
    );
    assert_eq!(subject_change::<String>(None, None), SubjectChange::Keep);
    assert_eq!(subject_change(Some(&a), None), SubjectChange::Stop);
    assert_eq!(
        subject_change(Some(&a), Some(b.clone())),
        SubjectChange::Start(b.clone())
    );
    assert_eq!(
        subject_change(None, Some(b.clone())),
        SubjectChange::Start(b)
    );
}

#[test]
fn helm_release_has_no_event_subject() {
    let key = ResourceKey::Kind {
        kind: ResourceKind::HelmReleases,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    };
    assert_eq!(event_subject(&key), None);
}

#[test]
fn event_subject_of_a_custom_key_uses_its_kind() {
    let crd = cluster::CrdSummary {
        name: "certificates.cert-manager.io".to_owned(),
        group: "cert-manager.io".to_owned(),
        kind: "Certificate".to_owned(),
        plural: "certificates".to_owned(),
        singular: "certificate".to_owned(),
        scope: cluster::ResourceScope::Namespaced,
        versions: vec![cluster::CrdVersion {
            name: "v1".to_owned(),
            is_served: true,
            is_storage: true,
            is_deprecated: false,
            deprecation_warning: None,
            printer_columns: Vec::new(),
            schema: cluster::SchemaOutline::default(),
        }],
        state: cluster::CrdState::Established,
        created_at: None,
    };
    let custom = crate::custom_kind::custom_kinds(
        &[crd],
        &mut crate::custom_kind::CustomKindCache::default(),
    )[0];
    let key = ResourceKey::Kind {
        kind: ResourceKind::Custom(custom),
        namespace: Some("shop".to_owned()),
        name: "web-tls".to_owned(),
    };
    assert_eq!(
        event_subject(&key),
        Some(subject("Certificate", Some("shop"), "web-tls"))
    );
}
