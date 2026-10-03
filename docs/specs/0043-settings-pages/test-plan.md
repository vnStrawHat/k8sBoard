# 0043 · Test plan

[Back to index](README.md). Offline and deterministic, one behaviour per test. File tests write under `std::env::temp_dir().join(format!("k8sboard-0043-{name}-{pid}"))` (the project `.tmp/` in agent runs) and clean it at start and end. No test opens a real cluster connection; GPUI tests install settings with `WriteMode::Disabled` (0024).

## Step 1: General, density

| Test | File | Checks |
|---|---|---|
| `settings_keys_are_the_allow_list` (extended per step) | `settings_tests.rs` | every key of the settings-model.md table, nothing else |
| `default_file_is_minimal` (unchanged expectation) | `settings_tests.rs` | defaults add no `general`/`appearance`/`logs`/`terminal` key |
| `section_with_one_change_round_trips` | `settings_tests.rs` | `clipboard_clear_seconds: 60` survives JSON; other general fields keep defaults |
| `clipboard_clear_is_clamped` | `settings_tests.rs` | 0 → 5 s, 30 → 30 s, 10,000 → 600 s |
| `density_row_heights` | `settings_tests.rs` | Compact 28, Comfortable 36; JSON `"compact"`/`"comfortable"` |
| `option_labels_round_trip` | `settings_tests.rs` | every option table: `label` ↔ `from_label`; unknown label → default; default label ends with ` (default)` |
| `pages_follow_w2_order` (updated) | `settings_window_tests.rs` | General, Clusters, Appearance, Keyboard Shortcuts, Safety (step 2 adds Terminal & Shell, Logs), About |
| `tls_watch_off_turns_the_secrets_feed_off` | `issue_feeds_tests.rs` | `CertificateWatch::Skip` → Secrets `FeedPlan::Off("off in Settings")`, other kinds unchanged |
| `tls_watch_off_is_named_in_coverage` | `issue_feeds_tests.rs` | note holds `Not checked: certificates (off in Settings).` |
| `watch_count_follows_the_condition_plan` (extended) | `issue_feeds_tests.rs` | Skip → one watch fewer |
| `export_start_folder_falls_back_to_home` | `file_export.rs` | stored dir missing → home; stored dir present → it (pure helper over an `is_dir` result) |
| `saved_export_remembers_its_folder` | `file_export.rs` / `app_shell_tests.rs` | after `Saved`, `general.export_dir` = parent; cancel and failure leave it |
| `secret_copy_uses_the_clear_setting` | `app_shell_tests.rs` | setting 15 s → the clear timer is armed with 15 s |
| `launch_screen_settings_general` | `launch_options_tests.rs` | `--screen settings-general` parses |

## Step 2: Logs, Terminal & Shell

| Test | File | Checks |
|---|---|---|
| `shell_command_serializes_lowercase` | `cluster` `pod_shell_tests.rs` | `auto`, `bash`, `sh` round trip |
| `tail_lines_is_clamped` / `scrollback_is_clamped` / `font_size_is_clamped` | `settings_tests.rs` | below, inside, above the range |
| `new_log_tab_takes_the_log_defaults` | `log_tab` tests (fake API window) | settings wrap on, timestamps off, JSON on → a new tab has them; a toggle on that tab leaves `AppSettings` unchanged |
| `log_tab_requests_the_configured_tail` | `log_tab` tests (fake API window) | `tail_lines: 500` → the recorded log request has `tailLines=500`; late joiner keeps 50 |
| `scrollback_follows_the_argument` (replaces `scrollback_is_capped`) | `terminal_session_tests.rs` | `new(size, 1_000)` keeps 1,000 lines of 1,500 |
| `open_shell_uses_the_default_shell` | `app_shell_tests.rs` (shell fixture) | `default_shell: bash` → the intent and the tab command are `Bash` |
| `terminal_font_size_overrides_the_theme` | `terminal_element_tests.rs` | `Some(16)` → cell height from 16 px; `None` → theme size |
| `launch_screen_settings_logs_and_terminal` | `launch_options_tests.rs` | both parse |

## Step 3: search, order, colour

| Test | File | Checks |
|---|---|---|
| `move_cluster_reorders_inside_the_group` | `cluster_form_tests.rs` | [a, b, c] move c → a gives [c, a, b]; `cluster_groups` shows that order |
| `move_cluster_registers_the_group_only` | `cluster_form_tests.rs` | unregistered rows of the group get entries; other groups keep their entries and order |
| `move_cluster_across_groups_is_a_no_op` | `cluster_form_tests.rs` | PROD row onto a DEV row → registry unchanged |
| `step_cluster_stops_at_the_ends` | `cluster_form_tests.rs` | Up on the first, Down on the last → unchanged |
| `shortcut_numbers_follow_the_new_order` | `cluster_switcher_rows_tests.rs` | after a move, `nth_cluster(.., 1)` is the moved row |
| `cluster_search_matches_label_context_env_and_file` | `cluster_form_tests.rs` | `prod eu`, `STG`, a file name each match; `xyz` matches none |
| `cluster_color_defaults_to_the_environment` | `environment_tests.rs` | `ClusterColor::of` per environment; `ALL` order |
| `cluster_color_uses_theme_tokens` | `environment_tests.rs` | each colour equals its token (Purple = `magenta`, Teal = `cyan`) |
| `environment_color_is_the_default_cluster_color` | `environment_tests.rs` | same `Hsla` for every environment |
| `profile_color_prefers_the_entry` | `cluster_registry_tests.rs` | entry Teal → Teal; none → environment colour |
| `picking_the_environment_colour_stores_none` | `clusters_page_tests.rs` | click the PROD swatch on PROD → `color: None`; Teal → `Some(Teal)` |
| `alt_arrows_move_the_selected_cluster` | `clusters_page_tests.rs` | `MoveClusterDown` on the first row → second place; ignored while an input is focused |
| `title_bar_border_uses_the_cluster_color` | `app_shell_tests.rs` (pure helper) | PROD + Teal → `cyan`; badge colour stays `danger` |

## Step 4: proxy

| Test | File | Checks |
|---|---|---|
| `proxy_url_parse_table` | `connection_tests.rs` | one row per validation rule (accepts `http://p:3128`, `https://p`, `socks5://[::1]:1080`, `HTTP://P:1/`; rejects `ftp://p`, `http://u:p@p:1`, `http://u@p`, `http://:1`, `http://p:0`, `http://p:70000`, `http://p/x`, `http://p?q`, spaces, 2,049 chars) |
| `proxy_url_display_is_scheme_host_port` | `connection_tests.rs` | `http://p:3128/` → `http://p:3128` |
| `proxy_choice_sets_the_config` | `connection_tests.rs` | over a kubeconfig with and without `proxy-url`: Kubeconfig / Direct / Url → expected `proxy_url` (pure helper split from `open`) |
| `invalid_settings_proxy_fails_open` | `connection_tests.rs` | `Url("http://u:p@x")` → `InvalidConfig`; text has no `u:p` |
| `proxy_url_userinfo_never_appears_in_invalid_config_error` (updated) | `connection_tests.rs` | now only `ftp://user:secret@…` fails; `http`/`socks5` with userinfo build a client |
| `connection_info_proxy_drops_userinfo` | `kubeconfig_tests.rs` | `http://user:secret@p:3128` → `http://p:3128` |
| `cluster_proxy_json_shape` | `cluster_registry_tests.rs` | `"direct"`, `{"url": "…"}`; absent → `ProxyChoice::Kubeconfig` |
| `proxy_form_rejects_credentials` | `cluster_form_tests.rs` | the exact Credentials message; stored value unchanged |
| `debug_of_proxy_choice_shows_host_only` | `connection_tests.rs` | `Debug` text has no path, no query |

## Step 5: folder watch

| Test | File | Checks |
|---|---|---|
| `scan_keeps_kubeconfig_candidates` | `kubeconfig_folder.rs` | `.yaml`, `config`, no extension kept; `.hidden`, `x.txt`, folders, 1 MiB + 1 dropped |
| `scan_caps_at_fifty_by_name` | `kubeconfig_folder.rs` | 52 files → 50 kept, `skipped_over_cap` 2 |
| `diff_scan_reports_added_changed_removed` | `kubeconfig_folder.rs` | len or mtime change → changed |
| `missing_folder_is_an_error` | `kubeconfig_folder.rs` | `scan_folder` of a removed dir → `Err(NotFound)` |
| `standalone_files_put_folders_last_without_duplicates` | `launch_options_tests.rs` | chain, registry, folder order; a folder file in the registry loads once |
| `folder_file_rows_come_and_go` | `cluster_catalog_tests.rs` | add a file → rows appear after the debounce; delete → rows leave; no write in the folder (dir listing unchanged) |
| `broken_rewrite_keeps_the_last_good_file` | `cluster_catalog_tests.rs` | valid → invalid content → rows stay, `Skipped` notice; valid again → notice gone |
| `burst_of_events_rescans_once` | `cluster_catalog_tests.rs` | five events inside 500 ms → one rescan (counter) |
| `missing_folder_raises_a_notice` | `cluster_catalog_tests.rs` | remove the folder → its rows leave, notice text per folder-watch.md |
| `stop_watching_keeps_the_files` | `cluster_catalog_tests.rs` | Stop → rows leave, `kubeconfig_folders` empty, files still on disk |
| `folder_rows_cannot_be_removed` | `cluster_form_tests.rs` | `RowOrigin::Folder` → disabled with the exact reason |

## Checks

- Greps: `grep -rn "CLIPBOARD_CLEAR_DELAY\|POD_TAIL_LINES" crates/app/src` → nothing; 0003 colour-literal grep clean; `git diff Cargo.lock` adds no `[[package]]`.
- coder-lite (read-only UAT, `--config-dir .tmp/config-0043`): pods with Compact and Comfortable; Test connection with `None (direct)` → `Connected`; with `Custom URL http://127.0.0.1:9` → `Failed`, no request reaches the API server, and the 0001 request trace shows only `GET /version` on the direct run.
- ui-verifier, light and dark: `settings-general`, `settings-appearance`, `settings-logs`, `settings-terminal`, `settings` (search box, swatches, Proxy row, a seeded watched folder line), `pods` at 28 and 36 px (no clipped pill, bar, or checkbox), title-bar border with a seeded Teal colour.
