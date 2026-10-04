# 0048 · Test plan

[Back to index](README.md). Offline and deterministic, one behaviour per test. Requests go through `fake_api` (cluster crate) with recorded method, path, and query; GPUI tests use `WriteMode::Disabled` settings and fake clocks. No test opens a cluster connection.

## Step 1 (cluster, pure)

| Test | File | Checks |
|---|---|---|
| `string_literal_escapes_quote_backslash_newline` | `promql_tests.rs` | `a"b\c⏎` → `"a\"b\\c\n"`; control chars dropped |
| `regex_literal_escapes_metacharacters` | `promql_tests.rs` | `api.v1` → `"api\\.v1"`; every RE2 metachar of the list |
| `pod_cpu_query_text` | `promql_tests.rs` | golden text for Pod, whole pod |
| `container_memory_query_text` | `promql_tests.rs` | `container="api"` filter |
| `workload_patterns_per_kind` | `promql_tests.rs` | the five patterns over the suffix alphabet `S`, name escaped |
| `deployment_pattern_skips_cronjob_pods` | `promql_tests.rs` | pattern text check: `api-28312790-xb7zq` has `0 1 3 8` outside `S`; `api-7d9f8c-x2k4q` fits (a hand-written matcher over the two classes, no regex dependency) |
| `node_network_uses_the_virtual_interface_filter` | `promql_tests.rs` | `V` filter on Node, `interface!="lo"` on pods |
| `network_ignores_the_container_filter` | `promql_tests.rs` | container target still selects the pod |
| `rate_window_is_at_least_two_minutes` | `promql_tests.rs` | 15 s and 1 min steps → `[120s]`; 2 h → `[7200s]` |
| `invalid_names_build_no_query` | `promql_tests.rs` | `pod="a\"b"`, uppercase namespace → `InvalidName` |
| `range_spec_limits` | `promql_tests.rs` | step < 1 s, end ≤ start, 401 points → `Err`; 400 → `Ok` |
| `step_table_stays_under_the_point_cap` | `promql_tests.rs` | every range of the table ≤ 400 points |
| `source_validation_per_field` | `metrics_source_tests.rs` | each row of the field table, ok and error |
| `prefix_rejects_dot_segments_and_query_chars` | `metrics_source_tests.rs` | `/a/../b`, `/a?x`, `/a#`, `/a%2f`, trailing `/`, no leading `/` (`select/0`), empty segment (`/a//b`) |
| `source_fields_round_trip_json` | `metrics_source_tests.rs` | JSON shape of settings-page.md; `scheme` lowercase; missing `prefix` = empty |
| `source_debug_is_display` | `metrics_source_tests.rs` | `Debug` prints `display()` only; `MetricsSource` is built only through `new` (no `Deserialize`, no `Default`) |
| `candidates_rank_vmselect_before_vmquery` | `metrics_source_tests.rs` | UAT services fixture → vmselect, vmquery; vminsert, vmstorage, grafana, node-exporter skipped |
| `candidates_per_flavor` | `metrics_source_tests.rs` | one fixture per table row: prefix, port, scheme |
| `candidate_port_prefers_names_then_number` | `metrics_source_tests.rs` | named `http` on 8481; fallback number; none → skipped |
| `candidates_cap_at_twenty` | `metrics_source_tests.rs` | 25 matches → 20 |

## Step 2 (cluster transport)

| Test | File | Checks |
|---|---|---|
| `query_range_path_and_query` | `metrics_query_tests.rs` | GET, path `/api/v1/namespaces/monitoring/services/http:vmselect-x:8481/proxy/select/0/prometheus/api/v1/query_range`, `start`/`end`/`step`/`timeout=15s`, percent-encoded query |
| `endpoints_are_fixed` | `metrics_query_tests.rs` | the three endpoint strings; nothing else reachable |
| `check_counts_cpu_series` | `metrics_query_tests.rs` | vector `[{"value":[t,"1234"]}]` → `cpu_series` 1234; empty vector → 0 |
| `matrix_decodes_with_gaps` | `metrics_query_tests.rs` | missing steps and `NaN` → `None`; aligned to `start + k × step` |
| `samples_snap_to_the_nearest_step` | `metrics_query_tests.rs` | `t` = `start + 2 × step + 0.4 s` → index 2; outside the range → dropped |
| `matrix_over_series_limit_is_cut` | `metrics_query_tests.rs` | 65 series → 64 summed, `was_cut` |
| `body_over_limit_is_too_large` | `metrics_query_tests.rs` | 4 MiB + 1 on a 200 and on a 500 → `TooLarge` |
| `slow_body_times_out` | `metrics_query_tests.rs` | paused tokio clock, body never ends → `TimedOut` |
| `forbidden_status_is_denied` | `metrics_query_tests.rs` | 403 `Status` → `Denied` |
| `missing_service_and_no_endpoints_are_unreachable` | `metrics_query_tests.rs` | 404, 503 `Status` → `Unreachable` |
| `backend_404_without_status_is_no_api_at_prefix` | `metrics_query_tests.rs` | 404 `404 page not found` → `NoApiAtPrefix` |
| `other_code_is_unexpected_with_the_code` | `metrics_query_tests.rs` | 502 HTML body → `Unexpected("HTTP 502")`, body not echoed |
| `backend_error_json_is_rejected` | `metrics_query_tests.rs` | 400 `{"status":"error","errorType":"bad_data","error":"parse error"}` → `Rejected("parse error")` |
| `success_with_error_status_is_rejected` | `metrics_query_tests.rs` | 200 `status: error` → `Rejected` |
| `error_text_is_cut_to_one_line` | `metrics_query_tests.rs` | 1,000-char multi-line message → 200 chars, no newline |
| `list_all_services_lists_every_namespace` | `service` tests | path `/api/v1/services`, pages followed |

Live (coder-lite, UAT, read-only): `probe --metrics-source monitoring/vmselect-vm-victoria-metrics-k8s-stack:8481/select/0/prometheus` → check line, 30d query line, and the three facts of the promql.md table (root-cgroup `node` label or the node-exporter label, `container_fs_*`, pod network within ±20 % of the 0011 kubelet summary), recorded in as-built. Trace shows only GETs to `…/services/http:vmselect-…:8481/proxy/…`.

## Steps 3a, 3b (app settings; `settings_keys…`, `invalid_metrics_entry…`, `reset_clears…`, `settings_change…`, `source_note_texts` are 3a)

| Test | File | Checks |
|---|---|---|
| `settings_keys_are_the_allow_list` (extended) | `settings_tests.rs` | the six `registry.clusters.metrics*` keys |
| `invalid_metrics_entry_fails_closed` | `cluster_registry_tests.rs` | bad prefix → `Some(Err(Prefix))`; state `Invalid`, no check started |
| `reset_clears_metrics` | `cluster_registry_tests.rs` | Reset to defaults drops the entry |
| `pages_follow_w2_order` (updated) | `settings_window_tests.rs` | Metrics between Logs and About |
| `metrics_page_without_cluster_shows_the_hint` | `metrics_page_tests.rs` | no `ActiveConnection` → hint only |
| `saved_source_is_preselected` | `metrics_page_tests.rs` | not detected → Other service with its fields |
| `other_service_validates_each_field` | `metrics_page_tests.rs` | messages; Test and Save disabled while invalid |
| `save_writes_fields_only` | `metrics_page_tests.rs` | stored JSON equals the fields; `metrics-server only` stores `None` |
| `settings_change_restarts_the_source_check` | `app_shell_tests.rs` | new entry → `Checking`; same entry → unchanged |
| `source_note_texts` | `cluster_metrics_tests.rs` | one line per state |
| `form_dropdown_clears_the_source` | `clusters_page_tests.rs` | `metrics-server only` → `None` |

## Step 4 (app Monitor)

| Test | File | Checks |
|---|---|---|
| `ranges_follow_the_source_state` | `monitor_source_tests.rs` | Ready + target → 6; otherwise 4 |
| `source_target_per_subject_and_scope` | `monitor_source_tests.rs` | the mapping table of promql.md |
| `host_network_pod_shows_the_network_notice` | `monitor_source_tests.rs` | pod and workload with one host-network pod → 0011 notice on Network, other charts from the source |
| `empty_cpu_source_offers_no_long_ranges` | `monitor_source_tests.rs` | `cpu_series == 0` → `SAMPLER` |
| `long_ranges_skip_the_sampler` | `monitor_source_tests.rs` | 30d builds from the result only; `monitor_data` is not called |
| `range_click_indexes_the_shown_set` | `monitor_source_tests.rs` | index 5 of `SOURCE` → `Days30` |
| `refresh_after_per_range` | `monitor_source_tests.rs` | 30 s / 5 min |
| `same_key_fresh_result_does_not_refetch` | `monitor_source_tests.rs` | fake clock; stale → refetch |
| `key_change_drops_the_running_fetch` | `monitor_source_tests.rs` | old task dropped |
| `hidden_monitor_drops_the_fetch` | `app_shell_tests.rs` | another drawer tab → `source_fetch` is `None`, no timer |
| `empty_cpu_falls_back_on_short_ranges` | `monitor_source_tests.rs` | 24h → `Fallback`; 30d → Alert reason |
| `failed_network_keeps_other_charts` | `monitor_source_tests.rs` | notice on Network only |
| `source_rows_newest_first_with_oom` | `monitor_source_tests.rs` | offsets, OOM row within half a step |
| `leaving_ready_resets_long_ranges` | `app_shell_tests.rs` | 30d → 24h |

## Screens and ui-verifier

`settings-metrics-fixture`, `pod-monitor-source-fixture` (light, dark); live UAT `settings-metrics` and `pod-monitor` with `--config-dir .tmp/cfg-0048` holding the vmselect entry. Checklist: W2 nav order and cluster form `Metrics` section; W4c range group, footer note, four cards, legend, no clipped text at 28 px density.
