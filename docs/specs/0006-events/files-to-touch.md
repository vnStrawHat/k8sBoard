# 0006 · Files to touch

[Back to index](README.md). Based on HEAD `e80ed80`. **S** is the implementation step. Each step passes the gate on its own; an item lands in the step of its first user.

## Cargo

No changes. k8s-openapi `v1_32` has `core::v1::Event`; kube-runtime 4.2 has `watcher::Config::fields`; gpui has `line_clamp` and `background_executor().timer`; the kit has `Selectable` buttons and `theme.link`.

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/event.rs` (new) + `src/event_tests.rs` once tests pass ~150 lines | `EVENT_LIMIT`, `EventType`, `EventFilter`, `InvolvedObject`, `EventSummary`, `event_summary`, `truncate_message`, `event_source`, the two selectors, the `default` namespace constant, `watch_events`, `watch_object_events` |
| `src/resource_watch.rs` | `StoreLimit`, `limited_summary_watch`, the shared core, `batch_updates` limit parameter, `SummaryStore::new(limit)`, `trim` |
| `src/resource_watch_tests.rs` | existing calls pass `None`; new store-limit tests |
| `src/lib.rs` | `mod event;` and its exports |
| `examples/probe.rs` | `events` and `warning events` tallies (the first user of `EventFilter`); doc comment |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/resource_kind.rs` | `Events`, `ALL: [_; 11]`, `NameColumn`, KindSpec `object_kind`, `name_column`, `has_labels`, `from_object_kind`; Events arm with `EventFilter::All` |
| 2 | `src/kind_row.rs` | `KindRow.event`, `EventDetail`, `KindCell::Qualified`, `DetailRow::{Code, Link}` |
| 2 | `src/event_rows.rs` (new) | `event_rows`, `event_row`, `event_tone`, `object_text`, `object_key`, `message_line`. Inline tests |
| 2 | `src/workload_rows.rs`, `batch_rows.rs`, `network_rows.rs`, `config_map_rows.rs`, `namespace_rows.rs`, `resource_kind.rs` tests | `event: None` |
| 2 | `src/kind_table.rs` | `columns` and `fit_width` per `NameColumn`, `cell_index`, `qualified_text`, `Qualified` cell, `shell` field |
| 2 | `src/kind_drawer.rs` | event title and subtitle, `Code` and `Link` rows, `has_labels`, `&Context<AppShell>` in `detail_element`, shell for the menu, `reveal` |
| 2 | `src/resource_actions.rs` | `kind_menu` event group and `shell` parameter |
| 2 | `src/table_selection.rs` (+ tests) | `ResourceKey::screen` |
| 2 | `src/app_shell.rs` | `reveal` (replaces `reveal_pod`), `KindTableDelegate::new(kind, cx.weak_entity())`, `Clear` through `change_selection` |
| 2 | `src/workspace.rs` | Events header count with `group_digits` |
| 2 | `src/navigation.rs` | test lists include Events |
| 2 | `src/launch_options.rs` | `USAGE` lists `events` |
| 2 | `src/main.rs` | `mod event_rows;` |
| 3 | `src/object_events.rs` (new) | `event_subject`, `SubjectChange`, `subject_change`, `events_title`, `event_note`, `recent_events`. Inline tests |
| 3 | `src/event_rows.rs` | `newest_first` (its first user is the object events subscription) |
| 3 | `src/cluster_session.rs` (+ tests) | `ObjectEvents`, `set_event_subject`, `events_of`, `is_object_events_loading`, `open_watch_count` |
| 3 | `src/app_shell.rs` | `EVENT_SUBJECT_DELAY`, `event_subject_task`; `change_selection` applies `subject_change`; `close_drawer` through it; `settle_input` uses `is_drawer_ready`; `PodEvents` tab setup |
| 3 | `src/drawer.rs` | `PodDrawerTab::Events` |
| 3 | `src/pod_drawer.rs` | Events tab |
| 3 | `src/node_drawer.rs`, `src/kind_drawer.rs` | Events section |
| 3 | `src/launch_options.rs` (+ tests) | `LaunchScreen::PodEvents`, `pod-events`, `USAGE` |
| 3 | `src/screenshot.rs` (+ tests) | `is_drawer_ready` |
| 3 | `src/main.rs` | `mod object_events;` |
| 4 | `src/resource_kind.rs` | `watch_rows(.., events: EventFilter)` |
| 4 | `src/cluster_session.rs` | `event_filter`, `event_filter()`, `set_event_filter`, filter passed to every explorer start |
| 4 | `src/app_shell.rs` | `toggle_warnings_only` |
| 4 | `src/workspace.rs` | Warnings only button |

All new app items are private or `pub(crate)`. The app still has no kube dependency.

## Docs (in the step that changes the behavior)

| S | Change |
|---|---|
| 1 | 0001 `probe-example.md`: `--watch-seconds` lists 14 watches |
| 1 | 0002 `README.md` non-goal "Events (deferred)": point to 0006 |
| 2 | 0005 `decisions.md` "Pre-built drawer sections" ceiling: point to 0006 decision 12 |
| 2 | 0003 `screenshot-hook.md`: `events`, `events-drawer` |
| 3 | 0003 `screenshot-hook.md`: `pod-events` |
