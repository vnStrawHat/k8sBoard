# 0046 · Inventory

[Back to index](README.md). Scoped greps of `crates/app/src` and `crates/cluster/src` at `6662108`. **D** delete, **S** simplify to one session, **K** keep. Lines are rough deletions (production only; tests in [test-plan.md](test-plan.md)).

`crates/cluster`: nothing multi-cluster (the "one cluster" hits in `access_review.rs`, `connection.rs` are unrelated). Untouched.

## Model and lifecycle

| File: items | Do | Step | ~Lines |
|---|---|---|---|
| `cluster_view.rs`: `ClusterView`, `ViewPlan`, `plan_view`, `TooManyClusters`, `MAX_VIEWED_CLUSTERS`, `riskiest`, `reorder`, `insert`, `take_all`, `sessions` | D | 2 (plan), 4 (struct) | 180 |
| `cluster_view.rs`: `ViewSlot` | S → `ActiveSession` in `active_session.rs`; file deleted | 4 | — |
| `app_shell_session.rs`: `view_clusters`, `apply_view`, `scope_for_view`, `release_slot`, `connect_slots`, `display_order`, `reapply_view_scope`, `set_slot_scope`, `retry_cluster`, `remove_from_view` | D | 2 | 300 |
| Orphans of `release_slot`: `dock.rs` `close_tabs_of` (~399), `edit_yaml_flow.rs` `close_edit_of` (~155), `cluster_view.rs` `ClusterView::remove` (~138) | D (`release_all` closes every tab and sets `edit = None`) | 2 | 30 |
| `#[cfg(test)]` hooks read only by `app_shell_multi_tests.rs`: `view_connects`, `ViewConnectCheck`, `released_sessions` | D with the file | 2 | 25 |
| `app_shell_session.rs`: `new_session`, `sync_view_sessions`, `refresh_slot_labels`, `on_first_live` | S (one session; `on_first_live` keeps sweep + `last_used`) | 2, 4 | — |
| `app_shell.rs`: `view`, `view_scope`, `view_request`, `view_connects`, `released_sessions`, `subject_cluster` | D (`view` → `active_session`) | 2, 4 | 60 |
| `app_shell.rs`: start `--view` branch, `named_clusters`; `switch_to` multi branches; `release_all` `set_multi` | D | 2 | 50 |
| `app_shell.rs`: `sessions()`, fan-out loops in `set_namespace`, `show_screen`, `sync_kubelet_demand` | S (the one session) | 3 | 40 |
| `scope_live`: `app_shell.rs` (~1501, 1861, 4004, 4466), `keyboard_navigation.rs` (~374), `namespace_picker.rs` (~192), `title_bar.rs` (~242) | D → `live(cx)` | 3 | 15 |
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
| `guard_for(&ClusterRef)`, `session_of`, `live_of`, `connection_of` | K signature; body: the active session iff `cluster` matches | 4 |
| `write_lock.rs`: `lock_target` | S → `active_cluster()` | 3 |
| `write_lock.rs`: `toggle_write_lock(cluster)`, `finish_unlock` generation check, `label_of` | K | — |
| `batch_write.rs` (`batch_plan`, `bulk_state`), `object_delete.rs` (`delete_scope`, `delete_gate`), `node_editor.rs` (`ticked_nodes`), `drain_dialog.rs` (`start_drain_of_ticked`): `Select rows of one cluster` | K ([write-safety.md](write-safety.md)) | — |
| `running_batches: HashSet<ClusterRef>`, session `generation`, `lock` | K | — |
| `port_forward_*`: forward `cluster`, `cluster_label`, page Cluster column, `Open {cluster} first`, `sync_forward_lock` | K (0035 AC 8) | — |
| `release_all`: forwards of the released cluster | K: no lock change; they keep their last control (decision 17) | — |
| `port_forward_dialogs.rs` New-forward form: cluster `Select` (~214), `FormCluster` list over slots (~355–364); `port-forward-new-fixture` over two clusters | S: one `FormCluster` from the active session, a fixed badge + label (no `Select`); fixture over one fixed cluster | 4 |
| `node_shell_sweep.rs` `show_leftover_notice` | A: drop the notice when `self.session_of(&cluster).is_none()` at show time (Review already refuses via `guard_for`) | 2 |
| `leaving_work.rs`: `leaving_work(&[ClusterRef])`, `confirm_leaving` | K; doc drops "view change", "Remove from view" | 2 |
| `ClusterObject` and its 37 user files | K ([write-safety.md](write-safety.md)) | — |

## Connect and write-path callers of the view (exact replacements, step 4)

Rule: a check on a named cluster becomes `self.session_of(&x).is_some()` (or `label_of`, `live_of`), **never** `self.active_session.is_some()`; the cluster check must survive.

| Site | Today | Becomes |
|---|---|---|
| `debug_open.rs` ~137 (debug options) | `self.view.slot_of(&pod.cluster).map(\|i\| self.view.slots()[i].label.clone())` | `self.label_of(&pod.cluster)` |
| `debug_open.rs` ~281 `has_owner` (node-shell / debug create landed) | `… && self.view.slot_of(&plan.cluster).is_some()` | `… && self.session_of(&plan.cluster).is_some()`; an A → B switch mid-create opens no tab and deletes the pod on the held connection |
| `node_shell_open.rs` ~117, `shell_open.rs` ~143 | label via `slot_of` + `slots()` | `self.label_of(cluster)` / `self.label_of(&open.cluster)` |
| `drain_driver.rs` ~105, ~124 (drain start) | `Some(index) = self.view.slot_of(&cluster)` … `slots()[index].label` | `Some(cluster_label) = self.label_of(&cluster)` in the same `let (…) else` |
| `drain_driver.rs` ~186 `stop_all_drains_now`; `node_shell_cleanup.rs` ~158 quit check | `running_drains_of(&self.view.clusters())`, `running_drain_names_of(&self.view.clusters())` | every running drain tab, whatever its cluster (a dock query without a cluster filter); never only the active cluster |
| `port_forward_page.rs` ~402 | `self.view.slot_of(&forward.cluster).is_some()` | `self.session_of(&forward.cluster).is_some()` |
| `write_lock.rs` ~153 `label_of` | over `slots()` | moves to `app_shell.rs`: `self.active_session.as_ref().filter(\|open\| open.cluster == *cluster).map(\|open\| open.label.clone())` |
| `object_delete.rs` ~822 `still_ready` | `guard_for(&cluster)` + generation | unchanged (test (a) in write-safety.md) |
