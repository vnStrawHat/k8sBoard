# 0043 · Files to touch

[Back to index](README.md). **S** = step. Each step passes the gate alone; every new item has a production reader in its step (dead-code rule). Base: main `52f8f87`. 0044 also adds a `Settings` section (`dock`) and 0046 edits `cluster_switcher*.rs`, `title_bar.rs`, and the start path in `app_shell.rs`: whichever lands second rebases; no shared logic.

## Cargo (step 4 and 5 only)

| S | File | Change |
|---|---|---|
| 4 | root `Cargo.toml` | `kube` features + `"http-proxy"`, `"socks5"` |
| 4 | `crates/cluster/Cargo.toml` | `http = "1"` normal dependency (drop `dep:http` from `test-support`) |
| 5 | root + `crates/app/Cargo.toml` | `notify = "7"` (workspace dependency; resolves to the locked 7.0.0) |

`Cargo.lock`: no new `[[package]]` in any step (AC 2).

## `crates/cluster` (steps 2, 4, 5)

| S | File | Change |
|---|---|---|
| 2 | `src/pod_shell.rs` | `ShellCommand`: `Serialize`, `Deserialize`, `rename_all = "lowercase"` |
| 4 | `src/connection.rs` (+ `connection_tests.rs`) | `ProxyChoice`, `ProxyUrl` (lowercase scheme, manual `Debug`), `ProxyUrlError`, `ClusterError::InvalidProxy`, `open(.., proxy)`, mapping table, redaction text |
| 4 | `src/kubeconfig.rs` (+ tests) | `ConnectionInfo.proxy` (userinfo dropped, the `server` helper) |
| 4 | `src/lib.rs` | export `ProxyChoice`, `ProxyUrl`, `ProxyUrlError` |
| 4 | `examples/probe.rs`, `tests/connection.rs` | `open(.., &ProxyChoice::Kubeconfig)` |
| 5 | `src/kubeconfig.rs` (+ tests) | `Kubeconfig::parse_file` (kube's relative-path rule against the file's folder); `KubeconfigError::TooLarge { path }` |

`object_write.rs`, `clippy.toml`, and the connect files are untouched.

## `crates/app`

| S | File | Change |
|---|---|---|
| 1 | `settings.rs` (+ `settings_tests.rs`) | `GeneralSettings`, `AppearanceSettings`, `RowDensity`, accessors, `is_default`, option tables; allow-list |
| 1 | `settings_window.rs` (+ tests) | `SettingsPage::General`, `general_page`, density item on Appearance, `PAGES` |
| 1 | `file_export.rs` | start folder from `general.export_dir` (background `is_dir` check), write `parent()` after `Saved` |
| 1 | `issue_feeds.rs` (+ tests), `cluster_session.rs` | `CertificateWatch`, `condition_plan(.., watch)` |
| 1 | `workspace.rs` | `.with_size(..)` on every `DataTable::new`; `_settings_observer` (`observe_global::<AppSettings>`) |
| 2 | `settings.rs`, `settings_window.rs` | `LogSettings`, `TerminalSettings`; `logs_page`, `terminal_page`; `PAGES` |
| 2 | `log_tab.rs` (+ tests) | `POD_TAIL_LINES` → `tail_lines()`; toggles from `LogSettings` |
| 2 | `terminal_session.rs` (+ tests), `shell_tab.rs`, `shell_open.rs`, `terminal_element.rs` | scrollback param; default shell; font size |
| 1, 2 | `launch_options.rs` (+ tests) | `--screen settings-general` (step 1), `settings-logs`, `settings-terminal` (step 2); USAGE |
| 3 | `cluster_form.rs` (+ tests) | `move_cluster`, `step_cluster`, `MoveStep`; `cluster_matches` |
| 3 | `cluster_switcher_rows.rs` | `search_text` → `pub(crate)` |
| 3 | `clusters_page.rs` (+ tests) | search input, drag, hint, Color row, `MoveClusterUp/Down` handlers |
| 3 | `environment.rs` (+ tests), `cluster_registry.rs` (+ tests), `title_bar.rs` | `ClusterColor`, `cluster_color`; `ClusterEntry.color`, `ClusterProfile.color`; border |
| 3 | `keymap.rs`, `shortcut_sheet.rs` (+ tests) | `alt-up` / `alt-down` in `SettingsWindow && !Input`; sheet rows |
| 4 | `cluster_registry.rs` (+ tests) | `ClusterProxy` (manual `Debug`), `ClusterEntry.proxy`, `ClusterProfile.proxy: Result<ProxyChoice, ProxyUrlError>`, `open_cluster` |
| 4 | `cluster_form.rs`, `clusters_page.rs` (+ tests) | Proxy row, commit on Enter or blur, `Not applied`, messages; `test_connection(.., proxy, ..)` |
| 4 | `cluster_session.rs`, `cluster_health.rs` | `open_cluster` with the profile's proxy |
| 5 | `kubeconfig_folder.rs` (new, inline tests) | candidates, `scan_folder`, `diff_scan`, `load_folder_file` (bounded), caps, debounce constants |
| 5 | `cluster_catalog.rs` (+ tests) | folder parts, watchers, event channel + `folder_events_for_test`, debounce/max-wait task, notices, `start_kubeconfigs()`, observer of `kubeconfig_folders` |
| 5 | `app_shell.rs` (+ tests) | `on_catalog_changed` / `resolve_start`: folder files only as exact `last_used`; empty state `No cluster selected…` + open the switcher |
| 5 | `cluster_health.rs` (+ tests), `app_shell.rs` (`ProbeCandidate` build) | `ProbeCandidate.origin`; `due` skips `RowOrigin::Folder` |
| 5 | `launch_options.rs` (+ tests) | `standalone_files` takes folder files after registry files |
| 5 | `cluster_form.rs`, `clusters_page.rs`, `cluster_registry.rs` | `RowOrigin::Folder`, folder lines, Stop watching, menu item enabled; `kubeconfig_folders` |
| 5 | `main.rs` | `mod kubeconfig_folder;` |

## Docs after merge (orchestrator or architect)

- 0024 `persisted-prefs.md` reserved table: `issues.watch_tls_secrets` and `logs.export_dir` moved to `general.*`; `appearance.density`, `.color` → 0043; `secrets.clipboard_clear_seconds` stays reserved (0016 decision 26 unchanged).
- 0025 `other-pages.md`: General, Terminal & Shell, Logs owned by 0043 (not 0016/0020, 0036, 0019); README non-goals (watch folder, drag, colours, proxy, density, search) → 0043.
- 0026 / 0046: start and probe rules for folder files (folder-watch.md "Untrusted by default").
- 0028 `keymap.md`: `alt-up` / `alt-down` in the Settings window.
- 0036 decision 32 and 0020 decision 11: point to 0043.
- Roadmap `wireframe-gap-audit.md` gap 5 rows → Done when verified.
