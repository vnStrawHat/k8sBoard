# 0022 · Test plan

[Back to index](README.md). Unit tests are offline and deterministic: fixed `now`, fixture summaries built into `KindRow`s through the real row builders. Each test checks one behavior. Names are binding (AC 2).

## Step 1

| Module | Tests |
|---|---|
| `topology_graph_tests.rs` | `deployment_owns_replica_set_owns_pods`, `inactive_replica_set_is_hidden`, `stateful_and_daemon_sets_own_pods`, `service_routes_to_matching_pods`, `selectorless_and_external_name_services_route_nowhere`, `ingress_routes_to_backend_and_default_services`, `ingress_tls_secret_is_a_mount`, `hpa_scales_its_target`, `config_refs_aggregate_to_top_workload`, `standalone_pod_keeps_its_refs`, `env_env_from_projected_and_claim_refs_are_read`, `projected_and_pull_secrets_are_refs`, `kube_root_ca_is_ignored`, `unreferenced_config_is_hidden`, `pending_claim_is_shown_without_refs`, `ref_with_loading_feed_is_not_checked`, `ref_with_off_feed_is_not_checked`, `pod_set_above_limit_becomes_group`, `bad_pods_stay_out_of_group`, `pod_captions_name_status_and_container`, `workload_captions_show_ready_counts`, `routed_service_caption_is_ingress_path`, `ids_and_edges_are_sorted_and_deduped`, `input_order_does_not_change_graph`, `resources_count_group_members`, `raw_input_above_limit_is_too_large_without_building`, `too_large_above_node_limit`, `secret_nodes_hold_names_only`, `group_by_defaults_to_app_with_labels`, `group_by_falls_back_to_components` |
| `topology_checks.rs` | `service_without_matching_pods_gets_ghost`, `no_pods_check_waits_for_pods`, `ingress_to_missing_service_is_bad`, `missing_config_map_and_secret_are_warn`, `missing_pull_secret_is_warn`, `missing_claim_is_bad`, `pending_claim_after_grace`, `pending_claim_within_grace_is_quiet`, `lost_claim_is_bad`, `hpa_missing_target`, `off_feed_skips_missing_checks`, `why_box_tones_node_and_adds_check`, `ingress_missing_tls_box_is_not_duplicated`, `pod_failure_tones_without_check`, `checks_sort_bad_first`, `chip_labels_singular_and_plural`, `coverage_names_off_and_loading_feeds`, `coverage_skips_chip_disabled_kinds` |
| `topology_layout_tests.rs` | `columns_follow_kind_table`, `config_row_sits_under_band`, `config_node_aligns_under_first_source`, `config_row_collision_moves_right`, `missing_ghost_takes_its_kind_placement`, `components_become_bands_largest_first`, `singletons_share_last_band`, `barycenter_untangles_shared_service_fixture` (Services `a`, `b`; Deployments `x`, `y`; `a` selects `y`'s pods and `b` selects `x`'s; the initial name order crosses twice; after sweeps, a crossing-count helper reads 0), `layout_is_deterministic_under_shuffle`, `short_columns_are_centered`, `extent_covers_nodes_and_margin`, `structure_ignores_status_changes`, `structure_changes_with_new_pod`, `new_pod_keeps_siblings_in_place` (with `previous`: existing rects unchanged; the new pod goes last in its column), `previous_drops_removed_ids`, `topology_budget` (40 Services, 3,000 pods, 100 ReplicaSets; build + layout ≤ 80 ms in the debug gate; prints the elapsed time) |
| `topology_canvas.rs` | `screen_graph_round_trip`, `zoom_at_keeps_cursor_point`, `zoom_steps_are_on_the_grid`, `zoom_is_clamped`, `fit_picks_grid_zoom_and_never_upscales`, `center_on_puts_point_mid_view`, `forward_edge_runs_right_to_left_side`, `mounts_edge_runs_bottom_to_top`, `backward_edge_runs_under_nodes`, `arrow_head_points_at_target`, `visible_nodes_culls_offscreen` |
| `topology_feeds.rs` | `plan_starts_all_kinds_while_checking`, `plan_marks_denied_kind_off`, `chip_off_kinds_get_no_feed`, `open_count_skips_off_feeds`, `rows_reports_loading_ready_off` |
| `cluster_session_tests.rs` | `open_watch_count_includes_topology` (hidden +0; visible +10; one denied +9; Config off +7), `row_of_prefers_explorer_then_topology`, `same_topology_subject_is_noop`, `subject_change_keeps_unchanged_feeds` |
| `kind_join_tests.rs` (0012) | `service_health_core_matches_list_wrapper` |
| `navigation.rs` | `enabled_items_are_pods_nodes_and_explorer_kinds` updated (Topology enabled) |
| `launch_options.rs` | `screen_topology_parses` |
| `screenshot.rs` | `topology_waits_for_feeds_and_build` |

## Step 2

| Module | Tests |
|---|---|
| `topology_graph_tests.rs` | `hidden_kind_filter_drops_nodes_and_edges`, `problems_only_keeps_neighbours`, `expanded_group_shows_pods`, `checks_follow_visible_nodes`, `app_group_from_labels`, `app_group_from_neighbour`, `unlabelled_component_is_ungrouped` |
| `topology_layout_tests.rs` | `app_bands_are_titled_and_sorted`, `ungrouped_band_is_last`, `pinned_node_keeps_its_origin` |
| `topology_canvas.rs` | `minimap_maps_both_ways`, `drag_below_slop_is_a_click` |
| `launch_options.rs` | `screen_topology_problems_parses` |
| `resource_actions_tests.rs` or kind-menu tests | `show_in_topology_disabled_outside_scope` |

## Step 3

| Module | Tests |
|---|---|
| `topology_export.rs` | `svg_has_a_node_per_graph_node`, `svg_escapes_names`, `svg_dash_per_relation`, `svg_uses_style_colors`, `hex_formats_rrggbb`, `svg_title_names_namespace`, `fit_chars_cuts_with_ellipsis`, `render_png_writes_png_signature` (a text-free SVG; checks `\x89PNG`), `export_scale_caps_longest_side_at_4096`, `svg_path_extension_is_case_insensitive`, `export_error_messages_name_the_cause` |

## Steps 4a, 4b — RBAC layer ([rbac-layer.md](rbac-layer.md))

Fixtures: namespace `shop` with Deployment `api` (pods run as `api`), pod `cron-x` without owner (runs as `default`), ServiceAccounts `api`, `default`; RoleBinding `api-reader` (`sa/api` → Role `reader`), RoleBinding `ghost` (→ missing Role `gone`), ClusterRoleBinding `ci-admin` (`sa/shop/api` → ClusterRole `cluster-admin`), ClusterRoleBinding `all-sa` (group `system:serviceaccounts` → ClusterRole `view`).

| Step | Module | Tests |
|---|---|---|
| 4a | `topology_graph_tests.rs` | `rbac_off_draws_no_access_nodes`, `account_aggregates_to_top_workload`, `standalone_pod_keeps_its_account_edge`, `direct_bindings_and_their_roles_are_drawn`, `group_bindings_only_count_in_caption`, `unused_accounts_and_bindings_are_hidden`, `cluster_role_node_is_plain_and_unchecked`, `access_edges_use_the_access_relation`, `rbac_rows_count_toward_raw_limit`, `only_cluster_role_bindings_of_namespace_accounts_count`, `other_namespace_role_bindings_are_not_drawn` |
| 4a | `topology_checks_tests.rs` | `missing_service_account_is_bad`, `binding_to_missing_role_is_warn`, `cluster_admin_account_is_warn_on_the_binding`, `cluster_admin_through_group_is_warn_on_the_account`, `cluster_admin_to_authenticated_is_warn_on_the_account`, `off_rbac_feed_skips_access_checks`, `access_chip_labels_singular_and_plural` |
| 4a | `topology_layout_tests.rs` | `access_row_sits_under_the_config_row`, `binding_and_role_take_the_next_slots`, `access_row_wraps_after_the_last_slot`, `new_pod_moves_no_access_card`, `topology_budget` (extended: + 40 accounts, 80 bindings, 40 roles) |
| 4b | `topology_feeds.rs` tests | `rbac_chip_starts_five_feeds`, `default_chips_leave_rbac_off`, `denied_rbac_feed_is_off`, `open_count_is_at_most_fifteen` |
| 4b | `cluster_session_tests.rs` | `open_watch_count_includes_topology` (extended: RBAC on +4, one denied +3) |
| 4a | `topology_colors.rs` tests | `kind_hues_avoid_the_tone_tokens` (extended: `Access`), `access_relation_is_cyan_light` |
| 4a, 4b | `topology_export.rs` tests | `svg_dash_per_relation` (extended: `access`, 4a), `legend_has_four_entries` (4b) |
| 4b | `topology_view_tests.rs` | `rbac_chip_is_enabled_and_toggles`, `click_on_feedless_row_reveals_instead_of_drawer` |
| 4b | `launch_options.rs` tests, `screenshot.rs` tests | `screen_topology_rbac_parses`, `topology_rbac_waits_for_rbac_feeds` |

Live (coder-lite, UAT, read-only): `--screen topology-rbac --namespace <ns>`; the RBAC nodes match the probe's list of ServiceAccounts and RoleBindings of `<ns>` (counts only); the watch count rises by the RBAC feeds that the access review allows. ui-verifier: `topology-rbac` light and dark: chip on, `access` legend entry, access row under the bands.

## Live checks (coder-lite, UAT `readonly@Monitor`, read-only)

1. `k8sboard --context readonly@Monitor --namespace <ns> --screen topology`, with a namespace that has Deployments and Services.
   - ReplicaSet → Pod edges match `kubectl get rs,pods -n <ns> --context readonly@Monitor -o wide`.
   - For each Service, its pod edges (and group members) match `kubectl get pods -n <ns> -l <selector> --context readonly@Monitor`. The selector is the Service's `spec.selector` joined with `,`.
2. The status-bar watch count rises by the started feeds on Topology, falls by 3 with Config off, and returns to normal on leaving.
3. A rollout or a scaled pod (if one happens during the check) appears without moving its siblings. If none happens, record that.
4. Never print the kubeconfig. The commands above use only `get`.

## ui-verifier checklist (W11)

- Header: `Topology`, `ns: … · N resources`, with `Fit` and `Export PNG` on the right. Toolbar order: segment, namespace, Group by (app), chips (RBAC ghosted), and the checks chip on the right.
- Columns run left to right: Ingress/HPA, Service/workloads, ReplicaSets, Pods. Config sits in a row under its workload. Bad cards have a red border; ghosts a dashed red one; unchecked a dashed muted one.
- Edge styles match the glyph legend. The minimap is bottom right, with the legend to its left. The dot grid shows at 100 %.
- A click opens the drawer over the graph; Esc closes it. Screenshots: `topology`, `topology-problems`. Light and dark themes are both readable.

## Large-namespace changes (decisions 37–41)

| Module | Tests |
|---|---|
| `topology_viewport.rs` | `fit_goes_below_the_wheel_floor_to_show_everything`, `the_wheel_does_not_push_a_fitted_view_back_up`, `first_view_fits_a_graph_that_is_readable_whole`, `first_view_of_a_large_graph_is_readable_and_anchored_top_left` (the viewport and minimap tests of `topology_canvas.rs` moved here) |
| `topology_layout_tests.rs` | `bands_flow_into_band_columns_to_match_a_wide_canvas`, `band_columns_do_not_overlap`, `band_packing_is_deterministic`, `adding_a_pod_moves_no_other_card`, `a_new_band_joins_the_shortest_column_and_keeps_the_others`, `config_row_wraps_after_the_last_slot` |
| `topology_canvas.rs` | `the_level_of_detail_steps_down_with_the_zoom`, `a_curve_is_inside_the_box_of_its_four_points` |
| `topology_checks_tests.rs`, `topology_feeds.rs` | `coverage_says_watch_failed_only_for_a_failed_feed`, `a_failed_feed_draws_its_targets_unchecked`, `a_feed_that_failed_reads_as_failed_not_loading` |
| `topology_graph_tests.rs`, `topology_view.rs` | `equal_inputs_build_equal_graphs`, `the_header_says_loading_too_large_or_the_count`, `a_selection_is_gone_only_from_a_feed_that_has_loaded`, `a_selection_waits_for_a_feed_that_is_not_ready`, `a_pod_is_gone_when_the_loaded_pods_lack_it` |
