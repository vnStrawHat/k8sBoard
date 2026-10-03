# 0043 · Test plan

[Back to index](README.md). Offline and deterministic, one behaviour per test. File tests write under `std::env::temp_dir().join(format!("k8sboard-0043-{name}-{pid}"))` (the project `.tmp/` in agent runs) and clean it at start and end. No test opens a real cluster connection; GPUI tests install settings with `WriteMode::Disabled` (0024). Folder tests never use a real `notify` watcher or wall-clock sleeps: they send `FolderEvent`s through `folder_events_for_test` and advance the GPUI fake clock.

## Step 1: General, density

| Test | File | Checks |
|---|---|---|
| `settings_keys_are_the_allow_list` (extended per step) | `settings_tests.rs` | every key of the settings-model.md table, nothing else (no `clipboard_clear_seconds`) |
| `default_file_is_minimal` (unchanged expectation) | `settings_tests.rs` | defaults add no `general`/`appearance`/`logs`/`terminal` key |
| `section_with_one_change_round_trips` | `settings_tests.rs` | `watch_tls_secrets: false` survives JSON; `export_dir` stays `None` |
| `density_row_heights` | `settings_tests.rs` | Compact 28, Comfortable 36; JSON `"compact"`/`"comfortable"`; default Compact |
| `option_labels_round_trip` | `settings_tests.rs` | every option table: `label` ↔ `from_label`; unknown label → default; default label ends with ` (default)` |
| `pages_follow_w2_order` (updated) | `settings_window_tests.rs` | General, Clusters, Appearance, Keyboard Shortcuts, Safety (step 2 adds Terminal & Shell, Logs), About |
| `tls_watch_off_turns_the_secrets_feed_off` | `issue_feeds_tests.rs` | `CertificateWatch::Skip` → Secrets `FeedPlan::Off("off in Settings")`, other kinds unchanged |
| `tls_watch_off_is_named_in_coverage` | `issue_feeds_tests.rs` | note holds `Not checked: certificates (off in Settings).` |
| `watch_count_follows_the_condition_plan` (extended) | `issue_feeds_tests.rs` | Skip → one watch fewer |
| `export_start_folder_falls_back_to_home` | `file_export.rs` | stored dir missing → home; stored dir present → it (pure helper over an `is_dir` result) |
| `saved_export_remembers_its_folder` | `file_export.rs` / `app_shell_tests.rs` | after `Saved`, `general.export_dir` = parent; cancel and failure leave it |
| `density_change_rerenders_the_table` | `app_shell_tests.rs` | update density → the workspace is notified (observer) and the pod table's options size is 36 px |
| `launch_screen_settings_general` | `launch_options_tests.rs` | `--screen settings-general` parses |

## Step 2: Logs, Terminal & Shell

| Test | File | Checks |
|---|---|---|
| `shell_command_serializes_lowercase` | `cluster` `pod_shell_tests.rs` | `auto`, `bash`, `sh` round trip |
| `tail_lines_is_clamped` / `scrollback_is_clamped` / `font_size_is_clamped` | `settings_tests.rs` | below, inside, above the range; scrollback 50,000 → 10,000 |
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
| `proxy_url_parse_table` | `connection_tests.rs` | one row per rule. Accepts `http://p:3128`, `socks5://[::1]:1080`, `HTTP://P:1/`, `SOCKS5://h:1` (stored scheme `socks5`). Rejects `https://p`, `ftp://p`, `http://u:p@p:1`, `http://u@p`, `http://:1`, `http://p:0`, `http://p:70000`, `http://p/x`, `http://p?q`, spaces, 2,049 chars |
| `proxy_url_display_is_scheme_host_port` | `connection_tests.rs` | `HTTP://p:3128/` → `http://p:3128` |
| `proxy_choice_sets_the_config` | `connection_tests.rs` | over a kubeconfig with and without `proxy-url`: Kubeconfig / Direct / Url → expected `proxy_url` (pure helper split from `open`) |
| `proxy_url_userinfo_never_appears_in_invalid_config_error` (updated) | `connection_tests.rs` | only `ftp://user:secret@…` fails now, without `secret` in the text; `http`/`socks5` with userinfo build a client |
| `debug_of_proxy_types_shows_host_only` | `connection_tests.rs` | `Debug` of `ProxyChoice` and `ProxyUrl`: scheme, host, port only |
| `connection_info_proxy_drops_userinfo` | `kubeconfig_tests.rs` | `http://user:secret@p:3128` → `http://p:3128` |
| `cluster_proxy_json_shape` | `cluster_registry_tests.rs` | `"direct"`, `{"url": "…"}`; absent → `Ok(ProxyChoice::Kubeconfig)` |
| `invalid_stored_proxy_fails_closed` | `cluster_registry_tests.rs` | `Url("http://u:p@x")` → profile `Err`; `open_cluster` → `InvalidProxy`, text without `u:p`, no client built |
| `debug_of_cluster_proxy_hides_userinfo` | `cluster_registry_tests.rs` | hand-edited `Url("http://u:p@x:1")` → `Url(<invalid>)`; valid → `Url(http://x:1)` |
| `proxy_form_rejects_credentials` | `cluster_form_tests.rs` | the exact Credentials message; stored value unchanged |
| `proxy_input_commits_on_enter_or_blur_only` | `clusters_page_tests.rs` | typing a valid URL stores nothing; Enter stores it; blur stores it; `Not applied` shown until then |
| `proxy_input_never_shows_an_unparsable_value` | `clusters_page_tests.rs` | stored `Url("http://u:p@x")` → input empty |

## Step 5: folder watch

| Test | File | Checks |
|---|---|---|
| `scan_keeps_kubeconfig_candidates` | `kubeconfig_folder.rs` | `.yaml`, `config`, no extension kept; `.hidden`, `x.txt`, folders, 1 MiB + 1 dropped |
| `scan_caps_at_fifty_by_name` | `kubeconfig_folder.rs` | 52 files → 50 kept, `skipped_over_cap` 2 |
| `diff_scan_reports_added_changed_removed` | `kubeconfig_folder.rs` | len or mtime change → changed |
| `missing_folder_is_an_error` | `kubeconfig_folder.rs` | `scan_folder` of a removed dir → `Err(NotFound)` |
| `oversized_file_is_refused_by_the_bounded_read` | `kubeconfig_folder.rs` | a file grown past 1 MiB after the scan → `TooLarge`, at most 1 MiB + 1 bytes read |
| `parse_file_resolves_relative_paths_against_the_folder` | `cluster` `kubeconfig_tests.rs` | relative `tokenFile`, `client-key`, exec `./bin/x` → absolute under the file's folder; same result as kube `read_from` on the fixture |
| `standalone_files_put_folders_last_without_duplicates` | `launch_options_tests.rs` | chain, registry, folder order; a folder file in the registry loads once |
| `folder_file_is_never_the_fallback_start` | `app_shell_tests.rs` | only a folder file loaded, no `last_used` → no session, switcher open; a folder context named by `--context` or `current-context` → not started; a folder file appearing while no session runs → still none |
| `folder_file_starts_when_it_is_last_used` | `app_shell_tests.rs` | `last_used` = that folder context → it starts |
| `folder_rows_are_never_probed_automatically` | `cluster_health_tests.rs` | `TokenFile`, `ClientCertificate`, `Token` rows with `RowOrigin::Folder` → not in `due`; the same rows from a registry file → in `due` |
| `folder_file_rows_come_and_go` | `cluster_catalog_tests.rs` | inject an event, advance 500 ms → added file's rows appear; delete + event → rows leave; folder listing unchanged by the app |
| `burst_of_events_rescans_once` | `cluster_catalog_tests.rs` | five events 100 ms apart → one rescan 500 ms after the last (counter) |
| `endless_events_rescan_every_two_seconds` | `cluster_catalog_tests.rs` | an event every 100 ms for 5 s → rescans at about 2 s and 4 s, then one after the stream ends |
| `broken_rewrite_keeps_the_last_good_file` | `cluster_catalog_tests.rs` | valid → invalid content → rows stay, `Skipped` notice; valid again → notice gone |
| `missing_folder_raises_a_notice` | `cluster_catalog_tests.rs` | remove the folder + event → its rows leave, notice text per folder-watch.md |
| `stop_watching_keeps_the_files` | `cluster_catalog_tests.rs` | Stop → rows leave, `kubeconfig_folders` empty, files still on disk |
| `folder_rows_cannot_be_removed` | `cluster_form_tests.rs` | `RowOrigin::Folder` → disabled with the exact reason |

## Checks

- Greps: `grep -rn "POD_TAIL_LINES" crates/app/src` → nothing; `CLIPBOARD_CLEAR_DELAY` still present (unchanged); `grep -rn "read_from" crates/app/src` → nothing new; 0003 colour-literal grep clean; `git diff Cargo.lock` adds no `[[package]]`.
- coder-lite (read-only UAT, `--config-dir .tmp/config-0043`): pods with Compact and Comfortable. Test connection with `None (direct)` → `Connected · v1.29.5`, and the `RUST_LOG=kube=trace` trace shows one `GET /version`. With `Custom URL http://127.0.0.1:9` (nothing listens) → `Failed: …` whose text names the proxy connect error (connection refused / tunnel), and the trace shows the attempt to `127.0.0.1:9` and no request line to the API server host. A watched `.tmp/kube-folder` with a copy-free fixture kubeconfig (no credentials) is listed within the debounce, never probed, never started.
- ui-verifier, light and dark: `settings-general`, `settings-appearance`, `settings-logs`, `settings-terminal`, `settings` (search box, swatches, Proxy row with `Not applied`, a seeded watched-folder line), `pods` at 28 and 36 px (no clipped status pill, usage bar, or checkbox; header included), title-bar border with a seeded Teal colour. A clip at 28 px is a defect of the cell renderer.
