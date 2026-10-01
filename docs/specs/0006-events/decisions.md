# 0006 · Decisions

[Back to index](README.md). The architect chose these defaults; the user asked not to stop for questions. Amended after advisor review (M1, S1–S7).

## Data and cluster crate

| # | Decision | Rationale |
|---|---|---|
| 1 | Watch **core/v1 `Event`**, not events.k8s.io/v1 | `kubectl get events` uses it; the existing `ListEvents` check is core group; same storage, so new-style events appear too; core supports `involvedObject.*` and `type` field selectors |
| 2 | Map old and new styles like kubectl's printer ([cluster-events.md](cluster-events.md)): series wins for count and last seen; count 0 means 1; source per field like `formatEventSource` | users compare with kubectl; new-style (scheduler) events have only `eventTime`, `series`, and `reporting*` |
| 3 | Cap each events store at `EVENT_LIMIT` = 2,000 newest (by last seen), evicting oldest first, inside `SummaryStore` | the 0002 non-goal asked for it; bounds memory and the per-snapshot clone and row-build cost |
| 4 | Messages are trimmed at both ends, then truncated to 1,024 bytes on a char boundary, plus `…` | the events.k8s.io note limit; see the memory model below |
| 5 | Rely on watch deletes for the 1 h TTL; no client-side expiry | the API server deletes expired events and the watcher sees the delete |
| 6 | Keep the 100 ms batch window | one code path; with the cap a snapshot is at most 2,000 rows |
| 7 | No `WatchEvents` access check; `ListEvents` gates the sidebar item | 0005 decision 8 (list only); a denied watch shows the list error state. Supersedes the 0002 note |
| 8 | **Warnings only** is a server-side field selector `type=Warning` that restarts the explorer watch (step 4) | rows, indices, and selection stay consistent; the cap then holds 2,000 warnings. Server cost: see below |
| 9 | Object events use a separate watch with `involvedObject.kind=…,involvedObject.name=…`, in the object's namespace, or in `default` for cluster-scoped objects (Node, Namespace) | exact; client-go's recorder writes events of cluster-scoped objects to `default`, so a namespaced read replaces a cluster-wide scan |

## App structure

| # | Decision | Rationale |
|---|---|---|
| 10 | Events is an 11th `ResourceKind` and reuses the kind table, drawer, gating, selection, and screenshot plumbing | the caller asked for reuse; no second table delegate |
| 11 | `KindSpec.name_column: NameColumn { Flexible, Hidden { flexible } }`. Events hide Name and flex Message | an event name (`pod.17a2b…`) means nothing to users; Message is the long column |
| 12 | Events keep **pre-built** `KindRow.sections`. This revises the 0005 "Known ceilings" note | the cap bounds rows to 2,000; the full message is one `SharedString` shared by the Message section and `EventDetail` (Arc clone). Upgrade path: a lazy drawer from the summary, if profiling shows the row build costs more than ~5 ms per snapshot |
| 13 | Events rows are sorted newest first on tokio (`event_rows`), not in the crate | the crate's snapshot order stays (namespace, name) for every kind |
| 14 | `KindRow.event: Option<EventDetail>` carries the title, object key, source, and message | the menu, header, and subtitle need typed data; mirrors `related_pods` |
| 15 | Columns follow W7 order with kubectl content: Type, Reason, Object, Message, Count, Last seen | W7 is the design source; ending with a right-aligned age column matches every kind |
| 16 | Object cell shows the involved object's namespace as a muted prefix (`KindCell::Qualified`) | same look as the Name column; a node event lives in `default`, so the event's namespace would mislead |
| 17 | Normal is `StatusTone::Done` (muted), Warning is `Warn` | W7 `@mute Normal` and `@warn Warning`; no new tone |
| 18 | `AppShell::reveal(key)` replaces `reveal_pod` and works for pods, nodes, and kinds through `sync_selection` | one path; a kind list that is still loading resolves the selection when it is ready |
| 19 | Event menu: Go to object, Copy message, View YAML (disabled), then Copy name, then Delete event… (disabled) | W7 actions minus the deferred Filter similar; same groups as other kinds |
| 20 | Pods get an **Events** tab (W4); Node and kind drawers get an **Events** section after Pods and before Labels. `recent_events` is placement-agnostic | the pod drawer already has tabs; 0007 moves the section into a tab without touching the renderer |
| 21 | The object events subject follows `AppShell.selected` through `change_selection`, **debounced 250 ms** by a replaceable `Task` | each start is an uncached list plus watch; arrow-key navigation must not start one per row |
| 22 | Object event failures do not set the status bar "interrupted" state; the section shows them | a drawer without RBAC for `default` would otherwise flag the whole app |
| 23 | At most 50 event rows per drawer, then `+{n} more`. Rows are not clickable; a tooltip shows the full message | same bound as related pods; the Events screen is one click away in the sidebar |
| 24 | Screenshots: `events`, `events-drawer`, `pod-events`; `node-drawer` and `deployments-drawer` now show events | covers the screen, the event drawer, the tab, and a section |
| 25 | Event drawer follows W7: section **Details**, subtitle `{type} · {namespace} · {source}` | W7 drawer meta and section title |

## Cost model (API server)

The API server keeps **no watch cache for events**, and event field selectors are **not indexed**. Every list goes to etcd, and every filter runs after the read.

| Action | Server cost |
|---|---|
| Events screen, scope All | one paged list of every event in the cluster from etcd (500 per page), then one etcd watch on the events prefix; filtered server-side per event. Repeated on screen open, scope change, toggle, and every relist after a 410 or a backoff |
| Events screen, Named scope | the same, limited to the namespace's events |
| Warnings only on or off | a full relist: the server still reads every event in scope and drops the non-warnings. It saves bandwidth and client memory, not server reads |
| Drawer events | one list of the object's namespace (or `default`) from etcd, filtered by kind and name, then a filtered watch. Debouncing bounds it to one per settled selection |

## Memory model (client)

- Per summary: about 1.3 KiB worst case (1 KiB message plus fields); typical events are ~0.3 KiB.
- Steady state: 2,000 × 1.3 KiB ≈ 2.5 MiB per events watch.
- Worst case is about **3×** (~7.5 MiB) during a relist: the visible `items` (1×) plus a relist buffer that grows to 2× before `trim`. Add one raw page of 500 events and the snapshot in flight (one in the channel, one in the UI as rows).

## Known ceilings

- **Selection scroll.** With newest-first order, every new event shifts the selected row, so `SelectionSync::Move` scrolls to it. Pause stream (open item 1) fixes this.
- **Apply at the cap** sorts the store (O(n log n), n = 2,000) per inserted event. Measure before optimizing (a min-heap would do).
- **Warnings only with a drawer open.** Toggling it with an event drawer open briefly unmounts the drawer while the list reloads.
- **`default` namespace convention (decision 9).** Events of cluster-scoped objects are found only where client-go's recorder puts them (`default`). A recorder that writes them elsewhere is missed. Upgrade path: a cluster-wide read, only with `list events` at cluster scope and a measured need.
