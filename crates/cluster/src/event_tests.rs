use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{EventSeries, EventSource};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{MicroTime, ObjectMeta, Time};

use super::*;

fn timestamp(text: &str) -> jiff::Timestamp {
    text.parse().expect("valid timestamp")
}

fn pod_object(namespace: Option<&str>) -> InvolvedObject {
    InvolvedObject {
        kind: "Pod".to_owned(),
        namespace: namespace.map(str::to_owned),
        name: "api-0".to_owned(),
    }
}

fn source(component: Option<&str>, host: Option<&str>) -> Option<EventSource> {
    Some(EventSource {
        component: component.map(str::to_owned),
        host: host.map(str::to_owned),
    })
}

#[test]
fn legacy_event_reads_count_and_last_timestamp() {
    let summary = event_summary(&Event {
        count: Some(7),
        first_timestamp: Some(Time(timestamp("2024-05-01T10:00:00Z"))),
        last_timestamp: Some(Time(timestamp("2024-05-01T10:30:00Z"))),
        ..Default::default()
    });
    assert_eq!(summary.count, 7);
    assert_eq!(summary.first_seen, Some(timestamp("2024-05-01T10:00:00Z")));
    assert_eq!(summary.last_seen, Some(timestamp("2024-05-01T10:30:00Z")));
}

#[test]
fn event_series_overrides_count_and_last_seen() {
    let summary = event_summary(&Event {
        count: Some(2),
        last_timestamp: Some(Time(timestamp("2024-05-01T10:30:00Z"))),
        series: Some(EventSeries {
            count: Some(9),
            last_observed_time: Some(MicroTime(timestamp("2024-05-01T11:00:00Z"))),
        }),
        ..Default::default()
    });
    assert_eq!(summary.count, 9);
    assert_eq!(summary.last_seen, Some(timestamp("2024-05-01T11:00:00Z")));
}

#[test]
fn new_style_singleton_counts_one_and_uses_event_time() {
    let summary = event_summary(&Event {
        count: Some(0),
        event_time: Some(MicroTime(timestamp("2024-05-01T12:00:00Z"))),
        ..Default::default()
    });
    assert_eq!(summary.count, 1);
    assert_eq!(summary.first_seen, Some(timestamp("2024-05-01T12:00:00Z")));
    assert_eq!(summary.last_seen, Some(timestamp("2024-05-01T12:00:00Z")));
    assert_eq!(event_summary(&Event::default()).count, 1);
    let negative = Event {
        count: Some(-3),
        ..Default::default()
    };
    assert_eq!(event_summary(&negative).count, 1);
}

#[test]
fn first_seen_falls_back_to_event_time_then_creation() {
    let metadata = ObjectMeta {
        creation_timestamp: Some(Time(timestamp("2024-05-01T09:00:00Z"))),
        ..Default::default()
    };
    let only_creation = Event {
        metadata: metadata.clone(),
        ..Default::default()
    };
    assert_eq!(
        event_summary(&only_creation).first_seen,
        Some(timestamp("2024-05-01T09:00:00Z"))
    );
    let with_event_time = Event {
        metadata,
        event_time: Some(MicroTime(timestamp("2024-05-01T10:00:00Z"))),
        ..Default::default()
    };
    assert_eq!(
        event_summary(&with_event_time).first_seen,
        Some(timestamp("2024-05-01T10:00:00Z"))
    );
    assert_eq!(event_summary(&Event::default()).first_seen, None);
}

#[test]
fn last_seen_falls_back_to_first_seen() {
    let summary = event_summary(&Event {
        first_timestamp: Some(Time(timestamp("2024-05-01T10:00:00Z"))),
        ..Default::default()
    });
    assert_eq!(summary.last_seen, Some(timestamp("2024-05-01T10:00:00Z")));
}

#[test]
fn only_warning_type_is_warning() {
    let kind = |type_: Option<&str>| {
        event_summary(&Event {
            type_: type_.map(str::to_owned),
            ..Default::default()
        })
        .event_type
    };
    assert_eq!(kind(Some("Warning")), EventType::Warning);
    assert_eq!(kind(Some("Normal")), EventType::Normal);
    assert_eq!(kind(Some("")), EventType::Normal);
    assert_eq!(kind(None), EventType::Normal);
}

#[test]
fn event_source_picks_component_and_host_per_field() {
    let text = |source, component: Option<&str>, instance: Option<&str>| {
        event_source(&Event {
            source,
            reporting_component: component.map(str::to_owned),
            reporting_instance: instance.map(str::to_owned),
            ..Default::default()
        })
    };
    let kubelet = source(Some("kubelet"), Some("node-1"));
    assert_eq!(
        text(kubelet, None, None).as_deref(),
        Some("kubelet on node-1")
    );
    assert_eq!(
        text(None, Some("default-scheduler"), Some("sched-0")).as_deref(),
        Some("default-scheduler on sched-0")
    );
    assert_eq!(
        text(source(Some("kubelet"), None), None, Some("node-2")).as_deref(),
        Some("kubelet on node-2")
    );
    assert_eq!(
        text(source(Some(""), Some("node-3")), Some("kubelet"), None).as_deref(),
        Some("kubelet on node-3")
    );
    assert_eq!(
        text(source(None, Some("node-4")), None, None).as_deref(),
        Some("node-4")
    );
    assert_eq!(text(None, None, None), None);
}

#[test]
fn involved_object_empty_namespace_is_none() {
    let object = |namespace: Option<&str>| {
        involved_object(&ObjectReference {
            kind: Some("Node".to_owned()),
            name: Some("node-1".to_owned()),
            namespace: namespace.map(str::to_owned),
            ..Default::default()
        })
    };
    assert_eq!(object(Some("")).namespace, None);
    assert_eq!(object(None).namespace, None);
    assert_eq!(object(Some("team-a")).namespace.as_deref(), Some("team-a"));
    let empty = involved_object(&ObjectReference::default());
    assert_eq!((empty.kind.as_str(), empty.name.as_str()), ("", ""));
}

#[test]
fn message_is_trimmed_and_keeps_inner_newlines() {
    assert_eq!(
        truncate_message("  \n back-off\nrestarting \t\n"),
        "back-off\nrestarting"
    );
}

#[test]
fn message_truncates_on_a_char_boundary() {
    // The two-byte char starts at byte 1,023 and straddles the 1,024-byte limit.
    let message = format!("{}é{}", "a".repeat(MESSAGE_LIMIT - 1), "b".repeat(50));
    let cut = truncate_message(&message);
    assert!(cut.ends_with('…'));
    let kept = cut.trim_end_matches('…');
    assert_eq!(kept.len(), MESSAGE_LIMIT - 1);
    assert!(kept.bytes().all(|byte| byte == b'a'));
}

#[test]
fn short_message_is_unchanged() {
    let exact = "m".repeat(MESSAGE_LIMIT);
    assert_eq!(truncate_message(&exact), exact);
    assert_eq!(truncate_message("pulled image"), "pulled image");
    assert_eq!(truncate_message(""), "");
}

#[test]
fn event_summary_drops_labels_and_annotations() {
    let labelled = Event {
        metadata: ObjectMeta {
            labels: Some(BTreeMap::from([(
                "label-key".to_owned(),
                "distinctive-label-value".to_owned(),
            )])),
            annotations: Some(BTreeMap::from([(
                "note".to_owned(),
                "distinctive-annotation-value".to_owned(),
            )])),
            ..Default::default()
        },
        ..Default::default()
    };
    let text = format!("{:?}", event_summary(&labelled));
    assert!(!text.contains("distinctive-label-value"));
    assert!(!text.contains("distinctive-annotation-value"));
}

#[test]
fn warnings_only_selects_warning_type() {
    assert_eq!(event_field_selector(EventFilter::All), None);
    assert_eq!(
        event_field_selector(EventFilter::WarningsOnly),
        Some("type=Warning")
    );
}

#[test]
fn object_selector_names_kind_and_name() {
    assert_eq!(
        object_field_selector(&pod_object(Some("team-a"))),
        "involvedObject.kind=Pod,involvedObject.name=api-0"
    );
}

#[test]
fn cluster_scoped_object_events_read_default_namespace() {
    assert_eq!(
        object_events_namespace(&pod_object(Some("team-a"))),
        "team-a"
    );
    let node = InvolvedObject {
        kind: "Node".to_owned(),
        namespace: None,
        name: "node-1".to_owned(),
    };
    assert_eq!(object_events_namespace(&node), "default");
}

#[test]
fn empty_series_counts_one_and_keeps_last_timestamp() {
    let summary = event_summary(&Event {
        last_timestamp: Some(Time(timestamp("2024-05-01T10:30:00Z"))),
        series: Some(EventSeries {
            count: None,
            last_observed_time: None,
        }),
        ..Default::default()
    });
    assert_eq!(summary.count, 1);
    assert_eq!(summary.last_seen, Some(timestamp("2024-05-01T10:30:00Z")));
}

#[test]
fn empty_object_namespace_reads_default() {
    assert_eq!(object_events_namespace(&pod_object(Some(""))), "default");
}
