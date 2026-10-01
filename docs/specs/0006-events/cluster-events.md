# 0006 · Cluster crate: events

[Back to index](README.md) · Step 1 · Modules: `src/event.rs` (new), `src/resource_watch.rs`, `src/lib.rs`, `examples/probe.rs`

## Public API (`event.rs`)

```rust
/// The most recent events one events watch keeps.
pub const EVENT_LIMIT: usize = 2_000;
const MESSAGE_LIMIT: usize = 1_024; // bytes, before the `…`

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventType { Normal, Warning }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EventFilter { #[default] All, WarningsOnly }

/// The object an event is about (core/v1 `involvedObject`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvolvedObject { pub kind: String, pub namespace: Option<String>, pub name: String }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventSummary {
    pub namespace: String,
    pub name: String,
    pub event_type: EventType,
    pub reason: String,                    // "" when absent
    pub object: InvolvedObject,
    pub message: String,                   // truncated, newlines kept
    pub count: u32,                        // >= 1
    pub first_seen: Option<jiff::Timestamp>,
    pub last_seen: Option<jiff::Timestamp>,
    pub source: Option<String>,            // "kubelet on ip-10-0-1-23"
}

impl ClusterConnection {
    /// Events in `scope`, at most `EVENT_LIMIT` newest. Snapshots are ordered by (namespace, name).
    pub fn watch_events(&self, scope: NamespaceScope, filter: EventFilter)
        -> impl Stream<Item = WatchUpdate<EventSummary>> + Send + 'static;
    /// Events about one object, read from its namespace. An object without a namespace (Node,
    /// Namespace) is read from `default`, where client-go's recorder puts its events.
    pub fn watch_object_events(&self, object: &InvolvedObject)
        -> impl Stream<Item = WatchUpdate<EventSummary>> + Send + 'static;
}
pub(crate) fn event_summary(event: &Event) -> EventSummary;
```

- Action texts: `"watching events"`, `"watching object events"`.
- `watch_events`: `scoped_api::<Event>(scope)`; config `watcher::Config::default()`, plus `.fields("type=Warning")` for `WarningsOnly` (private `fn event_field_selector(filter) -> Option<&'static str>`).
- `watch_object_events`: `scoped_api(Named(object.namespace or "default"))` via private `fn object_events_namespace(object: &InvolvedObject) -> &str` and `const CLUSTER_SCOPED_EVENTS_NAMESPACE: &str = "default"` (comment: client-go recorder convention); never `All`. `.fields(&object_field_selector(object))` = `involvedObject.kind={kind},involvedObject.name={name}`. No escaping: kinds are identifiers and names are DNS subdomains, so neither contains `,`, `=`, or `\`.
- Both use `limited_summary_watch(.., StoreLimit { max_items: EVENT_LIMIT, recency: |event| event.last_seen })`.

## Field mapping (`event_summary`), kubectl's printer rules

| Field | Rule |
|---|---|
| `event_type` | `type_ == Some("Warning")` → `Warning`; anything else (Normal, empty) → `Normal` |
| `reason` | `reason`, or `""` |
| `object` | `involved_object`: `kind` or `""`; `namespace` → `None` when missing or empty; `name` or `""` |
| `message` | `truncate_message(message or "")`: first `str::trim` (both ends, like Go `strings.TrimSpace`; inner newlines kept), then unchanged up to `MESSAGE_LIMIT` bytes; else cut at the largest char boundary ≤ the limit (walk back with `is_char_boundary`) and append `…` |
| `count` | `series.count` when `series` is set; else `count` when > 0; else 1. Negative or 0 → 1 |
| `first_seen` | `first_timestamp` → `event_time` → `metadata.creation_timestamp` |
| `last_seen` | `series.last_observed_time` → `last_timestamp` → `first_seen` |
| `source` | Per field, like kubectl `formatEventSource`: `component = first_non_empty(source.component, reporting_component)`, `host = first_non_empty(source.host, reporting_instance)`. Text (W7 wording): both → `{component} on {host}`; one → that one; none → `None` |

Not kept: `metadata.labels`, `annotations`, `managedFields`, `related`, `action`, `involvedObject.fieldPath`, `uid`, and `resourceVersion`.

## Store limit (`resource_watch.rs`)

```rust
/// Keeps a watch store to the `max_items` most recent summaries. `None` recency is oldest.
#[derive(Clone, Copy)]
pub(crate) struct StoreLimit<T> { pub(crate) max_items: usize, pub(crate) recency: fn(&T) -> Option<jiff::Timestamp> }

pub(crate) fn limited_summary_watch<K, T>(connection: &ClusterConnection, api: Api<K>,
    config: watcher::Config, action: &'static str, summarize: fn(&K) -> T, limit: StoreLimit<T>,
) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static; // same bounds as summary_watch
```

- `summary_watch` keeps its signature and calls the shared core with `Config::default()` and no limit. `batch_updates` gains `limit: Option<StoreLimit<T>>`; `SummaryStore::new(limit)` replaces `Default`.
- Private `fn trim<T>(map: &mut BTreeMap<ObjectKey, T>, limit: &StoreLimit<T>)`: when `len > max_items`, sort `(recency, key)` ascending and remove the first `len - max_items`. Ties fall to key order, so it is deterministic.

| Store call | With a limit |
|---|---|
| `init_apply` | insert, then `trim` the buffer once it holds `2 × max_items` (amortized) |
| `finish_init` | swap in the buffer, then `trim` to `max_items` |
| `apply` | `let previous = items.insert(..)`; `trim`; return `previous.as_ref() != items.get(&key)`. A new item older than everything retained is evicted at once and is not a change |
| `delete`, `snapshot` | unchanged |

## `lib.rs`

`mod event;` and `pub use event::{EVENT_LIMIT, EventFilter, EventSummary, EventType, InvolvedObject};`.

## Probe (`examples/probe.rs`)

- `--watch-seconds` adds `tally_source("events", connection.watch_events(scope.clone(), EventFilter::All))` and `tally_source("warning events", connection.watch_events(scope, EventFilter::WarningsOnly))`: 14 watch lines. Update the "twelve watches" doc comment.
- Counts only. The probe never prints a reason, message, object, or source.
