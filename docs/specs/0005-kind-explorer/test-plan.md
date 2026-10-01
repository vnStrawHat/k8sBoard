# 0005 · Test plan

[Back to index](README.md). **S** is the step that adds the test. Tests are deterministic and offline. Fixtures are k8s-openapi structs with `..Default::default()`, and each test checks one behavior.

## `crates/cluster` (all in step 1)

| File | Test |
|---|---|
| `workload.rs` | `selector_terms_format_labels_and_expressions` (all four operators) |
| `workload.rs` | `controller_ref_is_owner_with_controller_true` (moved from `pod_tests.rs`) |
| `workload.rs` | `template_containers_read_main_images_and_ports` (init excluded, protocol defaults to TCP) |
| `workload.rs` | `template_containers_keep_only_name_image_and_ports`. The fixture container has an `env` value and an `arg` with distinctive strings; `format!("{containers:?}")` contains neither |
| `workload.rs` | `int_or_string_text_reads_percent_and_number` |
| `deployment.rs` | `deployment_summary_reads_replica_counts` |
| `deployment.rs` | `deployment_desired_defaults_to_one` |
| `deployment.rs` | `deployment_strategy_reads_surge_and_unavailable` |
| `deployment.rs` | `deployment_revision_reads_annotation` |
| `stateful_set.rs` | `stateful_set_summary_reads_service_and_claim_templates` |
| `daemon_set.rs` | `daemon_set_summary_reads_scheduling_counts` |
| `daemon_set.rs` | `daemon_set_node_selector_terms` |
| `replica_set.rs` | `replica_set_summary_reads_owner_and_counts` |
| `job_tests.rs` | `job_status_follows_condition_order` (Failed > Complete > FailureTarget > Suspended > Running) |
| `job_tests.rs` | `job_finished_at_falls_back_to_failed_condition_time` |
| `cron_job.rs` | `cron_job_summary_reads_schedule_and_history` |
| `service.rs` | `service_port_display_matches_kubectl` (`80/TCP`, `80:30080/TCP`) |
| `service.rs` | `headless_service_has_no_cluster_ips` |
| `service.rs` | `load_balancer_addresses_precede_external_ips` |
| `service.rs` | `external_name_service_reports_its_name` |
| `ingress.rs` | `ingress_class_prefers_spec_over_annotation` |
| `ingress.rs` | `ingress_paths_flatten_host_path_backend` |
| `ingress.rs` | `ingress_hosts_are_deduplicated_in_order` |
| `config_map.rs` | `config_map_keys_merge_data_and_binary_sorted` |
| `config_map.rs` | `config_map_summary_drops_values`: `format!("{summary:?}")` does not contain a distinctive value |
| `access_review.rs` | `all_checks_cover_distinct_permissions` (19, replaces the nine-check test) |
| `access_review.rs` | `kind_checks_use_their_api_group` (apps, batch, networking.k8s.io, and core) |
| `access_review.rs` | `namespace_check_is_cluster_scoped` |
| `access_review.rs` | `check_display_matches_kubectl_wording` (all 19 texts) |

## `crates/app`

| S | File | Test |
|---|---|---|
| 2 | `resource_kind.rs` | `kind_labels_match_navigation_items` (every `label()` is in `SECTIONS`) |
| 2 | `resource_kind.rs` | `plural_slugs_round_trip` |
| 2 | `resource_kind.rs` | `only_namespaces_is_cluster_scoped` |
| 2 | `resource_kind.rs` | `rows_maps_snapshot_and_keeps_failure` |
| 3 | `resource_kind.rs` | `port_forward_kinds_are_deployments_stateful_sets_services` |
| 2 | `kind_row.rs` | `owns_pod_through_deployment_replica_set`: `api` matches `api-7d9f8c`; it does not match `api-worker-7d9f8c` or `api-canary` (outside the hash alphabet) |
| 2 | `kind_row.rs` | `owns_pod_requires_same_namespace` |
| 3 | `kind_row.rs` | `owns_pod_by_controller_kind_and_name` |
| 2 | `namespace_rows.rs` | `namespace_row_cells_match_column_count` |
| 2 | `workload_rows_tests.rs` | `workload_row_cells_match_column_count`, one fixture per workload builder; step 3 extends it to all 6. This is the invariant the table relies on |
| 2 | `workload_rows_tests.rs` | `replica_tone_by_ready_and_desired` |
| 2 | `workload_rows_tests.rs` | `deployment_status_prefers_deadline_then_pause` |
| 3 | `workload_rows_tests.rs` | `job_completions_follow_kubectl` |
| 3 | `workload_rows_tests.rs` | `cron_last_run_outcomes` (all four cases) |
| 3 | `workload_rows_tests.rs` | `stateful_set_pods_sort_by_ordinal` (`web-10` after `web-2`) |
| 3 | `workload_rows_tests.rs` | `owner_cells_use_lowercase_kind` (`deployment/api`) |
| 3 | `network_rows.rs` | `network_row_cells_match_column_count` |
| 3 | `network_rows.rs` | `service_row_shows_none_for_headless_and_pending_for_load_balancer` |
| 3 | `network_rows.rs` | `ingress_row_hosts_star_when_empty_and_tls_ports` |
| 3 | `config_map_rows.rs` | `config_map_row_cells_match_column_count` |
| 3 | `config_map_rows.rs` | `format_bytes_uses_binary_units` |
| 2 | `navigation.rs` | `enabled_items_are_pods_nodes_and_explorer_kinds` (replaces `only_pods_and_nodes_are_enabled`; step 3 updates the expected list) |
| 2 | `navigation.rs` | `denied_kind_reason_names_all_namespaces_scope` (All gives "… in all namespaces"; Named gives the short form) |
| 2 | `navigation.rs` | `checking_and_unknown_access_keep_kinds_enabled` |
| 2 | `kind_table.rs` | `empty_text_mentions_scope_only_for_namespaced_kinds` (pure `fn empty_text`) |
| 2 | `table_selection_tests.rs` | `kind_key_matches_row_by_kind_namespace_and_name` |
| 2 | `launch_options_tests.rs` | `kind_screens_parse_from_plural_slugs` |
| 2 | `screenshot.rs` | `kind_drawer_screen_needs_selection` |
| 2 | `workspace.rs` | `count_label_pluralizes` (adds `ingress`/`ingresses`) |

## Live checks (coder-lite)

| S | Check |
|---|---|
| 1 | Run `cargo run -p k8sboard-cluster --example probe -- --kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --watch-seconds 5`. Expect 12 `watch …` lines and 19 access lines. Record allowed or denied, and the item count, for each of the 10 kinds. **UAT RBAC for these kinds is unknown and must come from this output.** Then run the 0001 AC7 credential script; every count must be 0 |
| 2 | Run the app with `--screen namespaces`, `namespaces-drawer`, `deployments`, and `deployments-drawer` into `.tmp/ui-shots/`, applying the 0003 PNG checks |
| 3 | Run every remaining allowed kind's `<plural>` and `<plural>-drawer` screens |

## ui-verifier checklist (W7; step 2 for its two kinds, step 3 for all)

- Tables:
  - columns are in order, Name is flexible, and Age is right-aligned;
  - Ready cells are colored by replica tone;
  - only theme colors are used;
  - switching kinds starts at the top-left with fitted widths.
- A denied kind is greyed out with a lock. Its tooltip reads "Not permitted: list …", plus "in all namespaces" when the scope is All.
- Drawers:
  - header: badge, name, ⋯, and ✕;
  - a toned subtitle;
  - sections in the [kind-drawers.md](kind-drawers.md) order;
  - Forward buttons disabled, with a reason;
  - Pods rows live and clickable.
- The row menu and the ⋯ menu match. "View YAML" is disabled, mutating items read "Read-only mode", and Delete is last.
- Light and dark have no contrast defects.
