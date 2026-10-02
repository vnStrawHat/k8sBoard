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
