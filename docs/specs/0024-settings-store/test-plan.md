# 0024 · Test plan

[Back to index](README.md). Unit tests are offline. File tests use `std::env::temp_dir().join(format!("k8sboard-0024-{name}-{pid}"))`, remove it at the start and the end, and never touch the real config dir. Agent runs export `TMP`/`TMPDIR` to the project `.tmp/`.

## Step 1 — `kubeconfig_tests.rs` (fixtures written to the temp dir, no credentials)

| Test | Checks |
|---|---|
| `load_merges_contexts_of_all_files_in_order` | two files → contexts of both, file order |
| `first_file_wins_for_duplicate_context_names` | `source` and `cluster` come from the first file |
| `current_context_comes_from_the_first_file_that_sets_it` | first file without it, second with it |
| `context_source_is_the_defining_file` | `source` per context |
| `unreadable_file_is_skipped_and_reported` | missing second path → loaded, `skipped` has `Read` |
| `incompatible_file_is_skipped` | different `kind` → `Incompatible` |
| `load_fails_when_no_file_loads` | first error returned |
| `context_not_found_lists_every_source` | message names both paths |
| `incompatible_file_keeps_earlier_files` | file 1 loads, file 2 has another `apiVersion`, file 3 loads → contexts of 1 and 3 (pre-check, not `merge` Err) |

Removed: `load_reports_missing_file_with_path` (covered by `load_fails_when_no_file_loads`). Existing tests build fixtures with the new `from_document(vec!["fixture.yaml".into()], document, &HashMap::new())`.

## Step 2

| File | Tests |
|---|---|
| `settings_tests.rs` | `default_settings_have_the_current_version`, `default_settings_round_trip`, `full_settings_round_trip`, `unknown_fields_are_ignored` (top level and nested), `missing_fields_take_defaults`, `theme_serializes_lowercase`, `default_file_is_minimal` (`{"version":1,"theme":"system"}` in step 2; `"registry":{}` added in step 3), `settings_keys_are_the_allow_list` (walk the JSON of a fully populated value; the list grows in steps 3 and 4) |
| `settings_store_tests.rs` | `config_dir_prefers_flag_then_env_then_fallback`, `empty_env_value_is_ignored`, `relative_config_dir_becomes_absolute`, `missing_file_gives_defaults_without_notice`, `invalid_json_is_backed_up_and_reset`, `wrong_type_is_backed_up_and_reset`, `missing_version_is_backed_up_and_reset`, `version_above_u32_is_backed_up_and_reset`, `backup_replaces_an_older_backup`, `newer_version_loads_with_writes_disabled`, `write_creates_the_dir_and_file`, `write_replaces_existing_file_and_leaves_no_temp`, `older_generation_never_overwrites_newer` (gate), `serialized_settings_end_with_newline` |
| `settings_tests.rs` (gpui `TestAppContext`) | `update_writes_the_newest_snapshot` (two updates, `run_until_parked`, file holds the second), `unchanged_update_does_not_write` (file absent), `disabled_writes_never_touch_disk`, `write_failure_sets_notice` (config dir path is an existing file → `WriteFailed`), `flush_writes_the_last_snapshot` (update, then `AppSettings::flush` without parking → file holds it), `dismiss_clears_the_notice` |
| `launch_options_tests.rs` | `config_dir_flag_is_parsed`, `theme_accepts_system`, `theme_rejects_unknown` (updated) |

## Step 3

| File | Tests |
|---|---|
| `environment_tests.rs` | `guess_table` (every example in [environments.md](environments.md), one row each with a message), `riskiest_match_wins`, `tokens_ignore_substrings` (`latest`, `contest` → STG), `trailing_digits_are_trimmed`, `cluster_name_counts_when_context_is_unknown`, `uat_monitor_context_is_staging` (`readonly@Monitor` + `cluster.local`), `badge_text_per_environment`, `environment_order_is_risk` |
| `cluster_registry_tests.rs` | `profile_of_unregistered_context_uses_name_and_guess`, `entry_overrides_name_and_environment`, `blank_display_name_falls_back_to_context`, `entry_matches_only_the_same_source`, `start_choice_prefers_requested_in_load_order` (chain before registry file), `requested_missing_is_reported`, `start_choice_uses_last_used_from_the_same_source` (two same-named contexts, last-used picks the registry one), `last_used_beats_current_context_of_the_same_file`, `explicit_kubeconfig_ignores_last_used_from_other_files`, `explicit_kubeconfig_ignores_last_used_from_other_files`, `stale_last_used_is_ignored`, `switcher_label_adds_file_name_for_duplicate_names`, `entry_without_context_is_corrupt` (via `load_settings`) |
| `launch_options_tests.rs` | `kubeconfig_chain_flag_wins`, `kubeconfig_chain_lists_every_env_entry`, `kubeconfig_chain_falls_back_to_home`, `standalone_files_skip_chain_members_and_duplicates` |
| `app_shell_tests.rs` | `last_used_is_written_on_live`, `last_used_is_not_written_on_failure` (decision 22, amended for 0026) |

## Step 4 — `table_view.rs` tests

`prefs_round_trip_by_column_name`, `unknown_column_names_are_dropped`, `flexible_column_is_never_hidden`, `sort_on_unknown_column_is_dropped`, `screen_key_per_screen`, `custom_kind_key_is_the_crd_name_never_a_builtin_key`; `kind_table`: `new_view_applies_saved_prefs`.

## Step 5

`cluster_registry` test: `entry_mut_appends_once`. `resource_actions` tests: `namespaces_menu_has_set_as_default`, `set_as_default_is_checked_for_the_default`, `other_kinds_have_no_set_as_default`; `app_shell` (pure helper): `start_namespace_prefers_the_flag`, `start_namespace_falls_back_to_the_saved_default`, `toggle_default_namespace_stores_the_namespace`, `toggle_default_namespace_clears_it_when_already_set`.

## Live checks (coder-lite, read-only, UAT)

Always `--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-<case>`.

1. Clean dir, normal run for 20 s, then stop → `.tmp/config-clean/settings.json` holds `last_used` only; `test ! -e "$APPDATA/k8sboard"` passes (Windows; `~/.config/k8sboard` elsewhere).
2. `KUBECONFIG="monitor-uat-readonly.yml;.tmp/missing.yml"` without `--kubeconfig` → contexts listed, notice "Skipped kubeconfig".
3. Corrupt file (`{`) → `.bak` with the same bytes, notice shown, new file valid after a context switch.

## ui-verifier (screenshot build, seeded files; writes are off)

| Case | Seed | Expect |
|---|---|---|
| `pods` light/dark, clean dir | none | STG badge (amber) before `readonly@Monitor`, amber 3 px title-bar top border, no notice |
| `pods`, `env-prod` | entry `environment: production`, `display_name: uat-monitor` | PROD badge, red top border, label `uat-monitor` |
| `pods`, `tables` | `tables.pods` sort Restarts descending, hidden Node | sort arrow on Restarts, no Node column |
| `pods`, `corrupt` | `{` | notice button visible; tooltip text per settings-store.md |
| `namespaces`, default | `default_namespace: monitoring` | start scope `ns: monitoring` |

Compare with W1 (`.ctx` badge, `.tb` border-top). Report any color literal or misaligned badge as a defect.
