# 0009 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; nothing lands before its first user.

## Cargo

No changes in any step: `futures` (`select_all`, `scan`) is already a dependency of both crates, and the kit's `Popover`, `Checkbox`, `Input`, and `DropdownMenu` exist in gpui-component 0.7. `git diff Cargo.lock` stays empty.

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/namespace.rs` | `NamespaceScope::Several`, `of_namespaces`, `namespaces`; tests |
| `src/connection.rs` (+ `connection_tests.rs`) | `scoped_api` → `scoped_apis`; `ClusterError::Namespace` |
| `src/resource_watch.rs` (+ `resource_watch_tests.rs`) | `summary_watch` / `limited_summary_watch` take `Vec<Api<K>>`; `merge_snapshots` (a `Batcher`-like struct with `BATCH_WINDOW` coalescing); `StoreLimit: Clone + Copy` |
| `src/access_review.rs` | `Several` arm (cluster checks once, then one namespace at a time), `review_checks`, `resource_attributes(check, Option<&str>)`, `AccessReport::all_of` |
| `src/pod.rs` (+ `pod_tests.rs`) | `PodSummary.labels`; `list_pods` over several apis |
| `src/config_map.rs`, `cron_job.rs`, `daemon_set.rs`, `deployment.rs`, `event.rs`, `ingress.rs`, `job.rs`, `replica_set.rs`, `service.rs`, `stateful_set.rs`, `namespace.rs`, `node.rs` | pass `scoped_apis(scope)` or `vec![api]` |
| `examples/probe.rs` | `--namespace a,b` via `of_namespaces`; usage text |
| app compile arms | `cluster_session.rs` (`namespaces_label`, `scope_label`), `title_bar.rs` label, `navigation.rs` reason, fixtures with `labels` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/table_view.rs` (new) + `table_view_tests.rs` | `TableRow`, `CellValue`, `TableView`, `default_filter`, `FilteredTable`; `tracing::trace!` of rebuild time (counts only) |
| 2 | `src/table_filter.rs` (new) + `table_filter_tests.rs` | filter model, `matches`, `parse_label_queries`, `contains_ignore_ascii_case`; step 3 adds `on_node`, `set_equals` |
| 2 | `src/table_sort.rs` (new) + tests in module | `TableSort`, `next_sort`, `compare_values`, `natural_cmp` |
| 2 | `src/table_layout.rs` | `TableColumns`, `layout_columns`, `sortable_header` (replaces `header_cell`) |
| 2 | `src/resource_kind.rs` | `KindColumn`, `Align` derive `Clone, Copy`; `kind_columns(kind)` |
| 2 | `src/pod_table.rs`, `src/node_table.rs`, `src/kind_table.rs` | `POD_COLUMNS`/`NODE_COLUMNS`, view-based rows, `TableRow` impls, `FilteredTable` impls, `shell` handle, filtered empty state |
| 2 | `src/table_selection.rs` (+ tests) | `list_row_index` searches the view |
| 2 | `src/filter_bar.rs` (new) | filter bar row, chips, + Filter, quick filter input, Columns ▾ |
| 2 | `src/workspace.rs` | filter bar under the header; `N of M match` |
| 2 | `src/app_shell.rs` | `update_view` and the toolkit API, quick filter entity and screen sync, rebuild on notify and screen switch, `reset_filter` on context switch, root `key_context("AppShell")` and `FocusQuickFilter` handler |
| 2 | `src/main.rs` | modules; `actions!` and the `/` binding (`AppShell && !Input`) |
| 2 | `src/launch_options.rs` (+ tests) | `--filter` |
| 3 | `src/node_summary.rs` (new) + tests in module | `NodeGroup`, `NodeCounts`, `node_counts`, `node_in_group`, `role_counts` |
| 3 | `src/node_table.rs` | `in_preset`, `common_version`, Version tint |
| 3 | `src/filter_bar.rs`, `src/workspace.rs` | summary chips; Hide inactive and Pause stream buttons; Nodes role header; paused suffix |
| 3 | `src/cluster_session.rs` (+ tests) | `StreamFlow`, `FlowState`, `KindList.flow`, `set_explorer_paused`, `explorer_flow` |
| 3 | `src/resource_actions.rs` (+ tests) | View pods on node; Filter similar |
| 3 | `src/app_shell.rs` | `view_pods_on_node`, `filter_similar`, `toggle_explorer_paused`, `toggle_hide_inactive` |
| 4 | `src/namespace_picker.rs` (new) + tests in module | state and popover |
| 4 | `src/title_bar.rs` | uses `namespace_picker`; old `namespace_menu`/`scope_item` removed |
| 4 | `src/filter_bar.rs` | Namespace chips |
| 4 | `src/app_shell.rs`, `src/cluster_session.rs` | picker state; requested scope as `NamespaceScope` |
| 4 | `src/launch_options.rs` (+ tests) | `--namespace a,b`, max 5 |
| 5 | `src/row_selection.rs` (new) + tests in module | `bulk_actions`, selection bar element |
| 5 | `src/table_view.rs`, `src/table_layout.rs`, three delegates (`render_tr` click modifiers), `src/workspace.rs`, `src/app_shell.rs` | checked set and anchor, checkbox column, Ctrl/Shift click, bar |
| 5 | `src/launch_options.rs` | `pods-selected`, `nodes-selected` |

## Doc updates (on merge of the last step)

- `docs/roadmap/inventory-shell.md`: T5, H1, H2, H3, H4, H6 → Done (H6 Partial: no bulk actions); H5 Partial (Hide inactive); N3 → the spec chosen for C11.
- `docs/roadmap/inventory-kinds.md`: Columns ▾, chips, sort → Done; Events Pause stream and Filter similar, Nodes View pods on node, ReplicaSets Hide inactive → Done.
- `docs/roadmap/gap-plan-read-only.md`: 0012 drops `PodSummary.labels`; C11 moves out of 0009.
- `docs/roadmap/README.md`: status table rows for the shell and tables.
