# 0045 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; every new item has a production user in its step (test seams are `#[cfg(test)]`). `crates/cluster`, `Cargo.toml`, and `Cargo.lock` are not touched. Baseline main `a50264c`.

## `crates/app`

| S | File | Change |
|---|---|---|
| 1 | `src/issue_table.rs` (+ `issue_table_tests.rs`) | `sessions`, `addresses`, `set_sessions`, `issue_plan`, `sort_in_board_order`, `slot_issues`, `issue_at` with slot, Cluster cell, row menu per slot + `with_cluster_filter`, `open_item` → `reveal_object` |
| 1 | `src/issue_board.rs` (+ tests) | `sum_summaries` |
| 1 | `src/navigation.rs` (+ tests) | `merge_issue_counts`; `SlotCounts.issues`; Issues tooltip per cluster |
| 1 | `src/title_bar.rs` | flag and tooltip over all slots |
| 1 | `src/app_shell.rs` (+ `app_shell_multi_tests.rs`) | `issue_object`, `reveal_issue`, `reveal_first_issue`; delete `reveal_in_primary`; `set_issues_visible` on every slot; `navigation_counts` sums issues |
| 1 | `src/app_shell_view.rs` | `sync_view_sessions`: `issue_table.set_sessions` |
| 1 | `src/workspace.rs` | `multi_header_count` Issues arm; `issue_summary`, `render_issues_status`, `render_issues` over slots; remove the Issues part of the `Showing {label} only` notice and of the primary-only body |
| 1 | `src/resource_actions.rs` | `with_cluster_filter` becomes `pub(crate)` (the issue menu uses it) |
| 1 | `src/cluster_session.rs` | `#[cfg(test)] seed_nodes` |
| 1 | `src/overview.rs`, `src/node_heatmap.rs` | call sites only: primary `ClusterObject` + `reveal_object` (replaced in step 2) |
| 2 | `src/overview.rs` (+ tests in module) | `OverviewSlot`, `OverviewData.slots`, `merge_attention`, `merge_changes`, `stats_parts(&[&LiveCluster])`, `stats_line_of`, per-slot Capacity and Nodes blocks, cluster chip, click handlers with the slot's `ClusterObject` |
| 2 | `src/node_heatmap.rs` | `node_heatmap(cells, row, cx)` reveals in the row's cluster |
| 2 | `src/overview_report.rs` (+ tests) | `multi_report(slots) -> String` (one `live_report` per slot, `---` joined, not-connected lines) |
| 2 | `src/app_shell.rs`, `src/app_shell_view.rs` | `set_overview_visible` on every slot and in `new_slot`; `export_overview_report` multi path and cancel rule |
| 2 | `src/workspace.rs` | Overview header `{n} clusters`, summed stats line, build `OverviewSlot`s; remove the Overview part of the notice and primary-only body |
| 3 | `src/app_shell.rs`, `src/app_shell_view.rs` (+ `app_shell_multi_tests.rs`) | `topology_cluster`, `topology_slot`, `set_topology_cluster`, `topology_object`; `select_on_topology`; `show_in_topology(&ClusterObject)`; clear on release and single switch |
| 3 | `src/topology_view.rs` (+ tests) | `set_session` stops the old session's subject first |
| 3 | `src/workspace.rs` | Topology cluster dropdown in `topology_header_buttons`; topology slot body; remove the last of the notice |
| 3 | `src/resource_actions.rs` (+ tests) | `topology_menu` without `other_primary`; `show_in_topology_item(ClusterObject)` |
| 3 | `src/cluster_rows.rs`, `src/cluster_view.rs` | drop `is_primary` / `primary_label` from `RowContext` / `SlotSession` if no reader remains |
| 3 | `src/launch_options.rs` (+ tests), `src/screenshot.rs` (+ tests) | `--screen overview-multi`, `issues-multi`, `topology-multi` (with `--view`); settle when every slot is Live with its board summary (Overview, Issues) or its topology build (Topology), or Failed |

## Docs

| S | File | Change |
|---|---|---|
| — | `docs/specs/0027-multi-cluster/README.md`, `aggregated-views.md` | open items 4–6 and "Screens that land later" link this spec (done in this revision) |
| 3 | `docs/roadmap/inventory-screens.md`, `inventory-shell.md`, `wireframe-gap-audit.md` | W1 n6, W3, W11 multi rows → Done (0045) |
| 3 | [budget.md](budget.md) | "Results" |
