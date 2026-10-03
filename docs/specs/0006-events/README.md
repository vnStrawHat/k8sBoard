# 0006 — Events (read-only)

Status: amended after advisor review. Crates: `crates/cluster` and `crates/app`. Based on HEAD `e80ed80` (0005 merged). Wireframes: W7 "Events" kind, W4 drawer "Events" tab. [0007](../0007-yaml-tab/README.md) later moves the drawer Events section into a tab.

## Goal

- **Cluster › Events** becomes a live screen: core/v1 events, newest first, Warning toned, with a server-side **Warnings only** toggle. It reuses the 0005 explorer (`ResourceKind`, `KindRow`, kind table, kind drawer).
- The event drawer shows details and the full message, and **Go to object** reveals the involved object when k8sBoard has a screen for its kind.
- Pod, Node, and kind drawers show the object's **recent events**, from one debounced, field-selected watch that lives only while that drawer is open.
- Memory stays bounded: an events store keeps at most `EVENT_LIMIT` (2,000) newest events, and messages are cut at 1 KiB.

## Non-goals

Pause stream, Filter similar, filter chips, sorting by other columns, events.k8s.io/v1, the Overview/Issues screens, event YAML, deleting events, persisting or exporting events, event matching by object uid, and clicking drawer event rows.

## Implementation steps

Each step is one coder pass and passes the full quality gate on its own. Nothing lands before its first user.

| Step | Scope | ACs |
|---|---|---|
| 1 | `crates/cluster`: `EventSummary` and mapping, store limit in `resource_watch.rs`, `watch_events`, `watch_object_events`, probe | 1, 2, 3, 4, 5, 8 |
| 2 | App Events screen: `ResourceKind::Events`, hidden Name column, `event_rows.rs`, drawer rows, menu, `reveal`, `events` and `events-drawer` screens | 1, 2, 3, 6, 9, 10 |
| 3 | Per-object events: session `ObjectEvents`, debounced `set_event_subject`, pod Events tab, node and kind sections, `pod-events` screen, full ui-verifier run | 1, 2, 7, 9, 10 |
| 4 | Warnings only: session `event_filter`, `watch_rows` filter, header toggle | 1, 2, 11 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale, cost model, known ceilings |
| [cluster-events.md](cluster-events.md) | `EventSummary`, field mapping, watches, store limit, probe |
| [events-screen.md](events-screen.md) | `ResourceKind::Events`, columns, hidden Name column, rows, Warnings only |
| [event-drawer.md](event-drawer.md) | `EventDetail`, drawer sections, event menu, `reveal` |
| [object-events.md](object-events.md) | drawer events: debounce, lifecycle, subjects, renderers, screenshots |
| [files-to-touch.md](files-to-touch.md) | modules per crate and step |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [x] 1. The quality gate passes, and so does `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes, offline.
- [ ] 3. No kube or k8s-openapi type in a public signature. The 0001 read-only grep still finds only the SSAR `create`. The crate never spawns tasks. The app has no kube dependency. — superseded by 0030 (write allow-list and named connect files replace the read-only grep)
- [ ] 4. Secret safety: `EventSummary` keeps no annotations, labels, or `managedFields`; its message is at most 1 KiB plus `…`. `event.rs`, `event_rows.rs`, and `object_events.rs` contain no `tracing::` call. Nothing is written to disk.
- [ ] 5. Probe `--watch-seconds 5` prints 14 watch lines (adds `events` and `warning events`, counts only) and 19 access lines. The 0001 AC7 credential script reports 0.
- [ ] 6. On UAT, Events shows live rows newest first, with Warning rows toned. The drawer shows Details and Message sections; Go to object opens a pod, a node, and a deployment.
- [ ] 7. At most 5 watches per session (namespaces, pods, nodes, explorer, object events). The object events watch starts only after the selection has rested for 250 ms and exists only while a non-Event drawer is open (review of `change_selection` and `set_event_subject`). — superseded by later watch-budget growth (see open_watch_count_stays_within_3n_plus_5 in cluster_session_tests.rs)
- [x] 8. An events watch never emits more than `EVENT_LIMIT` items (`limited_watch_snapshot_holds_at_most_limit`).
- [ ] 9. The 0006 screenshots of the step exist, and the ui-verifier reports no high-severity defect against W7 Events and W4.
- [x] 10. The 0003 AC4 color-literal grep is clean.
- [ ] 11. On UAT, Warnings only reloads the Events list with warnings only, and turning it off restores all events.

## Open items

1. Pause stream and Filter similar (W7 Events toolbar and menu) need a frozen-snapshot mode and filter chips.
2. Object events match by kind and name, not uid: a recreated object shows its predecessor's events for up to an hour.
3. Node and Namespace events are read from the `default` namespace (the client-go recorder convention). Without `list events` in `default`, the section reads "Events are unavailable". Events that other recorders put elsewhere are missed.
4. A selected event row moves when newer events arrive, and selection sync scrolls to it (see decisions, ceilings).
5. The empty text does not mention Warnings only ("No events in team-a").
6. Overview "Needs attention" and Issues will consume Warning events; they need their own spec.
