# 0014 · Test plan

[Back to index](README.md). Unit tests are offline and deterministic; fixtures are k8s-openapi structs with `..Default::default()`. Names are binding (AC 2).

## Step 1 — `crates/cluster`

| Module | Tests |
|---|---|
| `persistent_volume_claim.rs` | `pvc_summary_reads_status_and_spec`, `pvc_phase_defaults_to_pending`, `pvc_access_modes_prefer_status`, `pvc_terminating_from_deletion_timestamp` |
| `persistent_volume.rs` | `pv_summary_reads_claim_class_and_reclaim`, `pv_reclaim_defaults_to_retain`, `csi_backend_keeps_driver_handle_fs_type`, `csi_attributes_and_secret_refs_are_not_copied` (`format!("{summary:?}")` lacks a distinctive attribute value and secret name), `nfs_and_host_path_backends`, `in_tree_source_reports_field_name`, `node_affinity_terms_in_selector_syntax` (with `matchFields`), `pv_keeps_mount_options` |
| `storage_class.rs` | `storage_class_defaults_delete_and_immediate`, `default_class_from_either_annotation`, `other_annotations_are_ignored`, `secret_like_parameters_are_hidden` (`restuserkey`, `adminPassword`, `access-key`, `token` hidden), `harmless_parameters_are_shown` (`kmsKeyId`, `type`, `csi.storage.k8s.io/provisioner-secret-name`, `csi.storage.k8s.io/node-stage-secret-namespace` shown), `parameters_in_key_order` |
| `object_yaml_tests.rs` | `storage_class_parameters_are_masked_in_yaml`, `storage_kinds_have_names_and_scope` (PV and StorageClass cluster-scoped) |
| `access_review.rs` | `all_checks_cover_distinct_permissions` (new length), `storage_checks_use_their_api_groups_and_scope`, `check_display_matches_kubectl_wording` |

## Step 2 — PVCs, PVs

| Module | Tests |
|---|---|
| `storage_rows_tests.rs` | `pvc_row_cells_match_column_count`, `pv_row_cells_match_column_count`, `phase_label_tones`, `capacity_falls_back_to_requested`, `access_modes_abbreviate`, `resizing_and_resize_pending_status`, `pvc_volume_is_a_link`, `pv_claim_cell_is_qualified`, `pv_source_rows_per_backend` |
| `kind_diagnosis_tests.rs` | `pvc_volume_lost`, `pv_released_retain`, `pv_released_delete`, `pv_reclaim_failed`, `bound_pv_has_no_box` |
| `kind_join_tests.rs` | `joined_column_indices_name_their_columns` (adds `CLAIM_USED`), `claim_used_percent_and_tone` (83 % Warn, 95 % Bad), `claim_without_stats_is_absent`, `zero_capacity_is_absent`, `full_claim_raises_status` |
| `live_sections_tests.rs` | `claim_pods_dedupes_and_sorts`, `claim_pods_ignore_other_namespaces`, `claim_usage_bars_and_inodes`, `claim_usage_note_when_not_bound`, `block_claim_reads_no_usage` |
| `kubelet_metrics_tests.rs` | `claim_subject_nodes_are_mounting_pods_nodes` |
| `cluster_session_tests.rs` | `kubelet_round_rejoins_pvc_rows`, `scope_change_keeps_cluster_scoped_explorer` |
| `resource_actions_tests.rs` | `pvc_menu_go_to_first_mounting_pod`, `pvc_go_to_pod_disabled_when_unmounted`, `pv_menu_go_to_claim` |
| `resource_kind.rs` | `cluster_scoped_kinds_are_listed` (replaces `only_namespaces_is_cluster_scoped`) |
| `navigation.rs` | enabled list adds PVCs, PVs |

## Step 3 — StorageClasses, companion

| Module | Tests |
|---|---|
| `storage_rows_tests.rs` | `storage_class_row_cells_match_column_count`, `pvc_and_pv_class_link_to_storage_class` (a `Text` in step 2, when the kind has no screen yet), `default_class_star_and_status`, `hidden_parameter_reads_hidden` |
| `kind_join_tests.rs` | `class_volumes_count_by_class`, `class_volumes_absent_without_companion` |
| `live_sections_tests.rs` | `class_volumes_counts_bound_and_released` |
| `cluster_session_tests.rs` | `companion_plan_per_kind` (Services, StorageClasses, none, denied), `companion_list_ignores_other_variant`, `open_watch_count_counts_several_related_and_companion` (cluster-scoped companion counts 1) |
| `navigation.rs` | enabled list adds StorageClasses |

## Live checks (coder-lite, UAT `readonly@Monitor`)

1. Step 1: `probe --watch-seconds 5 --counts`; fill [decisions.md](decisions.md) "UAT probe"; AC7 credential script reports 0. Also `probe --kubelet-seconds 20` (0011) and note how many PVCs report usage.
2. Step 2: `persistentvolumeclaims`, `persistentvolumeclaims-drawer` (`--filter` a Bound, mounted claim), `persistentvolumes`, `persistentvolumes-drawer`. AC 6 spot check (exec is not allowed): compare the Used % with `kubectl get --raw /api/v1/nodes/{node}/proxy/stats/summary` volume numbers (`get nodes/proxy` is allowed). Never print kubeconfig data.
3. Step 3: `storageclasses`, `storageclasses-drawer` (default class).

## ui-verifier (steps 2–3)

Screens above in light, `persistentvolumeclaims-drawer` in dark. Check against W7: column order; Status and Used tones; Usage bars (Used, Inodes); Mounted by rows clickable; PV Source and Claim link; RELEASED box when a Released PV exists; StorageClass ★ and PVs column; parameters show `hidden` where masked; disabled Expand / Set as default with "Read-only mode"; Delete last; badges `Pc`, `Pv`, `Sc`; no hardcoded colors.
