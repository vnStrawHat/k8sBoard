# 0024 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own, and every new item has a production user in its step (dead-code rule).

## Cargo

| File | Change |
|---|---|
| root `Cargo.toml` | `[workspace.dependencies]` + `dirs = "6"` (already locked via `shellexpand`; no new package) |
| `crates/app/Cargo.toml` | + `serde.workspace = true`, `serde_json.workspace = true`, `dirs.workspace = true` (step 2) |

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/kubeconfig.rs` | `load(&[PathBuf]) -> LoadedKubeconfig` (merge with kind/apiVersion pre-check), new private `from_document` shape, `sources()`, `ContextSummary.source`, `Incompatible` variant, `paths` in `ContextNotFound`/`NoContextSelected` |
| `src/kubeconfig_tests.rs` | merge tests ([test-plan.md](test-plan.md)); helpers call `from_document(vec!["fixture.yaml".into()], document, &HashMap::new())`; `load_reports_missing_file_with_path` is replaced by `load_fails_when_no_file_loads` |
| `src/lib.rs` | export `LoadedKubeconfig` |
| `src/connection_tests.rs` (if it builds summaries) | add `source` |
| `examples/probe.rs` | `load(&[path])`, print skipped errors and `sources()` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 1 | `src/app_shell.rs` | compile against the new `load` (one path, `skipped` ignored until step 3) |
| 2 | `src/settings.rs` (new) + `settings_tests.rs` | `Settings`, `SETTINGS_VERSION`, `ThemePreference`; `AppSettings` global (`install`, `get`, `update`, `notice`, `dismiss_notice`, `flush`, writer task, quit hook) (+ `registry` field in step 3, `tables` in step 4) |
| 2 | `src/settings_store.rs` (new) + `settings_store_tests.rs` | `config_dir`, `default_config_dir`, `CONFIG_DIR_ENV`, `load_settings`, `LoadedSettings`, `WriteMode`, `SettingsNotice` (+ `Display`), `WriteGate`, `write_settings` (rename retry), `serialize_settings` |
| 2 | `src/launch_options.rs` (+ tests) | `--config-dir <path>`; `--theme system|light|dark` → `ThemePreference`; `ThemeChoice` removed; USAGE |
| 2 | `src/main.rs` | `mod settings; mod settings_store;`; resolve dir, `load_settings` before `application().run`; `writes = Disabled` with `--screenshot`; `AppSettings::install`; `apply_theme(ThemePreference)` |
| 2 | `src/title_bar.rs` | notices button (settings notice only in step 2) |
| 2 | `src/app_shell.rs`, `src/app_shell_tests.rs` | new field `_settings_observer: Subscription` = `cx.observe_global::<AppSettings>(..)`; tests `install` defaults with `WriteMode::Disabled` |
| 3 | `src/environment.rs` (new) + `environment_tests.rs` | `Environment`, `badge`, `guess_environment`, `environment_color`, `environment_badge` |
| 3 | `src/cluster_registry.rs` (new) + `cluster_registry_tests.rs` | `ClusterRegistry`, `ClusterRef`, `ClusterEntry`, `ClusterProfile`, `launch_last_used`, `profile`, `switcher_label`, `StartChoice`, `start_choice` |
| 3 | `src/settings.rs` | `registry` field |
| 3 | `src/launch_options.rs` (+ tests) | `kubeconfig_chain` and `standalone_files` replace `kubeconfig_path` and `has_ignored_kubeconfig_entries` |
| 3 | `src/app_shell.rs` (+ tests) | load the chain + standalone files (`Vec<Arc<Kubeconfig>>`), `start_choice`, `active`, `switch_cluster`, `last_used` write, kubeconfig notices; delete `IGNORED_KUBECONFIG_NOTE`, `kubeconfig_error_message` and its test |
| 3 | `src/title_bar.rs` | `TitleBar` `border_t` env color; badge + display name on the trigger; `switcher_label` items; kubeconfig notices in the button |
| 4 | `src/table_sort.rs` | `SortDirection`: `Serialize`, `Deserialize`, lowercase |
| 4 | `src/settings.rs` | `tables` field, `TablePrefs`, `SavedSort`, `screen_key` |
| 4 | `src/table_view.rs` (+ tests) | `apply_prefs`, `prefs` |
| 4 | `src/pod_table.rs`, `src/node_table.rs`, `src/kind_table.rs` (+ tests) | apply startup prefs at view creation |
| 4 | `src/app_shell.rs` | `persist_table_prefs` after `cycle_sort` and `toggle_column` |
| 5 | `src/resource_actions.rs` (+ tests) | `kind_menu(.., default_namespace)`; "Set as default namespace" for Namespaces |
| 5 | `src/cluster_registry.rs` (+ tests) | `entry_mut` (appends when missing), test `entry_mut_appends_once` |
| 5 | `src/app_shell.rs` | `toggle_default_namespace`; default namespace at start and in `switch_cluster` |

## Docs (after merge, by the orchestrator or architect)

- `CLAUDE.md` "Environment rules" and `.claude/agents/{coder-lite,ui-verifier}.md` run commands: add `--config-dir .tmp/config` (or `.tmp/config-<case>`) to every app launch (AC 12). The architect cannot edit these files (docs/ only).
- `docs/roadmap/cross-cutting.md`: C2 and C5 → settled by 0024 (decisions 3, 4, 24); C6 row "Config dirs and format" → `dirs` + JSON.
- `docs/roadmap/inventory-shell.md`: T4 Partial (single cluster), G4 Done, S2/S3 data model only.
- 0009 README decision 3, 0016, 0019, 0020 decision 11, 0022 decisions 23/27: point to the reserved keys in [persisted-prefs.md](persisted-prefs.md).

## Read-only rule

No new Kubernetes call. The 0001 read-only grep is unchanged. The only new writes are `settings.json`, `.tmp`, and `.bak` in the config dir.
