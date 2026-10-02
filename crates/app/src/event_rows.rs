//! The Events row builder. Event text is arbitrary, so nothing here logs it.

use std::cmp::Ordering;

use cluster::{EventSummary, EventType, InvolvedObject, WatchUpdate};
use gpui_kit::SharedString;

use crate::kind_row::{DetailRow, DetailSection, EventDetail, KindCell, KindRow};
use crate::status_tone::{StatusLabel, StatusTone};
use crate::table_selection::ResourceKey;

/// Sorts a snapshot newest first, then builds its rows. This runs on tokio, so the main
/// thread only swaps a `Vec`.
pub(crate) fn event_rows(update: WatchUpdate<EventSummary>) -> WatchUpdate<KindRow> {
    match newest_first(update) {
        WatchUpdate::Snapshot(events) => {
            WatchUpdate::Snapshot(events.iter().map(event_row).collect())
        }
        WatchUpdate::Failed(error) => WatchUpdate::Failed(error),
    }
}

/// Sorts a snapshot newest first and passes a failure through.
pub(crate) fn newest_first(update: WatchUpdate<EventSummary>) -> WatchUpdate<EventSummary> {
    match update {
        WatchUpdate::Snapshot(mut events) => {
            sort_newest_first(&mut events);
            WatchUpdate::Snapshot(events)
        }
        WatchUpdate::Failed(error) => WatchUpdate::Failed(error),
    }
}

/// Last seen descending with unknown times last, then (namespace, name) so equal times keep
/// a stable order.
fn sort_newest_first(events: &mut [EventSummary]) {
    events.sort_by(|left, right| {
        right
            .last_seen
            .cmp(&left.last_seen)
            .then_with(|| compare_names(left, right))
    });
}

fn compare_names(left: &EventSummary, right: &EventSummary) -> Ordering {
    (&left.namespace, &left.name).cmp(&(&right.namespace, &right.name))
}

fn event_row(event: &EventSummary) -> KindRow {
    let status = event_tone(event.event_type);
    let object = object_text(&event.object);
    let reason = (!event.reason.is_empty()).then_some(event.reason.as_str());
    let message = SharedString::from(event.message.clone());
    let source = event.source.clone().map(SharedString::from);
    let object_key = object_key(&event.object);
    // The object name is the useful part of the title; an event without one shows its own name.
    let subject = if event.object.name.is_empty() {
        &event.name
    } else {
        &event.object.name
    };
    let title = match reason {
        Some(reason) => format!("{reason} · {subject}"),
        None => subject.clone(),
    };
    let cells = vec![
        KindCell::Toned(status.clone()),
        KindCell::text_or_absent(reason),
        KindCell::Qualified {
            prefix: event.object.namespace.clone().map(SharedString::from),
            text: object.clone().into(),
        },
        KindCell::Text(message_line(&event.message).into()),
        KindCell::count(event.count),
        KindCell::age(event.last_seen),
    ];
    KindRow {
        namespace: Some(event.namespace.clone()),
        name: event.name.clone(),
        created_at: None,
        status,
        cells,
        sections: vec![
            DetailSection {
                title: "Details",
                rows: vec![
                    object_row(event, &object, object_key.clone()),
                    DetailRow::field("Reason", KindCell::text_or_absent(reason)),
                    DetailRow::field("Count", KindCell::count(event.count)),
                    DetailRow::field("First seen", KindCell::age(event.first_seen)),
                    DetailRow::field("Last seen", KindCell::age(event.last_seen)),
                    DetailRow::field("Source", KindCell::text_or_absent(event.source.as_deref())),
                    DetailRow::field("Name", KindCell::Mono(event.name.clone().into())),
                ],
            },
            DetailSection {
                title: "Message",
                rows: vec![message_row(&message)],
            },
        ],
        event: Some(EventDetail {
            title: title.into(),
            reason: reason.map(SharedString::from),
            object: object_key,
            source,
            message,
        }),
        related_pods: None,
        labels: Vec::new(),
    }
}

/// A link when k8sBoard has a screen for the object's kind, else plain mono text.
fn object_row(event: &EventSummary, object: &str, key: Option<ResourceKey>) -> DetailRow {
    let text = match &event.object.namespace {
        Some(namespace) => format!("{namespace}/{object}"),
        None => object.to_owned(),
    };
    match key {
        Some(target) => DetailRow::Link {
            label: "Object".into(),
            text: text.into(),
            target,
        },
        None => DetailRow::field("Object", KindCell::Mono(text.into())),
    }
}

fn message_row(message: &SharedString) -> DetailRow {
    if message.is_empty() {
        DetailRow::Note("No message".into())
    } else {
        DetailRow::Code(message.clone())
    }
}

pub(crate) fn event_tone(event_type: EventType) -> StatusLabel {
    let (text, tone) = match event_type {
        EventType::Warning => ("Warning", StatusTone::Warn),
        EventType::Normal => ("Normal", StatusTone::Done),
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

/// `pod/api-x`, or the name alone when the kind is unknown.
fn object_text(object: &InvolvedObject) -> String {
    if object.kind.is_empty() {
        return object.name.clone();
    }
    format!("{}/{}", object.kind.to_lowercase(), object.name)
}

/// The key of the object when k8sBoard has a screen for its kind.
fn object_key(object: &InvolvedObject) -> Option<ResourceKey> {
    ResourceKey::of_object(&object.kind, object.namespace.as_deref(), &object.name)
}

/// The table shows one line, so line breaks become single spaces.
pub(crate) fn message_line(message: &str) -> String {
    message
        .lines()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[path = "event_rows_tests.rs"]
mod event_rows_tests;
