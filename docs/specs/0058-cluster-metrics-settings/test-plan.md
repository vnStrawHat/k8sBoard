# 0058 · Test plan

[Back to index](README.md). All offline: `FakeApi` connections, or a kubeconfig whose server is `https://127.0.0.1:1` (refused at once, as in `clusters_page_tests.rs`). No test reaches a cluster.

## `cluster_metrics_section_tests.rs` (moved from `metrics_page_tests.rs`)

Helper change: `open_page` → `open_section(row: Option<ClusterRow>)`: a window whose root view is the section over a test catalog; it calls `show_cluster` and draws once. `cluster_ref()` / `other_cluster_ref()` stay; `row_of(cluster)` builds the `ClusterRow`.

| Test | Status | Checks |
|---|---|---|
| `other_service_validates_each_field` | moved as is | pure |
| `saved_source_is_preselected` | moved as is | pure |
| `test_lines_per_outcome` | moved as is | pure |
| `saved_line_names_the_source_and_its_state` | moved as is | pure |
| `metrics_page_without_cluster_shows_the_hint` | **renamed** `section_without_cluster_is_empty_and_idle` | `cluster` `None`, `Idle`, not settled (the hint text is gone) |
| `detection_lists_the_candidates_of_the_connected_cluster` | **renamed** `open_cluster_detects_at_first_draw` | same asserts; the label is the row label |
| `saved_candidate_is_preselected_after_detection` | moved | — |
| `save_writes_fields_only_and_server_only_clears_them` | moved | — |
| `detect_again_keeps_the_picked_row_and_save_writes_it` | moved | — |
| `a_picked_row_that_detection_drops_still_saves_what_was_picked` | moved | — |
| `invalid_other_service_cannot_be_saved_or_tested` | moved | — |
| `test_checks_the_selected_source` | moved | — |
| `switching_clusters_clears_inputs_and_drops_a_running_detection` | **reworked** `show_cluster_resets_inputs_and_drops_a_running_detection` | first cluster open on a hung list, with a saved source (inputs filled); publish the answering fake for `other`, `show_cluster(other row)` → new list, `ServerOnly`, empty inputs |
| `fixture_page_is_settled_and_sends_nothing` | **renamed** `fixture_section_is_settled_and_sends_nothing` | `ClusterMetricsFixture` + `show_cluster(Some(row))`; 3 candidates; label from the row |
| `cluster_that_is_not_open_sends_nothing_until_detect` | new | `ActiveConnection` is another cluster (fake API); two draws; detection `Idle`, the fake has 0 requests; Save of a valid `Other service` still writes the entry |
| `detect_on_a_cluster_that_is_not_open_opens_a_one_time_client` | new | catalog row on `https://127.0.0.1:1`; `start_detection` → `Detection::Failed(_)` from the connect error; the open cluster's fake has 0 requests |
| `test_on_a_cluster_that_is_not_open_reports_cannot_connect` | new | same row, valid `Other service`, `start_test` → `TestResult::Done(Err(MetricsError::Unexpected(m)))`, `m` starts with `cannot connect: ` |
| `show_cluster_again_for_the_same_row_resets_the_choice` | new | the Reset to defaults path: choose `Other` and type, clear the entry, `show_cluster(same row)` → `ServerOnly`, empty inputs, `Idle` |
| `section_follows_the_cluster_becoming_open` | new | not open → `Idle`; publish `ActiveConnection` for it → the next draw starts detection (one `/api/v1/services`) |

## `clusters_page_tests.rs`

| Test | Status | Checks |
|---|---|---|
| `source_button_names_metrics_server_or_the_saved_service`, `form_dropdown_clears_the_source` | deleted | the dropdown is gone |
| `selecting_a_row_retargets_the_metrics_section` | new | `two_cluster_setup`; `metrics_section().read(cx).cluster()` is `prod-a`; select `prod-b`, draw → `prod-b`. Reset to defaults reaches `show_cluster` through the same `sync_form` path; the reset itself is covered by `show_cluster_again_for_the_same_row_resets_the_choice` |

## `settings_window_tests.rs`

| Test | Status | Checks |
|---|---|---|
| `pages_follow_w2_order` | changed | nine titles, no `Metrics` |
| `default_page_is_clusters` | changed | `PAGES.len() == 9`, `About.index() == 8` |
| `a_window_on_another_page_lists_no_service_until_the_metrics_page_shows` | **replaced** by `cluster_metrics_redirect_detects_only_when_shown` | a chain file with two contexts (`install(None, &[file], cx)`); `ActiveConnection` = the second row, on the fake (a selection the catalog does not list is dropped by `resolve_selection`); open on General, two draws → 0 requests; `show_cluster_metrics` → Clusters page, `first_group == Some(CLUSTER_METRICS_GROUP)`, `clusters.read(cx).metrics_section().read(cx).cluster()` is the open cluster, one `/api/v1/services` |
| `show_page_clears_the_metrics_group` | new | after `show_cluster_metrics`, `show_page(General)` → `first_group` `None` |

## Others

| File | Test | Change |
|---|---|---|
| `launch_options_tests.rs` | `screen_settings_metrics_parses` → `screen_settings_cluster_metrics_fixture_parses` | parses; `settings_screen() == Some((Clusters, Standard))`; `opens_cluster_metrics()`; `Screen::Overview`; `settings-metrics` no longer parses |
| `screenshot.rs` | `metrics_settings_screen_waits_for_its_page` → `cluster_metrics_screen_waits_for_its_section` | field rename; the one screen |
| `topology_view_tests.rs`, `cluster_metrics_tests.rs` | existing text asserts | new text |

## Checks after the gate

- Grep: `SettingsPage::Metrics`, `metrics_page`, `show_metrics_page`, `settings-metrics`, and `Settings › Metrics` / `Settings \u{203a} Metrics` have no match under `crates/`.
- ui-verifier (light, dark): `--screen settings-cluster-metrics-fixture --kubeconfig monitor-uat-readonly.yml --context readonly@Monitor` (section in view, fixed rows, Test line, nothing clipped in `Other service`) and `--screen settings` (form without the Metrics row, the section below). The W2 nav still draws `Metrics`; 0058 supersedes it on user request.
- Live UAT (read-only): `--screen settings` with a `--script` that clicks the `readonly@Monitor` row, scrolls down, `wait 3s`, `shot`: the section lists `vmselect-…` then `vmquery-…` (0048 AC 7); the trace shows only `GET /api/v1/services` from the section.
