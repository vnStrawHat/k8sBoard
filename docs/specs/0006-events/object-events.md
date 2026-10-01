# 0006 · App: events in Pod, Node, and kind drawers

[Back to index](README.md) · Step 3 · Modules: `object_events.rs` (new), `cluster_session.rs`, `app_shell.rs`, `drawer.rs`, `pod_drawer.rs`, `node_drawer.rs`, `kind_drawer.rs`, `launch_options.rs`, `screenshot.rs`

## Session (`cluster_session.rs`)

```rust
pub(crate) struct LiveCluster { /* … */ object_events: Option<ObjectEvents> }
/// The open drawer's events. Dropping it stops the watch.
pub(crate) struct ObjectEvents { pub(crate) subject: InvolvedObject, pub(crate) list: LiveList<EventSummary>, _subscription: WatchSubscription }
impl ClusterSession {
    /// Starts, replaces, or stops the object events watch.
    pub(crate) fn set_event_subject(&mut self, subject: Option<InvolvedObject>, cx: &mut Context<Self>);
}
impl LiveCluster {
    pub(crate) fn events_of(&self, subject: &InvolvedObject) -> Option<&LiveList<EventSummary>>;
    pub(crate) fn is_object_events_loading(&self) -> bool; // Some and Loading
}
```

| Event | Effect |
|---|---|
| same subject as the running one | no-op (no re-list) |
| a different subject | old subscription dropped first; `Some(ObjectEvents { subject, Loading, subscribe(watch_object_events(&subject).map(newest_first)) })` |
| `None` | `object_events = None` |
| not live | ignored: a new session has no selection |

- `apply` and `on_closed` guard on `object_events.subject == subject`, like the explorer kind guard.
- `watch_count()` counts it: `open_watch_count(explorer, object_events)` = 3 + explorer + object events.
- `has_problem()` ignores it (decision 22); the drawer shows its failure.

## Following the selection, debounced (`app_shell.rs` and `object_events.rs`, decision 21)

Each start is an uncached list plus watch on the API server ([decisions.md](decisions.md), cost model), so arrow-key navigation must not start one per row.

```rust
const DRAWER_SUBJECT_DELAY: Duration = Duration::from_millis(250); // drawer.rs since spec 0007; also the first YAML fetch
struct AppShell { /* … */ event_subject_task: Option<Task<()>> } // app_shell.rs; replaced on every change; dropping cancels it
// object_events.rs, pure and unit-tested:
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SubjectChange { Keep, Stop, Start(InvolvedObject) }
/// Pure: what the object events watch must do when the selection becomes `next`.
pub(crate) fn subject_change(running: Option<&InvolvedObject>, next: Option<InvolvedObject>) -> SubjectChange;
```

| `running` (the session's subject) | `next` | Result |
|---|---|---|
| any | equal to `running` | `Keep` |
| any | `None` | `Stop` |
| any | a different subject | `Start(next)` |

`change_selection` is the only writer of `selected`. After it stores a new key it computes `subject_change(live subject, self.selected.as_ref().and_then(event_subject))`:

- `Keep`: nothing. (While a start is pending, `running` is `None`, so a quick A → B → A restarts A once, after the delay.)
- `Stop`: `event_subject_task = None`, then `set_event_subject(None)` at once (stopping is free).
- `Start(subject)`: `set_event_subject(None)` at once, then `event_subject_task = Some(cx.spawn(..))` (replacing, and so cancelling, a pending one), which waits `cx.background_executor().timer(DRAWER_SUBJECT_DELAY)`, then in one `update` sets `event_subject_task = None` and calls `set_event_subject(Some(subject))`.

Also:

- `close_drawer` calls `change_selection(None, cx)` instead of assigning `selected`; step 2 already routes `SelectionSync::Clear` through it.
- The watch exists only while a Pod, Node, or non-Event kind drawer is open: `show_screen`, `set_namespace`, and a context switch all close the drawer first.
- While the timer runs, `events_of(subject)` is `None`, which renders as Loading.

## Subjects and renderers (`object_events.rs`)

```rust
/// The object whose events a drawer shows; `None` for an event's own drawer.
pub(crate) fn event_subject(key: &ResourceKey) -> Option<InvolvedObject>;
pub(crate) fn events_title(list: Option<&LiveList<EventSummary>>) -> String;     // "Events", or "Events {n}" once ready
fn event_note(list: Option<&LiveList<EventSummary>>) -> Option<&'static str>;
pub(crate) fn recent_events(list: Option<&LiveList<EventSummary>>, cx: &App) -> AnyElement; // rows have no listeners
```

| `ResourceKey` | Subject |
|---|---|
| `Pod { namespace, name }` | `Pod`, `Some(namespace)`, name |
| `Node { name }` | `Node`, `None`, name |
| `Kind { Events, .. }` | `None` |
| `Kind { kind, namespace, name }` | `kind.object_kind()`, namespace, name |

| List | `event_note` |
|---|---|
| `None` or `Loading` | "Loading events…" |
| `Failed` | "Events are unavailable" (the failure text follows, muted `text_xs`) |
| `Ready`, empty | "No recent events" |
| `Ready`, rows | `None` |

`recent_events` renders the note, or at most `MAX_EVENT_ROWS = 50` rows (snapshot order, already newest first) and a muted `+{n} more`. It renders no title, so a section (here) or a tab (0007) can host it unchanged. Each row is non-interactive (no click, no hover, no cursor change): `id(("object-event", ix))`, rounded, `px_2 py_1`, and a tooltip with the full message.

- line 1, `h_flex gap_2 text_sm`: `toned_text(event_tone(..))`, the reason (`font_semibold`, truncated), `×{count}` muted when count > 1, then `{age} ago` muted and right-aligned (`format_age(last_seen, now)`);
- line 2: the message, `text_xs`, muted, wrapping, `line_clamp(3)`.

## Placement

| Drawer | Where |
|---|---|
| Pod | third tab, `PodDrawerTab::Events`, label `events_title(list)`; the body is `recent_events`. The tab survives a subject change like the others |
| Node | the Events tab (spec 0007 moved it from an Overview section): `recent_events` |
| Kind | the Events tab, only when `event_subject(&key)` is `Some` (spec 0007) |

`list` is `live.events_of(&subject)`; `None` (debounce or switch in progress) renders as Loading.

0007 compatibility: the Node and kind sections are one `section_title` plus `recent_events`, so 0007 can move them into an Events tab (label `events_title(list)`, like the pod tab) without changing `object_events.rs` or the session.

## Screenshots

- `LaunchScreen::PodEvents` (`--screen pod-events`): Pods with a pod drawer on the Events tab, (spec 0007 folds it into `LaunchScreen::PodDrawer(DrawerTab::Events)`; `AppShell::new` sets `drawer.tab`). `has_drawer` is true. Update `USAGE`.
- Pure `pub(crate) fn is_drawer_ready(has_selection: bool, is_launch_pending: bool, is_object_events_pending: bool) -> bool` in `screenshot.rs` = `(has_selection || !is_launch_pending) && !is_object_events_pending`. `settle_input` passes `event_subject_task.is_some() || live.is_object_events_loading()`, so every drawer screen waits for the debounce and then its events.
