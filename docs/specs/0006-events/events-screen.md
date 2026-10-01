# 0006 · App: Events screen

[Back to index](README.md) · Step 2 (step 4 for Warnings only) · Modules: `resource_kind.rs`, `kind_table.rs`, `kind_row.rs`, `event_rows.rs` (new), `workspace.rs`, `cluster_session.rs` (step 4), `app_shell.rs` (step 4)

## `ResourceKind::Events` (`resource_kind.rs`)

```rust
pub(crate) enum ResourceKind { Namespaces, Events, Deployments, /* … */ }  // ALL: [Self; 11]
/// Whether a kind leads with the Name column, or hides it and flexes one of its own columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NameColumn { Flexible, Hidden { flexible: usize } } // index into columns()
struct KindSpec { /* 0005 fields */ object_kind: &'static str, name_column: NameColumn, has_labels: bool }
impl ResourceKind {
    pub(crate) fn object_kind(self) -> &'static str;        // "Deployment", "Event", …
    pub(crate) fn name_column(self) -> NameColumn;
    pub(crate) fn has_labels(self) -> bool;                // drawer Labels section
    pub(crate) fn from_object_kind(text: &str) -> Option<Self>;
    /// Step 2: `(self, connection, scope)`; Events use `EventFilter::All`. Step 4 adds `events`,
    /// which only the Events arm reads.
    pub(crate) fn watch_rows(self, connection: &ClusterConnection, scope: NamespaceScope,
        events: EventFilter) -> BoxStream<'static, WatchUpdate<KindRow>>;
}
```

| `EVENTS` field | Value |
|---|---|
| label, singular, plural, badge | `Events`, `event`, `events`, `Ev` |
| is_namespaced, access_check | `true`, `AccessCheck::ListEvents` |
| object_kind, name_column, has_labels | `Event`, `Hidden { flexible: 3 }`, `false` |
| read_only_actions, delete_label, has_port_forward | `[]`, `Delete event…`, `false` |

The other 10 kinds: `name_column: Flexible`, `has_labels: true`, and `object_kind` = Namespace, Deployment, StatefulSet, DaemonSet, ReplicaSet, Job, CronJob, Service, Ingress, ConfigMap.

Events arm: `connection.watch_events(scope, EventFilter::All).map(event_rows).boxed()` in step 2; `events` replaces `All` in step 4.

## Columns

| # | Column | Width | Align | Cell |
|---|---|---|---|---|
| 0 | Type | 90 | Left | `Toned(event_tone(type))` |
| 1 | Reason | 170 | Left | `Text(reason)`, `Absent` when empty |
| 2 | Object | 260 | Left | `Qualified { prefix: object.namespace, text: object_text }` |
| 3 | Message | 280 (minimum) | Left | `Text(message_line(message))`, flexible |
| 4 | Count | 64 | Right | `count(count)` |
| 5 | Last seen | 80 | Right | `age(last_seen)` |

## Kind table (`kind_table.rs`)

- `columns(kind, flexible_width)`: `Flexible` keeps today's Name-first list. `Hidden { flexible }` emits only `kind.columns()`; the flexible one gets `.width(flexible_width).min_width(px(spec.width))`.
- `fit_width`: the flexible column is Name (min `NAME_MIN_WIDTH`) or `flexible` (min its spec width); `fixed_width` sums every other column.
- Pure `fn cell_index(name_column: NameColumn, col_ix: usize) -> Option<usize>`: `Flexible` → `col_ix.checked_sub(1)` (0 is Name); `Hidden` → `Some(col_ix)`. `render_td` and `align` use it; `render_td` calls `name_cell` only for `Flexible` and `col_ix == 0`.
- New cell `KindCell::Qualified { prefix: Option<SharedString>, text: SharedString }`: mono, `{prefix}/` muted, truncated, tooltip with the full text. Extract the body of `name_cell` into `fn qualified_text(id, prefix: Option<&str>, text: &str, mono, cx)` and use it for both. Id: `("kind-qualified", row_ix)` (one qualified column per kind).
- `KindTableDelegate::new(kind, shell: WeakEntity<AppShell>)`; `context_menu` passes `&self.shell` to `kind_menu` ([event-drawer.md](event-drawer.md)).

## Rows (`event_rows.rs`, built on tokio)

```rust
pub(crate) fn event_rows(update: WatchUpdate<EventSummary>) -> WatchUpdate<KindRow>; // newest_first, then event_row
pub(crate) fn newest_first(update: WatchUpdate<EventSummary>) -> WatchUpdate<EventSummary>; // step 3
fn sort_newest_first(events: &mut [EventSummary]); // last_seen desc (None last), then (namespace, name)
fn event_row(event: &EventSummary) -> KindRow;
pub(crate) fn event_tone(event_type: EventType) -> StatusLabel; // "Warning" Warn, "Normal" Done
pub(crate) fn object_text(object: &InvolvedObject) -> String;   // "pod/api-x"; the name alone when kind is ""
pub(crate) fn object_key(object: &InvolvedObject) -> Option<ResourceKey>;
fn message_line(message: &str) -> String;                       // line breaks become single spaces
```

- `object_key`: `Pod` with a namespace → `ResourceKey::Pod`; `Node` → `Node`; else `ResourceKind::from_object_kind(kind)` except `Events` → `Kind { kind, namespace: object.namespace if kind.is_namespaced() else None, name }`; anything else → `None`.
- `event_row`: `namespace: Some(event.namespace)`, `name: event.name`, `created_at: None`, `status: event_tone(..)`, cells as above, sections per [event-drawer.md](event-drawer.md), `related_pods: None`, `labels: []`, `event: Some(EventDetail { .. })`.
- The other ten builders and test fixtures add `event: None`.

## Header count (step 2)

`workspace.rs`, `Screen::Kind(Events)` only: `{count_label} · {scope}`, plus ` · newest 2,000` when the count is at least `EVENT_LIMIT`. Pure `fn group_digits(n: usize) -> String` (`2000` → `2,000`) formats it, so the text follows the constant.

## Warnings only (step 4)

Builds on the step 2 screen and the step 3 session; nothing else changes.

| Member | Change |
|---|---|
| `ClusterSession.event_filter: EventFilter` | default `All`; survives Connecting and retry like `explorer_kind`; a new context starts at `All` |
| `ClusterSession::event_filter()` | getter for the header |
| `ClusterSession::set_event_filter(filter, cx)` | same value → no-op. Store it; when live and the explorer kind is `Events`, drop the explorer and `KindList::start` again (`Loading`); `cx.notify()` |
| `KindList::start`, `subscribe_explorer`, `LiveCluster::start` | take the filter and pass it to `watch_rows`; `set_scope` and `set_explorer_kind` pass `self.event_filter` |
| `AppShell::toggle_warnings_only(cx)` | flips the session filter. The drawer stays; `sync_selection` clears it when its row is filtered out |

Header button (`workspace.rs`, `Screen::Kind(Events)` only):

- Right side (`ml_auto`): `Button::new("warnings-only").label("Warnings only").small()`, primary variant when on and outline when off, `.selected(is_on).toggled(is_on)` (the kit's `Selectable` plus the real-toggle a11y flag), `on_click` → `toggle_warnings_only`. Tooltip: "Show only Warning events".
