# 0017 · Test plan

[Back to index](README.md). Unit tests are offline and deterministic; names are binding (AC 2). Fixture helper (cluster tests only): `release_secret(ns, name, rev, status, json)` builds a `Secret` with Helm labels, type `helm.sh/release.v1`, and `data.release` = base64(gzip(json)). Fixture JSON uses the distinctive texts `config-secret-value`, `manifest-secret-value`, `notes-secret-text`.

## Step 1 · `helm_release_tests.rs`

| Test | Verifies |
|---|---|
| `release_json_decodes_base64_gzip` | the double encoding round-trips |
| `release_json_accepts_plain_json` | decision 10 |
| `release_json_rejects_bad_base64_and_bad_gzip` | `NotBase64`, `NotGzip` |
| `release_json_stops_at_size_limit` | `release_json` calls private `release_json_within(bytes, limit)`; with limit 1024, a 1025-byte payload → `TooLarge`, 1024 → Ok |
| `gzip_capacity_reads_isize_capped_at_limit` | private `gzip_output_capacity(bytes, limit)`: ISIZE trailer value, capped at `limit`; a short input → 0 (decision 9) |
| `revision_head_reads_labels_and_chart` | name, revision, status, chart name/version/app version, last deployed, description |
| `revision_head_without_identity_labels_is_none` | decision 8 |
| `revision_head_with_bad_payload_keeps_row` | `chart: None`, times fall back to creation |
| `go_zero_time_reads_as_none` | `0001-01-01T00:00:00Z` |
| `helm_status_parses_every_helm_text` | the 8 Helm texts map to variants; Helm's own `unknown` and an arbitrary text → `Unknown(text)` |
| `releases_group_by_highest_revision` | sort by (namespace, name); highest revision wins |
| `failed_latest_names_deployed_revision` | decision 7 |
| `summary_debug_holds_no_payload_text` | none of the three distinctive texts in `format!("{:?}")` of summaries |
| `release_watch_config_selects_current_revisions` | labels, field selector, `MostRecent`, page size 10 |
| `history_watch_config_selects_one_release` | `owner=helm,name=api` |
| `history_revision_reads_modified_at_label` | `modifiedAt` wins over creation time |
| `history_sorts_newest_first` | 10 before 9 (numeric, not name order) |
| `secret_name_of_revision` | `sh.helm.release.v1.api.v38` |

`resource_watch_tests.rs`: `metadata_summary_watch_batches_like_summary_watch` (fake event stream through `batch_updates` with `PartialObjectMeta<Secret>`).

## Step 1 · `helm_release_detail_tests.rs` and `helm_values_diff_tests.rs`

| Test | Verifies |
|---|---|
| `mask_values_hides_strings_and_numbers` | bools, null, `{}`, `[]`, keys stay; count returned |
| `masked_user_values_have_hidden_header` | `# k8sBoard hid 3 values as <hidden>.` |
| `revealed_payload_has_values_and_notes_unmasked` | `helm_revealed` path: no mask, no header, notes text present |
| `detail_holds_notes_line_count_only` | `HelmReleaseDetail` has `notes_lines == 2` and no notes text in any field |
| `missing_config_reads_as_empty_values` | `{}` |
| `coalesce_merges_user_over_defaults` | nested maps merge |
| `coalesce_user_null_removes_default` | key gone |
| `coalesce_replaces_arrays_whole` | no element merge |
| `manifest_masks_secret_data_per_document` | `data` and `stringData` hidden; ConfigMap data kept |
| `manifest_keeps_only_source_comments` | `# Source:` kept; other comments dropped |
| `manifest_hides_unreadable_document` | placeholder line, counted |
| `manifest_env_literals_follow_env_values` | hidden by default, shown with `Shown` |
| `manifest_masks_last_applied_annotation` | rule 2 per document |
| `manifest_options_raise_budget` | `manifest_options()` budget: nodes 2,000,000, events 8,000,000, depth 128 |
| `detail_errors_have_fixed_text` | each `PayloadIssue` and decode failure maps to its table text; no payload byte in `Display` |
| `diff_lists_added_removed_changed_sorted` | paths sorted, three kinds |
| `diff_masks_strings_and_numbers` | `<hidden>` both sides; bools shown |
| `diff_keeps_masked_change_visible` | different secrets → one change `<hidden> → <hidden>` |
| `diff_of_equal_values_is_empty` | — |
| `diff_paths_quote_unusual_keys` | `annotations["kubernetes.io/ingress.class"]` |
| `diff_paths_index_arrays` | `servers[0].host` |
| `diff_cuts_long_values` | 200 chars + `…` |
| `diff_caps_at_limit` | 500 listed, rest counted |

Existing `object_yaml_tests.rs` pass unchanged (extraction only).

## Step 2 (app)

| File | Tests |
|---|---|
| `helm_rows_tests.rs` | `release_row_cells_match_columns`, `release_without_chart_has_absent_cells`, `helm_status_tones`, `revision_cell_sorts_numerically` |
| `kind_diagnosis` tests | `upgrade_failed_box_names_deployed_revision`, `rollback_failed_by_description_prefix`, `install_failed_for_other_descriptions`, `pending_release_box_warns`, `deployed_release_has_no_box`, `failed_without_description_has_fallback_text` |
| `live_sections` tests | `helm_history_rows_newest_first`, `helm_history_caps_at_fifty`, `helm_history_loading_failed_empty_notes`, `release_section_notes_missing_payload` |
| `related_objects` tests | `helm_release_row_has_history_subject` |
| `drawer` tests | `helm_release_drawer_has_overview_only` (step 3: `..._has_helm_tabs`) |
| `object_events_tests.rs`, `yaml_view` tests | `helm_release_has_no_event_subject`, `helm_release_has_no_object_ref` |
| `resource_kind` / `cluster_session` tests | `releases_follow_secrets_in_all`, `releases_have_no_count`, `releases_watch_count_within_bound` |
| `resource_actions_tests.rs` | `helm_release_menu_disables_rollback_and_uninstall` |

## Step 3 (app, `helm_release_view_tests.rs` unless noted)

`next_need_for_each_tab_layout_and_reveal` (Overview, Values, Manifest, Notes), `failed_slot_waits_for_refresh`, `manifest_never_needs_revealed_values`, `notes_open_masked_and_expire` (Notes need `Detail` masked, `Revealed` after Reveal, masked again after 30 s), `tab_change_drops_revealed`, `earlier_revision_is_highest_below`, `diff_needs_an_earlier_revision`, `shown_text_tracks_source_env_and_reveal`, `reveal_expires_after_thirty_seconds`, `revealed_copy_emits_private_clipboard_mark` (`private_copy` passes the selection to the writer and returns its mark; the view emits `SecretCopied`), `revealed_copy_fails_closed` (writer error → no mark, `copy_error` set), `empty_selection_copies_nothing`, `helm_subject_only_on_helm_tabs`, `helm_subject_overview_uses_latest`, `helm_subject_follows_chosen_revision`; `launch_options_tests.rs`: `releases_values_and_manifest_slugs_parse`, `values_slug_rejected_for_other_kinds`; `resource_actions_tests.rs`: `helm_release_menu_opens_values_and_manifest`; `live_sections` tests: `oldest_history_row_has_no_diff`, `values_changed_section_notes_first_revision`.

## Live checks and ui-verifier

- coder-lite step 1: the probe (0001 `probe-example.md` invocation, `monitor-uat-readonly.yml`, context `readonly@Monitor`) with `--watch-seconds 5 --helm`; inspect the output for any value or description text (there must be none; descriptions appear only as `description {n} chars`); copy result lines into [decisions.md](decisions.md) "UAT probe". The 0001 credential script reports 0 on the output.
- coder step 3: the live checks in [helm-safety.md](helm-safety.md).
- ui-verifier, `--config-dir .tmp/...`: step 2 `releases`, `releases-drawer`; step 3 `releases-drawer` (with the masked "Values changed" section), `releases-values`, `releases-manifest` (with `--filter <first release>`). Check W7 columns, status tones, the status box, History order, masked values only. No release on UAT → `releases` empty state only, noted in the report.
