//! Core/v1 events: summaries that follow kubectl's printer rules, and the watches.
//!
//! Event messages are arbitrary text, so nothing in this module logs them.

use futures::Stream;
use k8s_openapi::api::core::v1::{Event, ObjectReference};
use kube::runtime::watcher;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{StoreLimit, WatchUpdate, limited_summary_watch};
use crate::workload::non_empty;

/// The most recent events one events watch keeps.
pub const EVENT_LIMIT: usize = 2_000;

/// Bytes kept of a message, before the `…`. The events.k8s.io note limit.
const MESSAGE_LIMIT: usize = 1_024;

/// client-go's event recorder writes the events of cluster-scoped objects here.
const CLUSTER_SCOPED_EVENTS_NAMESPACE: &str = "default";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventType {
    Normal,
    Warning,
}

/// Which events a watch asks the server for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EventFilter {
    #[default]
    All,
    WarningsOnly,
}

/// The object an event is about (core/v1 `involvedObject`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvolvedObject {
    pub kind: String,
    /// `None` for cluster-scoped objects.
    pub namespace: Option<String>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventSummary {
    pub namespace: String,
    pub name: String,
    pub event_type: EventType,
    /// Empty when the event has none.
    pub reason: String,
    pub object: InvolvedObject,
    /// Trimmed and cut at 1 KiB plus `…`. Inner newlines are kept.
    pub message: String,
    /// At least 1.
    pub count: u32,
    pub first_seen: Option<jiff::Timestamp>,
    pub last_seen: Option<jiff::Timestamp>,
    /// For example `kubelet on ip-10-0-1-23`.
    pub source: Option<String>,
    /// The container named by `involvedObject.fieldPath`: `spec.containers{api}`,
    /// `spec.initContainers{api}`, or `spec.ephemeralContainers{api}`; else `None`.
    pub container: Option<String>,
}

impl ClusterConnection {
    /// Watches events in `scope`, keeping the `EVENT_LIMIT` most recent. Snapshots are
    /// ordered by (namespace, name).
    pub fn watch_events(
        &self,
        scope: NamespaceScope,
        filter: EventFilter,
    ) -> impl Stream<Item = WatchUpdate<EventSummary>> + Send + 'static {
        let mut config = watcher::Config::default();
        if let Some(selector) = event_field_selector(filter) {
            config = config.fields(selector);
        }
        limited_summary_watch(
            self,
            self.scoped_apis(&scope),
            config,
            "watching events",
            event_summary,
            event_limit(),
        )
    }

    /// Watches the events about one object, read from its namespace. An object without a
    /// namespace (Node, Namespace) is read from `default`.
    pub fn watch_object_events(
        &self,
        object: &InvolvedObject,
    ) -> impl Stream<Item = WatchUpdate<EventSummary>> + Send + 'static {
        let scope = NamespaceScope::Named(object_events_namespace(object).to_owned());
        let config = watcher::Config::default().fields(&object_field_selector(object));
        limited_summary_watch(
            self,
            self.scoped_apis(&scope),
            config,
            "watching object events",
            event_summary,
            event_limit(),
        )
    }

    /// Watches the warning `FailedCreate` events of `namespace`: the controller events that
    /// record a pod or other object rejected at admission, for example by a quota.
    pub fn watch_failed_creates(
        &self,
        namespace: &str,
    ) -> impl Stream<Item = WatchUpdate<EventSummary>> + Send + 'static {
        let scope = NamespaceScope::Named(namespace.to_owned());
        let config = watcher::Config::default().fields(failed_create_selector());
        limited_summary_watch(
            self,
            self.scoped_apis(&scope),
            config,
            "watching failed creates",
            event_summary,
            event_limit(),
        )
    }
}

fn failed_create_selector() -> &'static str {
    "type=Warning,reason=FailedCreate"
}

fn event_limit() -> StoreLimit<EventSummary> {
    StoreLimit {
        max_items: EVENT_LIMIT,
        recency: |event| event.last_seen,
    }
}

fn event_field_selector(filter: EventFilter) -> Option<&'static str> {
    match filter {
        EventFilter::All => None,
        EventFilter::WarningsOnly => Some("type=Warning"),
    }
}

/// The app only passes kinds whose names are DNS subdomains, so neither value contains
/// `,`, `=`, or `\` and no escaping is needed.
fn object_field_selector(object: &InvolvedObject) -> String {
    format!(
        "involvedObject.kind={},involvedObject.name={}",
        object.kind, object.name
    )
}

fn object_events_namespace(object: &InvolvedObject) -> &str {
    object
        .namespace
        .as_deref()
        .filter(|namespace| !namespace.is_empty())
        .unwrap_or(CLUSTER_SCOPED_EVENTS_NAMESPACE)
}

pub(crate) fn event_summary(event: &Event) -> EventSummary {
    let first_seen = event
        .first_timestamp
        .as_ref()
        .map(|time| time.0)
        .or_else(|| event.event_time.as_ref().map(|time| time.0))
        .or_else(|| {
            event
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|time| time.0)
        });
    let last_seen = event
        .series
        .as_ref()
        .and_then(|series| series.last_observed_time.as_ref())
        .map(|time| time.0)
        .or_else(|| event.last_timestamp.as_ref().map(|time| time.0))
        .or(first_seen);
    EventSummary {
        namespace: event.metadata.namespace.clone().unwrap_or_default(),
        name: event.metadata.name.clone().unwrap_or_default(),
        event_type: match event.type_.as_deref() {
            Some("Warning") => EventType::Warning,
            _ => EventType::Normal,
        },
        reason: event.reason.clone().unwrap_or_default(),
        object: involved_object(&event.involved_object),
        message: truncate_message(event.message.as_deref().unwrap_or_default()),
        count: event_count(event),
        first_seen,
        last_seen,
        source: event_source(event),
        container: event
            .involved_object
            .field_path
            .as_deref()
            .and_then(field_path_container),
    }
}

fn involved_object(reference: &ObjectReference) -> InvolvedObject {
    InvolvedObject {
        kind: reference.kind.clone().unwrap_or_default(),
        namespace: non_empty(reference.namespace.as_deref()),
        name: reference.name.clone().unwrap_or_default(),
    }
}

/// A series wins over the legacy count. Zero or negative reads as one occurrence, which
/// is how new-style singleton events arrive.
fn event_count(event: &Event) -> u32 {
    let count = match &event.series {
        Some(series) => series.count,
        None => event.count,
    };
    u32::try_from(count.unwrap_or(0)).map_or(1, |count| count.max(1))
}

/// Like kubectl's `formatEventSource`, per field: the legacy `source` wins over the
/// `reporting*` fields of new-style events.
fn event_source(event: &Event) -> Option<String> {
    let source = event.source.as_ref();
    let component = first_non_empty(
        source.and_then(|source| source.component.as_deref()),
        event.reporting_component.as_deref(),
    );
    let host = first_non_empty(
        source.and_then(|source| source.host.as_deref()),
        event.reporting_instance.as_deref(),
    );
    match (component, host) {
        (Some(component), Some(host)) => Some(format!("{component} on {host}")),
        (Some(text), None) | (None, Some(text)) => Some(text.to_owned()),
        (None, None) => None,
    }
}

fn first_non_empty<'a>(first: Option<&'a str>, second: Option<&'a str>) -> Option<&'a str> {
    first
        .filter(|text| !text.is_empty())
        .or(second.filter(|text| !text.is_empty()))
}

/// `truncate_message`, with empty text as `None`.
pub(crate) fn optional_message(message: Option<&str>) -> Option<String> {
    Some(truncate_message(message?)).filter(|message| !message.is_empty())
}

/// The container in a `spec.containers{name}`-style field path. The kubelet's
/// `implicitly required container {name}` and any other text give `None`.
fn field_path_container(path: &str) -> Option<String> {
    [
        "spec.containers",
        "spec.initContainers",
        "spec.ephemeralContainers",
    ]
    .iter()
    .find_map(|prefix| {
        path.strip_prefix(prefix)?
            .strip_prefix('{')?
            .strip_suffix('}')
    })
    .filter(|name| !name.is_empty())
    .map(str::to_owned)
}

/// Trims both ends like Go's `strings.TrimSpace`, then cuts at the largest char boundary
/// within `MESSAGE_LIMIT` bytes and appends `…`.
fn truncate_message(message: &str) -> String {
    let message = message.trim();
    if message.len() <= MESSAGE_LIMIT {
        return message.to_owned();
    }
    let end = message.floor_char_boundary(MESSAGE_LIMIT);
    let mut cut = message.get(..end).unwrap_or_default().to_owned();
    cut.push('…');
    cut
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod event_tests;
