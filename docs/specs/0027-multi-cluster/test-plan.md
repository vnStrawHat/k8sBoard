# 0027 · Test plan

[Back to index](README.md). Unit tests are offline. Headless tests use the 0026 fixture (0026 test-plan.md: tokio runtime kept alive, `ClusterRuntime` global, `AppSettings` with writes off, `ClusterCatalog` over fixture kubeconfigs at `https://127.0.0.1:1`). Tests that need a **Live** slot build it with the `#[cfg(test)]` seams of [view-model.md](view-model.md): `ClusterConnection` from `runtime.block_on(ClusterConnection::open(..))` (no request), then `ClusterSession::live_fixture(..)`, then `seed_pods(..)`. Watches started by `live_fixture` fail against the dead port and never feed rows.

## Step 1 — `cluster_view_tests.rs` and headless

| Test | Checks |
|---|---|
| `plan_keeps_releases_and_connects` | {A,B} → {B,C}: keep B, release A, connect C |
| `plan_is_empty_for_the_same_set` | |
| `plan_orders_by_display_order` | wanted [C, A] → [A, C] |
| `plan_refuses_more_than_five` | six → `TooManyClusters` |
| `primary_stays_when_in_the_set` | current B, wanted {A, B} (A first in display order) → primary B |
| `primary_is_first_when_current_leaves` | current C, wanted {A, B} → primary A |
| `riskiest_is_the_max_environment` | DEV + PROD → PROD (border only) |
| `apply_keeps_the_staying_session` (headless, live_fixture) | B's entity id unchanged |
| `apply_releases_before_connecting` (headless) | at the deferred connect: A's weak handle cannot upgrade, and `slots + connects ≤ 5` |
| `single_switch_from_multi_releases_all` (headless) | Ctrl 1 → one slot |
| `fan_out_reaches_every_slot` (headless, live_fixture ×2) | `set_scope`, `set_explorer_kind`, `set_event_filter`, `set_explorer_paused` reach both |
| `slot_going_live_gets_the_view_scope` (headless) | |
| `last_used_follows_the_primary` (headless) | |
| `filters_kept_when_primary_stays` / `filters_reset_on_new_primary` | |
| `trigger_shows_plus_n_and_primary_label` / `border_uses_riskiest` | title-bar helpers |
| `status_bar_multi_line` | `Watching 7 resource types · 2 clusters` |
| `picker_lists_union_with_loading_lines` | ready slot names + `Loading namespaces of {label}…` |

## Step 2 — rows and keys

| Test | Checks |
|---|---|
| `toggle_tick_adds_and_removes` / `toggle_tick_refuses_a_sixth` | notice text |
| `ticks_start_as_viewed_set` | single → `[current]`; multi → the slots |
| `ticks_differ_as_sets` | order ignored |
| `query_ignores_whitespace` | `prod eu` matches |
| `footer_only_when_ticks_differ` (headless) | |
| `space_ticks_in_filter` (headless) | path `Popover > ClusterSwitcher > Input`; filter text unchanged; popover still open |
| `space_ticks_on_row` (headless) | path `Popover > ClusterSwitcher` |
| `enter_applies_when_ticks_differ` / `enter_switches_when_ticks_equal` (headless) | |
| `ticking_never_connects` (headless) | |

## Step 3a — `cluster_rows_tests.rs`, tables

`merge_keeps_slot_order_and_addresses`, `same_pod_in_two_clusters_is_two_rows`, `clustered_label_is_switcher_label`, `cluster_column_is_last_and_only_in_multi`, `cluster_column_sorts_by_label`, `equals_chip_filters_one_cluster`, `prefs_skip_the_cluster_column`, `leaving_multi_drops_cluster_hidden_and_sort`, `hidden_prefs_by_name_survive_multi`, `header_counts_all_clusters` (`2 clusters · 2,431 pods`, seeded pods), `loading_only_until_some_slot_is_ready`.

## Step 3b

`selection_is_cluster_qualified`, `checks_are_cluster_qualified`, `reveal_targets_the_cluster`, `sidebar_count_sums_known_slots`, `failed_slot_shows_banner_with_remove` (headless), `interrupted_slot_shows_banner` (headless, live_fixture + forced problem), `all_failed_shows_error_view_with_retry_all` (headless).

## Step 4 — wrong-cluster sites (headless, live_fixture ×2 with the same pod name)

`yaml_view_is_rebuilt_for_same_name_in_other_cluster`, `same_pod_in_two_clusters_opens_two_tabs`, `subject_moves_between_clusters_stops_old_starts_new`, `pending_subjects_differ_by_cluster`, `kubelet_demand_only_on_subject_slot` (other slot's demand is `KubeletDemand::default()`), `drawer_reads_the_row_cluster`, `drawer_closes_when_its_slot_is_released`, `menu_uses_row_cluster_context`, `log_tab_title_has_cluster_in_multi`, `released_slot_closes_its_log_tabs`, `screen_pods_multi_parses`, `pods_multi_settles_when_slots_are_live_or_failed`.

## Live checks (coder-lite, read-only)

`--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0027`, plus a seeded unreachable fixture in `registry.kubeconfigs`.

1. Tick UAT + fixture, View 2 clusters: UAT rows with Cluster column, banner for the fixture with Retry and Remove from view.
2. Remove from view → single mode, UAT session id unchanged (test hook log), Cluster column gone.
3. Budget runs of [budget.md](budget.md); fill "Results". Never print the kubeconfig; no request-log capture.

## ui-verifier (screenshot build; writes are off)

| Screen | Seed | Expect (W1) |
|---|---|---|
| `pods-multi`, light and dark | `--view readonly@Monitor,<fixture ctx>`; fixture `environment: production` | trigger `{fixture label} +1` (no current cluster at launch, so the first in display order, the PROD fixture, is primary), red top border, Cluster column last with badges, header `2 clusters · {n} pods`, fixture banner |
| `switcher` with ticks | same | checkboxes; no footer (ticks equal the view) |
