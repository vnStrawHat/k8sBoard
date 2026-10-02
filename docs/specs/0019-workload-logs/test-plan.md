# 0019 · Test plan

[Back to index](README.md). Pure logic only: no network, no cluster, no file system, no GPUI windows. Layout is the ui-verifier's job.

## Step 1

| File | Test | Verifies |
|---|---|---|
| `log_level.rs` | `json_level_field_is_detected` | `{"level":"ERROR","msg":"x"}` → Error; `severity`, `lvl`, `log.level` too |
| | `json_numeric_level_maps_pino_scale` | 10, 20 → Debug; 30 → Info; 40 → Warn; 50, 60 → Error; 35 and `true` → `None` |
| | `logfmt_level_is_detected` | `ts=1 level=warn msg=x` → Warn; `lvl="info"` → Info |
| | `klog_prefix_is_detected` | `E0501 10:47:58.902 …` → Error; `I0501 …` → Info; `F…` → Error |
| | `uppercase_keyword_in_head_is_detected` | `2024 ERROR [main] boom` → Error |
| | `lowercase_word_needs_brackets_or_tabs` | `[error] x` → Error; `\tinfo\tx` → Info; `no error here` → `None` |
| | `underscore_joined_word_is_not_a_level` | `ERROR_COUNT=3`, `no_error` → `None` |
| | `keyword_after_64_bytes_is_ignored` | |
| | `level_set_toggles_and_reports_hidden` | |
| `line_matcher_tests.rs` | `plain_matcher_matches_ignoring_ascii_case` | |
| | `regex_matcher_matches_alternation_case_insensitively` | `error\|timeout` vs `Upstream TIMEOUT` |
| | `invalid_regex_is_an_error` | `a\|(` → `Err(InvalidRegex)` |
| | `blank_text_parses_to_no_matcher` | both modes |
| | `regex_ranges_skip_empty_matches` | `x?` on `abc` → `is_match` true, no ranges |
| | `oversized_regex_is_rejected` | a pattern above the 1 MiB compiled limit (e.g. `\w{1000}{1000}`) → `Err` |
| | the 0004 `find_matches_*` tests | moved unchanged |
| `log_json.rs` | `json_line_splits_message_and_details` | `msg` headline; `level`, `time` dropped; remaining keys pretty, sorted |
| | `json_line_without_message_has_no_headline` | |
| | `json_line_with_only_a_message_has_no_details` | split from the test above |
| | `non_object_text_is_not_json` | `[1,2]`, `{broken`, plain text → `None` |
| `log_buffer_tests.rs` | 0004 tests adapted | same names, `SourcedLine` input, `set_view` |
| | `push_detects_and_stores_levels` | |
| | `indented_line_inherits_level_of_same_source` | source 0 error, source 1 info, source 0 `\tat x` → Error |
| | `hidden_level_hides_lines` | INFO hidden hides Info and unknown-level lines (decision 15) |
| | `view_combines_levels_and_matcher` | |
| | `change_counts_match_visible_len_with_levels_hidden` | the 0004 invariant |
| | `visible_text_writes_prefixes_and_clock_time` | `LineTime::Clock` and `Hidden`, short prefixes by `SourceId`, missing prefix → none (the Rfc3339 variant moves to step 3) |

## Step 2a

| File | Test | Verifies |
|---|---|---|
| `pod_log_tests.rs` (cluster) | `log_params_uses_requested_tail` | `tail_lines: 50` → `Some(50)`; existing `log_params_*` pass 1000 |
| `log_target.rs` | `workload_label_uses_kubectl_short_kinds` | deploy, sts, ds, rs, job; node → `None` |
| | `log_target_of_workload_rejects_nodes` | pure, no session |
| | `same_target_matches_pods_by_name_and_workloads_by_owner` | |
| `log_workload.rs` | `ranked_pods_put_ready_first_then_newest` | ties broken by name; `None` created last |
| | `ranked_pods_keep_only_owned_pods` | Deployment through its ReplicaSets |
| | `member_change_fills_free_slots_in_rank_order` | |
| | `member_change_keeps_listed_members_beyond_limit` | sticky |
| | `member_change_reports_unlisted_members_as_left` | |
| | `member_change_offers_returning_name_as_join` | a name absent from `current` but ranked joins |
| | `join_slots_counts_every_live_stream` | 18 live, 2 per pod → 1; 20 live → 0; pod limit reached → 0 |
| | `pod_limit_divides_streams_by_containers` | 1 → 10, 2 → 10, 3 → 6, 30 → 1, 0 → 10 |
| | `pod_short_name_is_last_segment_except_stateful_sets` | `api-7d9f8c-x2k4q` → `x2k4q`; `postgres-0` stays |
| | `scope_covers_named_several_and_all` | |
| `log_tab.rs` | `workload_tone_follows_stream_states` | table in [workload-streams.md](workload-streams.md) |
| | `staged_lines_sort_by_timestamp_stably` | pure `sort_staged`; equal times keep order; `None` first |
| | `slot_for_reuses_slot_of_returning_pod` | pure `fn slot_for(pod_slots: &mut Vec<String>, pod: &str) -> usize` |
| | `first_sync_selects_default_container_of_first_ranked_pod` | pure `plan_membership` |
| | `plan_admits_only_selected_containers_the_pod_has` | |
| | `plan_without_pods_selects_nothing_yet` | |
| | `plan_is_frozen_when_scope_excludes_the_workload` | |
| | `merge_window_opens_with_the_first_admission_only` | staging is armed lazily, on the first admission |
| `resource_actions_tests.rs` | `workload_menu_offers_view_logs_when_allowed` | Deployment row: first item `View logs (all pods)` |
| | `job_menu_labels_view_logs` | |
| | `workload_view_logs_denied_reason_names_access_check` | |
| | `non_workload_kind_menu_has_no_view_logs` | ConfigMap row |

## Step 2b

| File | Test | Verifies |
|---|---|---|
| `log_workload.rs` | `container_names_union_in_first_seen_order_init_last` | |
| `log_tab.rs` | `toggled_selection_keeps_offer_order_and_the_last_container` | the picker toggle keeps the offer order; the last checked container stays |
| `launch_options_tests.rs` | `parses_logs_workload_screen` | |
| `screenshot.rs` | `controller_owner_of_maps_known_kinds` | ReplicaSet, StatefulSet, DaemonSet, Job; others `None` |

## Step 3

Moved here from steps 1, 2a, and 2b (they need code that lands in step 3): `no_log_target_displays_reason` (`NoLogTarget`, `selected_log_target`), `revision_bumps_on_push_clear_and_view` and `visible_text_writes_prefixes_and_rfc3339_time` (`LogBuffer::revision`, `LineTime::Rfc3339`), and `log_target_of_container_is_explicit` (`ContainerChoice`, `LogTarget::of_container`, `TabStream.full_prefix`).

| File | Test | Verifies |
|---|---|---|
| `log_volume.rs` | `bucket_width_picks_smallest_fitting_width` | 50 s → 1 s; 10 min → 15 s; 3 h → 300 s; 90 d → 1 d |
| | `volume_counts_lines_and_errors_per_aligned_bucket` | |
| | `volume_fills_empty_buckets_between_first_and_last` | |
| | `volume_keeps_newest_buckets_beyond_limit` | at most 60 |
| | `volume_without_two_timestamps_is_none` | |
| | `bucket_label_formats_by_width` | |
| `log_export.rs` | `export_file_name_sanitizes_label_and_stamps_utc` | `deploy/api` + 2024-05-01T10:47:58Z → `deploy_api-20240501-104758Z.log` |
| | `export_state_is_busy_while_choosing_or_saving` | |
| `log_volume.rs` | `width_label_names_the_unit` | the caption unit |
| `log_dock.rs` | `move_tab_keeps_active_tab_active` | forward and backward moves |
| | `move_tab_ignores_same_or_out_of_range_index` | |
| `container_detail_tests.rs` | `container_tabs_put_logs_before_monitor` | `CONTAINER_TABS` order |

## Live checks (coder-lite)

1. `grep -c '^\[\[package\]\]' Cargo.lock` equals the pre-step count; `cargo tree -p k8sboard -i regex --depth 0` shows 1.13.x.
2. Scoped grep for `tracing` over exactly `crates/app/src/{log_buffer,log_tab,log_dock,log_level,line_matcher,log_json,log_rows,log_target,log_workload,log_volume,log_export}.rs` → report each hit; none may take line text, `path`, `file_name`, or export text. (`log_filter.rs` is the pre-existing tracing pin and is out of scope.) `std::fs::` in `crates/app/src` → only `log_export.rs` (plus pre-existing hits, listed).
3. `git diff --stat -- crates/cluster` lists only `pod_log.rs`, `pod_log_tests.rs`, `examples/probe.rs`; the 0004 probe `--logs-seconds 5` still reports `started` and 0 failures.
4. App on UAT with `--context readonly@Monitor`: open Deployments → a deployment with ≥ 2 pods → "View logs (all pods)"; report pods joined, prefix colors, and whether lines interleave. Counts only, never log text.

## ui-verifier checklist

1. Capture `logs-dock`, `logs-zoomed`, `logs-workload`, `pod-containers` in light and dark (`.tmp/ui-shots/0019-<screen>-<theme>.png`).
2. W8: toolbar shows filter + Regex toggle, ERROR/WARN/INFO/DEBUG chips, Previous, JSON, Timestamps, Wrap, Copy, Export, status; "+ ▾" after the tabs; no histogram in Normal.
3. W8b (`logs-workload`): tab label `rs/…` or `deploy/…`; legend chips with color dots; histogram strip under the toolbar; prefix column colored per pod; title bar, sidebar, status bar visible.
4. W4b: sub-tabs read `Info · Env n · Mounts n · Logs · Monitor`.
5. Accepted deviations (do not report): no per-row level column (the text carries it; only JSON mode shows a level tag); no Follow toggle (tail-follow + "Jump to latest" is follow); level chips are ERROR/WARN/INFO/DEBUG in both layouts (W8 shows three); legend and histogram only when zoomed.
6. Not capturable headlessly (user spot-check on UAT): regex typing and `Invalid regex`, level chips, JSON blocks, Export dialog and file, drag reorder, "+ ▾" items, Logs sub-tab focusing the dock.
