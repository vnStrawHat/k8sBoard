# 0025 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; every new item has a production user in its step (dead-code rule). Prerequisite: 0024 merged (all steps).

## Cargo

No change. `Cargo.lock` unchanged (kit `setting`, `switch`, `select`, `dialog` are in `gpui-component` already).

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/kubeconfig.rs` | `parse`, `connection_info`, `entry_names`; `ConnectionInfo`, `AuthKind` (+ `Display`), `EntryNames` |
| `src/kubeconfig_tests.rs` | tests in [test-plan.md](test-plan.md) |
| `src/lib.rs` | export `ConnectionInfo`, `AuthKind`, `EntryNames` |

`connection_info` and `entry_names` get their first production user in step 3/4; to keep step 1 free of dead code they are `pub` (library API, not flagged by `dead_code`).

## `crates/app`

| S | File | Change |
|---|---|---|
| 2a | `src/cluster_catalog.rs` (new, tests in module) | `ClusterCatalog` (`new`, `kubeconfigs`, `is_loading`, `notices`, `clear_notices`), `CatalogPart`, `CatalogHandle`, `CatalogNotice::Skipped`; standalone reload on registry change |
| 2a | `src/app_shell.rs` (+ tests) | `KubeconfigState`, load task, and notices move to the catalog; `_catalog_observer`; start choice on first load; `switch_cluster` reads the catalog |
| 2a | `src/title_bar.rs` | switcher and notices read the catalog (no visible change) |
| 2a | `src/main.rs` | `mod cluster_catalog;`; catalog global before the window |
| 2b | `src/settings_window.rs` (new) + `settings_window_tests.rs` | `OpenSettings`, `ImportKubeconfig`, `SettingsWindowHandle`, `open_settings_window`, `forget_closed_window`, `quit_when_main_window_closes`, `bind_keys`, `SettingsWindow` view (no `Debug`), Appearance and About pages |
| 2b | `src/settings.rs` (0024) | `ThemePreference::apply`, `THEME_OPTIONS`, `theme_label`, `theme_from_label`; `AppSettings::config_dir` |
| 2b | `src/title_bar.rs` | Settings button enabled, `tooltip_with_action(.., &OpenSettings, None)`; "Manage clusters…" enabled |
| 2b | `src/main.rs` | `mod settings_window;`; `on_action(OpenSettings)`; `bind_keys`; `quit_when_main_window_closes(main_id, \|cx\| cx.quit(), cx)`; `apply_theme` removed |
| 2b | `src/launch_options.rs` (+ tests), `src/screenshot.rs` (+ tests) | `--screen settings`, `settings-appearance`; capture the Settings window |
| 3 | `src/cluster_registry.rs` (0024, + tests) | `ClusterProfile.read_only` resolver (decision 17) |
| 3 | `src/cluster_catalog.rs` | `is_chain_source`, `PathStyle`, `same_path_text` (+ tests) |
| 3 | `src/cluster_form.rs` (new) + `cluster_form_tests.rs` | `ClusterRow`, `RowOrigin`, `ClusterGroup`, `cluster_groups`, `count_text`, `validate_display_name`, `validate_namespace`, `FieldError`, `reset_entry`, `TestState`, `test_connection`, `TEST_CONNECTION_TIMEOUT` |
| 3 | `src/settings_window.rs` | Clusters page: list, form, inputs, Test connection, Reset; `ClustersPageState` (no `Debug`) |
| 4 | `src/kubeconfig_import.rs` (new) + `kubeconfig_import_tests.rs` | `ImportPreview`, `ImportSource`, `ContextPreview`, `NameCollision`, `EntryKind`, `CollisionPlace`, `ImportError`, `name_collisions`, `check_new_file`, `pasted_file_path`, `write_pasted_kubeconfig`, `is_app_owned`, `PASTED_DIR` |
| 4 | `src/cluster_catalog.rs` | `chain`, `add_pasted`, `remove_kubeconfig`, `paste_status`, `reset_paste_status`, `PasteStatus`, `ClipboardFingerprint`, the other `CatalogNotice` variants, unregistered-file scan |
| 4 | `src/cluster_form.rs` | `remove_kubeconfig` (registry edit only) |
| 4 | `src/settings_window.rs` | add-cluster menu, `ImportKubeconfig` action, preview and paste dialogs, `paste_text`, Remove dialog, notices with actions |

## Docs (after merge)

- `docs/roadmap/inventory-shell.md`: S1 → Done; S2, S3 → Partial (no drag order, watch folder); S5 → Partial (Appearance, About; the rest with their owners); T10 → Done.
- `docs/roadmap/cross-cutting.md` C2: "never store tokens" now reads "settings never hold tokens; a pasted kubeconfig is a separate user-requested file in `<config>/kubeconfigs/`".
- 0024 `persisted-prefs.md` reserved table: owners of `appearance.density` and `registry.clusters[].color` → later.
- 0028 spec (draft, not edited here): the follow-ups at the end of [other-pages.md](other-pages.md), applied by the orchestrator.

## Read-only rule

The only cluster request is `server_version` (GET `/version`) in Test connection. No new mutating call; the 0001 read-only grep is unchanged. Local writes: `settings.json` (0024), files in `<config>/kubeconfigs/`, and their deletion on Remove (registered or unregistered app-owned files only).
