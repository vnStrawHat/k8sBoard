# 0043 · As built

[Back to index](README.md) · Built 2026-10-04 on main `bdc1f31` (steps 1–5, in order, the gate green after each). Where the code differs from the plan above, this page is the truth.

## What shipped

- **Step 1.** `general` and `appearance` sections, the General page (export folder with `Choose…` / `Use home folder`, TLS Secrets switch), Row density on Appearance. `file_export::start_export_with` reads `general.export_dir` (the `is_dir` check runs on the background executor) and writes `path.parent()` only after `Saved`. `issue_feeds::condition_plan(.., CertificateWatch)` turns the Secrets feed off (`FeedPlan::Off("off in Settings")`), and coverage says `Not checked: certificates (off in Settings).`.
- **Step 2.** `logs` and `terminal` sections, the Terminal & Shell and Logs pages (W2 order: General, Clusters, Appearance, Keyboard Shortcuts, Safety, Terminal & Shell, Logs, About). `ShellCommand` serializes lowercase. `POD_TAIL_LINES` is gone: `LogSettings::tail_lines()` (10…10,000) feeds the pod stream and the first member wave of a workload tab; late joiners keep 50. `TerminalSession::new(size, scrollback)` takes `terminal.scrollback_lines()` (1,000…10,000); `ShellTab` repaints on a settings change so the font size applies at once. `--screen settings-general|settings-logs|settings-terminal`.
- **Step 3.** Search (switcher rule: `cluster_matches`), drag inside a group (`DraggedCluster`, tint `drop_target` only in the same group, off while searching), `Alt ↑/↓` (`SettingsWindow && !Input`), `move_cluster` / `step_cluster`, six swatches (`ClusterColor`, theme tokens only) driving the title-bar border while the badge keeps the environment colour, hint and Reset tooltip.
- **Step 4.** `kube` features `http-proxy` and `socks5`; `ProxyChoice`, `ProxyUrl`, `ProxyUrlError`, `ClusterError::InvalidProxy`, `open(.., &ProxyChoice)`; `ClusterProxy` and `ClusterProfile.proxy: Result<ProxyChoice, ProxyUrlError>`; `open_cluster` is the only way the app opens a client (session start and Retry, switcher probes, Test connection); `ConnectionInfo.proxy`. The Connection row has the three modes, the URL input commits on Enter or blur, `Not applied` shows until the typed URL is stored, and the form shows `ProxyUrl::display()` only.
- **Step 5.** `registry.kubeconfig_folders`; `kubeconfig_folder.rs` (scan, diff, bounded read), the folder part of the catalog (`cluster_catalog_folders.rs`), `RowOrigin::Folder`, `Watch a kubeconfig folder…`, one line per folder with `Stop watching`, `Kubeconfig::parse_file`, `KubeconfigError::TooLarge`. Start rule: `ClusterCatalog::start_kubeconfigs()` (chain and registry only); a folder context starts only as the exact `last_used` (not with `--context`, never from `current-context` or the first file); otherwise `No cluster selected. Pick one in the switcher.` and the switcher opens. `ProbeCandidate.origin`: `HealthBoard::due` skips folder rows whatever their auth kind.

## Deviations from the plan

| Plan | Built | Why |
|---|---|---|
| `workspace.rs` gains `observe_global::<AppSettings>` | not added | `AppShell` already observes the global and notifies; `workspace.rs` only got `row_size(cx)` and `.with_size(..)` on the 4 `DataTable::new` sites (the plan said 7) |
| Proxy types in `connection.rs` | new `crates/cluster/src/proxy.rs` (+ `proxy_tests.rs`); `ProxyChoice::config_uri` replaces `proxy_for` | one concept per file; `connection_tests.rs` keeps the open and redaction tests |
| `FolderEvent { folder: usize }` | `FolderEvent { folder: PathBuf }` | an index shifts when a folder is added or stopped while an event is queued |
| Folder code in `cluster_catalog.rs` | child module `cluster_catalog_folders.rs`; `FolderStatus` and `FolderSummary` in `kubeconfig_folder.rs`; folder files live in the catalog's `standalone` list with a `PartSource` | the file stayed readable; `standalone_files(registered, folder_files, chain)` does the one dedupe |
| Per-type `x_label` / `x_from_label` | one `OptionTable<T>` (`label`, `value`, `choices`) in `settings.rs` | six tables, one pattern |
| Option labels | `Compact (28 px) (default)`, `Auto: bash, ash, sh (default)`, `Theme size (default)` | every default ends with ` (default)`; `Theme default (default)` read badly |
| Real `notify` in the catalog | `start_watcher` returns `None` under `cfg(test)` | the tests send events and use the fake clock |
| Redaction for `ProxyProtocolDisabled` and `Unsupported` | only `Unsupported` | with the features on, `Disabled` cannot occur |
| Tests inline in `kubeconfig_folder.rs` | `kubeconfig_folder_tests.rs` | house rule: `*_tests.rs` siblings |

Test names that moved: `density_change_rerenders_the_table` → `density_change_resizes_the_table_rows` (`workspace.rs`, the observer exists already); `shortcut_numbers_follow_the_new_order` is in `cluster_form_tests.rs` (it needs a kubeconfig fixture); `proxy_choice_sets_the_config` and the parse table are in `proxy_tests.rs`; the Alt arrow keys are tested as a keymap resolution (`alt_arrows_move_clusters_outside_text_fields`) and `step_selected`.

## Checks run

- Density at 28 and 36 px (Pods light and dark, Nodes, Issues, UAT read-only): the selection checkbox, the usage bars of Nodes, and the status text fit in both; no `DataTable` cell draws a pill. Shots: `.tmp/ui-shots/v97-*`. No cell needed a fix.
- Real `notify` (manual, fixture kubeconfig, no cluster): creating and deleting a file in a watched folder each caused exactly one rescan after the debounce (`rescanning watched folders`, a count only).
- Proxy: fakes only. `a_refusing_proxy_fails_closed_and_the_server_is_never_reached` (the error names the refused proxy address; a listener standing in for the API server saw no connection) and the `open_cluster` fail-closed test. UAT was never put behind a proxy.
- Read-only UAT (`readonly@Monitor`, direct): seven screenshot launches and the ignored live `Test connection` test (one `GET /version`). A launch sends about 90 requests through `ClusterConnection::run` (`sending request` in the `cluster` debug log: `reviewing access` 58, `reading kubelet stats` 4, `counting …` 18, version, pod and node metrics, leftover node shell pods); no writes.
- Port-forward tests that failed once under load: `stopping_a_forward_closes_its_listener` now uses a fixed free local port (another test's `Auto` forward could take the port it frees), and the `wait_for` loops of the app tests (this one, `app_shell_switch_tests`, `app_shell_write_tests`, `app_shell_values_edit_tests`, `debug_open_tests`, `shell_open_tests`, `app_shell_tests`) poll for 30 s instead of 10 s: they return at once on success, and a machine busy with other builds made a write test time out too.

## Known ceilings

- A missing watched folder is looked at again only at the next start.
- A settings change to `terminal.font_size` repaints open terminals through `observe_global`; the scrollback of an open tab is never changed.
- The search box cannot be typed into by the screenshot tool, so no shot shows a filtered list; the filter is covered by `cluster_search_matches_label_context_env_and_file` and `filter_groups_keeps_matching_rows_and_drops_empty_groups`.
