use super::*;
use crate::resource_kind::ResourceKind;

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

fn object(kind: &str, namespace: Option<&str>, name: &str) -> InvolvedObject {
    InvolvedObject {
        kind: kind.to_owned(),
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
    }
}

fn summary() -> EventSummary {
    EventSummary {
        namespace: "team-a".to_owned(),
        name: "api-0.17a2b".to_owned(),
        event_type: EventType::Warning,
        reason: "BackOff".to_owned(),
        object: object("Pod", Some("team-a"), "api-0"),
        container: None,
        message: "Back-off restarting failed container".to_owned(),
        count: 4,
        first_seen: Some(at(100)),
        last_seen: Some(at(200)),
        source: Some("kubelet on node-1".to_owned()),
    }
}

fn named(namespace: &str, name: &str, last_seen: Option<jiff::Timestamp>) -> EventSummary {
    EventSummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        last_seen,
        ..summary()
    }
}

fn detail(row: &KindRow) -> &EventDetail {
    row.event.as_ref().expect("event rows carry a detail")
}

#[test]
fn event_row_cells_match_column_count() {
    let row = event_row(&summary());
    assert_eq!(row.cells.len(), ResourceKind::Events.columns().len());
    assert_eq!(row.namespace.as_deref(), Some("team-a"));
    assert_eq!(row.name, "api-0.17a2b");
    assert_eq!(row.created_at, None);
    assert!(row.labels.is_empty() && row.related_pods.is_none());
}

#[test]
fn events_sort_newest_first_with_unknown_last() {
    let mut events = vec![
        named("b", "x", Some(at(10))),
        named("a", "unknown", None),
        named("b", "y", Some(at(30))),
        named("a", "tie-2", Some(at(20))),
        named("a", "tie-1", Some(at(20))),
    ];
    sort_newest_first(&mut events);
    let order: Vec<&str> = events.iter().map(|event| event.name.as_str()).collect();
    assert_eq!(order, ["y", "tie-1", "tie-2", "x", "unknown"]);
}

#[test]
fn event_rows_sorts_a_snapshot_newest_first() {
    let update = WatchUpdate::Snapshot(vec![
        named("a", "old", Some(at(1))),
        named("a", "new", Some(at(2))),
    ]);
    let WatchUpdate::Snapshot(rows) = event_rows(update) else {
        panic!("a snapshot must stay a snapshot");
    };
    let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["new", "old"]);
}

#[test]
fn event_rows_passes_a_failure_through() {
    let failure = WatchUpdate::Failed(cluster::ClusterError::TimedOut {
        context: "ctx".to_owned(),
        action: "watching events",
    });
    assert!(matches!(event_rows(failure), WatchUpdate::Failed(_)));
}

#[test]
fn event_tone_warns_on_warning_and_mutes_normal() {
    let warning = event_tone(EventType::Warning);
    assert_eq!(
        (warning.text.as_ref(), warning.tone),
        ("Warning", StatusTone::Warn)
    );
    let normal = event_tone(EventType::Normal);
    assert_eq!(
        (normal.text.as_ref(), normal.tone),
        ("Normal", StatusTone::Done)
    );
}

#[test]
fn object_text_lowercases_kind() {
    assert_eq!(object_text(&object("Pod", None, "api")), "pod/api");
    assert_eq!(
        object_text(&object("HorizontalPodAutoscaler", None, "web")),
        "horizontalpodautoscaler/web"
    );
    assert_eq!(object_text(&object("", None, "api")), "api");
}

#[test]
fn object_key_maps_viewable_kinds() {
    let key = |kind, namespace, name| object_key(&object(kind, namespace, name));
    assert_eq!(
        key("Pod", Some("team-a"), "api-0"),
        Some(ResourceKey::Pod {
            namespace: "team-a".to_owned(),
            name: "api-0".to_owned(),
        })
    );
    assert_eq!(
        key("Node", None, "node-1"),
        Some(ResourceKey::Node {
            name: "node-1".to_owned(),
        })
    );
    assert_eq!(
        key("Deployment", Some("team-a"), "api"),
        Some(ResourceKey::Kind {
            kind: ResourceKind::Deployments,
            namespace: Some("team-a".to_owned()),
            name: "api".to_owned(),
        })
    );
    // Namespaces are cluster-scoped: a stray namespace on the reference is dropped.
    assert_eq!(
        key("Namespace", Some("team-a"), "team-a"),
        Some(ResourceKey::Kind {
            kind: ResourceKind::Namespaces,
            namespace: None,
            name: "team-a".to_owned(),
        })
    );
    assert_eq!(key("HorizontalPodAutoscaler", Some("team-a"), "web"), None);
    assert_eq!(key("Event", Some("team-a"), "e"), None);
    assert_eq!(key("Pod", None, "api-0"), None);
}

#[test]
fn message_line_joins_lines() {
    assert_eq!(message_line("a\nb\r\nc"), "a b c");
    assert_eq!(message_line("a\n\nb"), "a b");
    assert_eq!(message_line("single line"), "single line");
    assert_eq!(message_line(""), "");
}

#[test]
fn event_title_is_reason_and_object_name() {
    let row = event_row(&summary());
    assert_eq!(detail(&row).title, "BackOff · api-0");
    let without_reason = EventSummary {
        reason: String::new(),
        ..summary()
    };
    let row = event_row(&without_reason);
    assert_eq!(detail(&row).title, "api-0");
    assert_eq!(row.cells.get(1), Some(&KindCell::Absent));
}

#[test]
fn event_row_has_details_and_message_sections() {
    let long = EventSummary {
        message: "line one\nline two".to_owned(),
        ..summary()
    };
    let row = event_row(&long);
    let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
    assert_eq!(titles, ["Details", "Message"]);
    let details = row.section("Details").expect("details section");
    assert!(matches!(
        details.rows.first(),
        Some(DetailRow::Link { text, .. }) if text.as_ref() == "team-a/pod/api-0"
    ));
    let message = row.section("Message").expect("message section");
    assert_eq!(message.rows, [DetailRow::Code("line one\nline two".into())]);
    assert_eq!(detail(&row).message, "line one\nline two");
    assert_eq!(detail(&row).source.as_deref(), Some("kubelet on node-1"));
}

#[test]
fn event_object_without_a_screen_is_plain_text_and_empty_message_is_a_note() {
    let event = EventSummary {
        object: object("HorizontalPodAutoscaler", Some("team-a"), "web"),
        container: None,
        message: String::new(),
        ..summary()
    };
    let row = event_row(&event);
    assert_eq!(detail(&row).object, None);
    let details = row.section("Details").expect("details section");
    assert!(matches!(
        details.rows.first(),
        Some(DetailRow::Field { value: KindCell::Mono(text), .. })
            if text.as_ref() == "team-a/horizontalpodautoscaler/web"
    ));
    let message = row.section("Message").expect("message section");
    assert_eq!(message.rows, [DetailRow::Note("No message".into())]);
}

#[test]
fn event_title_falls_back_to_the_event_name_without_an_object_name() {
    let nameless = EventSummary {
        object: object("", None, ""),
        container: None,
        ..summary()
    };
    let row = event_row(&nameless);
    assert_eq!(detail(&row).title, "BackOff · api-0.17a2b");
    let bare = EventSummary {
        reason: String::new(),
        ..nameless
    };
    assert_eq!(detail(&event_row(&bare)).title, "api-0.17a2b");
}
