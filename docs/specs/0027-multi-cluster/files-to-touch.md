# 0027 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; every new item has a production user in its step (dead-code rule; test seams are `#[cfg(test)]`). `crates/cluster` is not touched. `Cargo.toml`/`Cargo.lock` unchanged.

## Prerequisites (merged first)

| Needs | Why |
|---|---|
| 0024 all steps | `ClusterRef` (`Hash`), `ClusterProfile`, `environment_color`, `Environment: Ord`, `last_used` on Live |
| 0025 steps 1–3 | `ClusterCatalog`, `cluster_groups` (or the 0026 fallback in `cluster_registry.rs`) |
| 0026 all steps | `switch_cluster`, `scope_memory`/`start_scope`, `cluster_switcher*`, health, weak menu sessions, the headless fixture (tokio runtime kept alive, `ClusterRuntime` global, writes off, catalog over dead-port fixtures; 0026 test-plan.md) |
| 0009 | `TableView`, `TableRow`, `FilterChip::Equals`, row checks |

## `crates/app`

| S | File | Change |
|---|---|---|
| 1 | `src/cluster_view.rs` (new) + `cluster_view_tests.rs` | `MAX_VIEWED_CLUSTERS`, `ViewSlot` (`has_reported_live`), `ClusterView`, `ViewPlan`, `plan_view` (primary rule), `TooManyClusters` |
| 1 | `src/cluster_session.rs` | `#[cfg(test)]` `live_fixture`, `seed_pods` (view-model.md "Test seams") |
| 1 | `src/app_shell_view.rs` (new, `#[path]` child of `app_shell.rs`, like `workspace.rs`) | the view lifecycle: `apply_view`, `scope_for_view`, `release_slot`, `connect_slots`, `new_slot`, `sync_view_sessions`, `refresh_slot_labels`, `display_order`, `on_first_live`, `reapply_view_scope`, `remove_from_view`, `retry_cluster` |
| 1 | `src/app_shell.rs` (+ tests) | `session` → `view`; `view_clusters`; `slot_live`; fan-out table; scope re-apply on a slot's first Live; `last_used` for the primary only; filters reset only on a new primary |
| 1 | `src/pod_table.rs`, `src/node_table.rs`, `src/kind_table.rs` | `set_session` → `set_sessions` (primary rows only until step 3a) |
| 1 | `src/title_bar.rs`, `src/status_bar.rs` (+ tests), `src/namespace_picker.rs` (+ tests) | `+N`, tooltip, riskiest border; multi status line; union with per-slot loading lines |
| 2 | `src/cluster_switcher_rows.rs` (+ tests) | `is_ticked`, `toggle_tick`, `ticks_differ`, `normalize_query`, whitespace-free `search_text` |
| 2 | `src/cluster_switcher.rs` | checkboxes, footer when ticks differ, `ToggleClusterTick` + `space` bindings, Enter rule, tick notice |
| 3a | `src/cluster_rows.rs` (new) + `cluster_rows_tests.rs` | `CLUSTER_COLUMN`, `Clustered`, `RowAddress`, `merge_rows` |
| 3a | `src/table_layout.rs`, `src/table_view.rs`, `src/pod_table.rs`, `src/node_table.rs`, `src/kind_table.rs` (+ tests) | plan with the column in multi mode; merged rebuild; `addresses`; cell; `prefs` skips the column; `set_sessions` drops it from `hidden`/`sort` |
| 3a | `src/workspace.rs`, `src/resource_actions.rs` (+ tests) | header count; row-menu `Filter by this cluster` |
| 3b | `src/table_selection.rs` (+ tests), `src/table_view.rs` | `ClusterObject`; checked keys and `RowName` carry the cluster |
| 3b | `src/app_shell.rs`, `src/workspace.rs`, `src/navigation.rs` | selection, reveal, pending subjects by `ClusterObject`; per-slot banners (failed, interrupted, connecting, denied); summed sidebar counts |
| 4 | `src/app_shell.rs` (+ tests) | every drawer/YAML/logs/Monitor read → `slot_live(&selected.cluster)`; `sync_yaml_view`; `follow_drawer_subjects` with `subject_slot`; `PendingSubjects::has_same_subjects` with cluster; `sync_kubelet_demand` per slot |
| 4 | `src/yaml_view.rs` (+ tests) | `cluster` field; `is_for(&ClusterObject)` |
| 4 | `src/log_dock.rs`, `src/log_tab.rs` (+ tests) | tab `cluster`; `is_for(&ClusterRef, ..)`; title suffix; close tabs of a released slot |
| 4 | `src/drawer.rs`, `src/pod_drawer.rs`, `src/kind_drawer.rs`, `src/node_drawer.rs` | slot session; header badge + label; menus capture weak session + `ClusterRef` |
| 4 | `src/launch_options.rs` (+ tests), `src/screenshot.rs` (+ tests) | `--screen pods-multi` with `--view <ctx>[,<ctx>]`; settle when every slot is Live with a loaded list or Failed |

## Follow-ups for the orchestrator (specs not owned here)

- **0028**: `space` left `RESERVED_KEYS` and the keymap.md Space row says "bound by 0027" (switcher-local, no sheet row). Done.
- **0029**: the `@` cluster scope may tick several clusters only by calling `view_clusters`; otherwise it switches one (0026).
- **0020, 0021, 0022, 0035** (when implemented): follow [aggregated-views.md](aggregated-views.md) "Screens that land later".
- **0030**: the action gate takes `ClusterObject.cluster` of the row, never the primary.

## Docs (after merge)

- `docs/roadmap/inventory-shell.md`: T3, T4 (multi), H8, M1 → Done or Partial per merged screens.
- `docs/roadmap/cross-cutting.md` C4: cap 5, keep-on-apply.
- [budget.md](budget.md) "Results" filled.
