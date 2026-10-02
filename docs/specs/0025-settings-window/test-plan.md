# 0025 · Test plan

[Back to index](README.md). Unit tests are offline. File tests use `std::env::temp_dir().join(format!("k8sboard-0025-{name}-{pid}"))`, removed at start and end. Fixture kubeconfigs hold fake values such as `token: fixture-token-value` so tests can assert absence.

## Step 1 — `kubeconfig_tests.rs`

| Test | Checks |
|---|---|
| `parse_reads_contexts_from_text` | two contexts, `source` = origin |
| `parse_merges_multiple_documents` | two `---` documents → contexts of both |
| `parse_error_does_not_quote_the_text` | invalid YAML with a token line → message lacks it |
| `server_drops_userinfo_path_and_query` | `https://u:p@h:6443/x?y` → `https://h:6443` |
| `auth_kind_per_user_shape` | table: exec, auth provider, client cert file/data, token, token file, basic, none |
| `exec_kind_keeps_only_the_command_file_name` | `/usr/local/bin/aws` and `C:\tools\aws.exe` + args → `exec: aws`, `exec: aws.exe`; args absent from `Debug` and `Display` |
| `entry_names_list_contexts_clusters_users` | file order |
| `connection_info_debug_has_no_credentials` | `format!("{:?}")` lacks the fixture token |

## Step 2a — `cluster_catalog.rs` (gpui `TestAppContext`, `run_until_parked`)

`catalog_lists_chain_before_standalone`, `added_registry_file_is_loaded`, `removed_registry_file_is_dropped`, `chain_is_not_reloaded_on_registry_change`, `failed_file_becomes_a_notice`. Every existing `app_shell` test passes unchanged.

## Step 2b

| File | Tests |
|---|---|
| `settings_window_tests.rs` | `open_twice_keeps_one_window`, `open_after_close_opens_a_new_window`, `closing_settings_keeps_the_app`, `closing_settings_clears_the_handle`, `closing_main_window_quits`, `pages_follow_w2_order`, `default_page_is_clusters`, `dialog_opens_in_settings_window` |
| `settings_tests.rs` (0024 file) | `theme_label_round_trip` (every `THEME_OPTIONS` entry), `unknown_theme_label_is_system`, `appearance_change_updates_settings_and_theme` (gpui: `Theme` mode follows) |
| `launch_options_tests.rs` | `screen_settings_parses`, `screen_settings_appearance_parses` |
| `screenshot` tests | `settings_screen_waits_for_catalog` |

`closing_settings_keeps_the_app` and `closing_main_window_quits` register `quit_when_main_window_closes` with a quit callback that sets a `Rc<Cell<bool>>` flag. An `on_app_quit` flag cannot work here: the test platform's `quit()` is a no-op and quit observers run only in `shutdown()` (window-and-sharing.md).

## Step 3

`cluster_form_tests.rs`:

| Test | Checks |
|---|---|
| `groups_follow_env_order_and_merge_dev_and_local` | Production, Staging, "Development · Local" |
| `empty_groups_are_skipped` | |
| `registered_rows_come_first_in_registry_order` | |
| `row_meta_shows_auth_kind_and_file_name` | `exec: aws · config` for `/home/u/.kube/config` and `C:\Users\u\.kube\config` |
| `duplicate_context_names_get_file_labels` | 0024 `switcher_label` |
| `origin_is_chain_registry_or_app_owned` | |
| `count_text_uses_singular_and_plural` | `1 cluster · 1 kubeconfig file` |
| `display_name_table` | empty → `Ok(None)`; 65 chars; tab; duplicate (case-insensitive) → exact messages |
| `display_name_may_equal_its_own_label` | no self-collision |
| `namespace_table` | `a`, `kube-system` ok; `-a`, `A`, `a_b`, 64 chars → exact message; empty → `Ok(None)` |
| `reset_removes_only_that_entry` | |
| `read_only_defaults_to_production` | `ClusterProfile.read_only` (cluster_registry tests) |
| `test_connection_reports_unreachable_server` | fixture at `https://127.0.0.1:1`, `timeout` = 2 s, own current-thread tokio runtime, **real time** (gpui `advance_clock` does not drive tokio timers); `Failed` text equals the top-level `ClusterError` `Display` |

`cluster_catalog.rs`: `same_path_text_table` (Windows: `C:\A\b.yaml` = `c:/a/./B.yaml`; Unix: `/a/B` ≠ `/a/b`, `\` is not a separator).

Window tests (`settings_window_tests.rs`): `editing_display_name_updates_title_bar` (shell observes the global), `invalid_namespace_is_not_saved`, `selection_change_recreates_inputs`, `read_only_switch_leaves_title_bar_badge`.

## Step 4

`kubeconfig_import_tests.rs`:

| Test | Checks |
|---|---|
| `collisions_report_context_display_name_cluster_and_user` | exact warning texts |
| `context_collision_names_its_source_file` | `ContextSummary.source`, not the kubeconfig's first source |
| `chain_collision_says_kubeconfig_chain` | Cluster/User against the chain → `your KUBECONFIG chain` |
| `no_collision_for_distinct_names` | |
| `already_added_file_is_rejected` / `chain_file_is_rejected` | `check_new_file` messages (case rules: `same_path_text_table`) |
| `pasted_file_path_slugs_the_first_context` | `arn:aws:eks:…/prod-eu-1` → `arn-aws-eks-…` ≤ 40; none → `pasted` |
| `pasted_file_path_skips_existing_names` | `-2`, `-3` |
| `write_pasted_never_overwrites` | `create_new` |
| `write_pasted_sets_owner_only_mode` | `#[cfg(unix)]`: file `mode & 0o077 == 0`; folder `mode & 0o777 == 0o700` |
| `app_owned_only_under_kubeconfigs_folder` | |
| `preview_debug_has_no_credentials` | derived `Debug` lacks the fixture token |
| `remove_drops_path_entries_and_last_used` | `cluster_form::remove_kubeconfig` |
| `clipboard_over_one_mebibyte_is_rejected` | |

`cluster_catalog.rs` (gpui): `paste_writes_loads_then_registers`, `paste_survives_settings_window_close` (close the window while `Saving`; file and registry path both exist), `failed_write_registers_nothing_and_keeps_clipboard`, `clipboard_fingerprint_matches_same_text_only`, `unregistered_pasted_file_raises_notice`, `remove_unregistered_deletes_the_file`, `remove_app_owned_deletes_before_dropping_entries`, `failed_delete_keeps_the_file_listed` (the target path is a directory, so `remove_file` fails on every OS).

Window tests: `paste_cancel_drops_text` (`paste_text` is `None` after Cancel), `paste_escape_drops_text` (also proves Esc runs `on_close`), `paste_preview_error_drops_text`, `paste_imports_and_clears_matching_clipboard` (test clipboard via `TestAppContext::write_to_clipboard`), `paste_keeps_unrelated_clipboard`, `paste_disabled_without_config_dir`, `import_file_adds_only_the_path`, `settings_window_is_released_on_close` (a `WeakEntity<SettingsWindow>` no longer upgrades).

## Live checks (coder-lite, read-only, UAT)

Always `--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0025-<case>`.

1. Test connection → `Connected · v1.29.5 · {n} ms`; record n. With `RUST_LOG=kube=trace`, the log shows one request, `GET /version` (record the count only; never copy headers).
2. Import a fixture copy of a fake kubeconfig from `.tmp/` → its contexts appear in the main switcher; `settings.json` holds the path only (`grep -c "token" settings.json` → 0).
3. Never paste `monitor-uat-readonly.yml`: the paste check uses a fake fixture placed on the clipboard; the file appears in `.tmp/config-0025-paste/kubeconfigs/`.
4. Windows: `Ctrl ,` twice, with the main window focused the second time, brings the one Settings window to the front.
5. `grep -rnE "(trace|debug|info|warn|error)!" crates/app/src/kubeconfig_import.rs crates/app/src/cluster_catalog.rs crates/app/src/settings_window.rs` shows paths and error kinds only.

## ui-verifier (screenshot build; writes are off)

| Screen | Seed | Expect (W2) |
|---|---|---|
| `settings`, light and dark | entry `environment: production`, `display_name: uat-monitor` | nav (Clusters, Appearance, About), Clusters header `1 cluster · 1 kubeconfig file`, group "Production", row badge PROD, row meta `{auth kind} · monitor-uat-readonly.yml`, form sections General / Connection / Safety, auth kind only |
| `settings`, clean dir | none | group "Staging", STG badge, Environment "Auto (STG)" |
| `settings-appearance` | `theme: dark` | dropdown shows Dark |

No token, certificate, or exec argument visible in any screenshot. Report misaligned form rows, clipped labels, and color literals as defects. Known deviations, not defects: no nav count; row meta shows the file name, not `~/…`.
