# 0046 · Inventory

[Back to index](README.md). Scoped greps of `crates/app/src` and `crates/cluster/src` at `6662108`. **D** delete, **S** simplify to one session, **K** keep. Lines are rough deletions (production only; tests in [test-plan.md](test-plan.md)).

`crates/cluster`: nothing multi-cluster (the "one cluster" hits in `access_review.rs`, `connection.rs` are unrelated). Untouched.

## Model and lifecycle

| File: items | Do | Step | ~Lines |
|---|---|---|---|
| `cluster_view.rs`: `ClusterView`, `ViewPlan`, `plan_view`, `TooManyClusters`, `MAX_VIEWED_CLUSTERS`, `riskiest`, `reorder`, `insert`, `take_all`, `sessions` | D | 2 (plan), 4 (struct) | 180 |
| `cluster_view.rs`: `ViewSlot` | S → `ActiveSession` in `active_session.rs`; file deleted | 4 | — |
| `app_shell_view.rs`: `view_clusters`, `apply_view`, `scope_for_view`, `release_slot`, `connect_slots`, `display_order`, `reapply_view_scope`, `set_slot_scope`, `retry_cluster`, `remove_from_view` | D | 2 | 300 |
| `app_shell_view.rs`: `new_slot`, `sync_view_sessions`, `refresh_slot_labels`, `on_first_live` | S (one session; `on_first_live` keeps sweep + `last_used`) | 2, 4 | — |
| `app_shell.rs`: `view`, `view_scope`, `view_request`, `view_connects`, `released_sessions`, `subject_cluster` | D (`view` → `active_session`) | 2, 4 | 60 |
| `app_shell.rs`: start `--view` branch, `named_clusters`; `switch_to` multi branches; `release_all` `set_multi` | D | 2 | 50 |
| `app_shell.rs`: `scope_live`, `sessions()`, fan-out loops in `set_namespace`, `show_screen`, `sync_kubelet_demand` | S (the one session) | 3 | 40 |
| `app_shell.rs`: `reveal_in_primary`, `primary_object`, `context_cluster` | D → `reveal`, `in_context` over `active_cluster()` | 3 | 25 |
| `app_shell.rs`: `row_object`, `row_of_item`, `table_addresses`, `merged_index` | S (item index direct) | 4 | 40 |
| `app_shell.rs`: `filter_by_cluster`, Cluster chip in `view_pods_on_node`, `sync_drawer_cluster` | D | 3 | 45 |
| `app_shell.rs`: `switch_cluster`, `switch_to`, `release_all`, `connect_active`, `record_leaving`, `back_to_previous` | K (0026) | — | — |

## Switcher, launch, screenshots

| File: items | Do | Step | ~Lines |
|---|---|---|---|
| `cluster_switcher.rs`: `ToggleClusterTick`, `Checkbox`, `ticked`, `tick_notice`, `tick_notice_line`, `ticks_footer`, primary bold | D; `space` → `gpui_kit::NoAction` in both contexts | 2 | 130 |
| `cluster_switcher_rows.rs`: `is_ticked`, `toggle_tick`, `ticks_differ`, `ViewedCluster.is_primary` | D | 2 | 40 |
| `cluster_switcher_rows.rs`: `normalize_query` | K (a typed space stays harmless) | — | — |
| `app_shell.rs`: `has_pending_ticks`, `toggle_cluster_tick`, `toggle_highlight_tick`, `clear_switcher_ticks`, `apply_switcher_ticks`, tick branch of `confirm_switcher_highlight`; `viewed_health` | D; `viewed_health` S | 2 | 60 |
| `launch_options.rs`: `--view`, `LaunchScreen::PodsMulti`, `pods-multi` | D | 2 | 40 |
| `screenshot.rs`: multi settle ("settles last among several clusters") | D | 2 | 30 |

## Shell chrome

| File: items | Do | Step | ~Lines |
|---|---|---|---|
| `title_bar.rs`: `TriggerText.plus/tooltip`, `border_environment` multi, `badge_face` / `badge_tooltip` several, lock `dropdown_menu` | S (active only) | 3 | 110 |
| `status_bar.rs`: `{n} clusters` line, API range | D branch | 3 | 30 |
| `namespace_picker.rs`: `union_content`, per-slot notes | D | 3 | 90 |
| `navigation.rs`: `SlotCounts`, `slots`, `slot_tooltip`, sums over slots | D (active counts) | 3 | 110 |
| `workspace.rs`: `render_multi_body`, `render_slot_notices`, `slot_notice_list`, `slot_notices`, `SlotState`, `SlotStatus`, `SlotNotice`, `ListFailure`, `render_slot_notice`, `multi_header_count`, `clusters_count_text`, `Showing {label} only`, `Retry all` | D (and `list_*` helpers once unused) | 2 | 400 |
| `dock.rs`: `is_multi`, `set_multi`; `log_tab.rs`: `tab_title` suffix; `close_tabs_of` | D (`cluster_label` fields only if then unused) | 3 | 40 |
| `drawer.rs`: `DrawerCluster` chip | D if unused after `sync_drawer_cluster` goes | 3 | 20 |
| `environment.rs`: `Environment: Ord` | K (name guessing uses it) | — | — |

## Tables, menus, palette

| File: items | Do | Step | ~Lines |
|---|---|---|---|
| `cluster_rows.rs`: `CLUSTER_COLUMN`, `Clustered`, `RowAddress`, `SlotRows`, `merge_rows`, `merge_slot_rows`, `merged_index` | D; file deleted | 4 | 140 |
| `cluster_rows.rs`: `SlotSession`, `RowContext` | S → `row_context.rs`: `TableSession`, `RowContext` without `is_primary`, `primary_label`, `is_multi` | 3, 4 | — |
| `pod_table.rs`, `node_table.rs`, `kind_table.rs`: `sessions: Vec`, `set_sessions`, `is_multi`, plans with the column, `slot_rows`, `addresses`, `cluster_column` | S (one `Option<TableSession>`) | 4 | 180 |
| `table_layout.rs`: `session_column`, `with_cluster_column`; `table_view.rs`: `TableRow::cluster`, `RowName.cluster`, prefs skip | D | 4 | 50 |
| `table_selection.rs`, `issue_table.rs`: `addresses` | S / D | 4 | 30 |
| `resource_actions.rs`: `Filter by this cluster`, `is_multi`, Topology "draws only the primary" disable | D | 3 | 45 |
| `palette_search.rs`: `sessions: Vec`, `label`, `is_primary`, `scope_session`, `with_cluster` | S (`session: Option<PaletteSession>`) | 3 | 45 |
| `command_palette.rs`: `@` row → `switch_cluster` | K | — | — |

## Write path and connect features

| File: items | Do | Step |
|---|---|---|
| `guard_for(&ClusterRef)`, `slot_session`, `slot_live`, `slot_connection` | K signature; body: the active session iff `cluster` matches | 4 |
| `write_lock.rs`: `lock_target` | S → `active_cluster()` | 3 |
| `write_lock.rs`: `toggle_write_lock(cluster)`, `finish_unlock` generation check, `slot_label` | K | — |
| `batch_write.rs` (`batch_plan`, `bulk_state`), `object_delete.rs` (`delete_scope`, `delete_gate`), `node_editor.rs` (`ticked_nodes`), `drain_dialog.rs` (`start_drain_of_ticked`): `Select rows of one cluster` | K ([write-safety.md](write-safety.md)) | — |
| `running_batches: HashSet<ClusterRef>`, session `generation`, `lock` | K | — |
| `port_forward_*`: forward `cluster`, `cluster_label`, page Cluster column, `Open {cluster} first`, `sync_forward_lock` | K (0035 AC 8); `slot_of` check → `== active_cluster()` | 4 |
| `leaving_work.rs`: `leaving_work(&[ClusterRef])`, `confirm_leaving` | K; doc drops "view change", "Remove from view" | 2 |
| `ClusterObject` and its 37 user files | K ([write-safety.md](write-safety.md)) | — |
