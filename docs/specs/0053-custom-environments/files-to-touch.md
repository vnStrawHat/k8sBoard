# 0053 · Files to touch and steps

[Back to index](README.md) · One coder, three steps in order; the quality gate (plus the `screenshot` clippy) after each. Paths under `crates/app/src/`. Step 4 is the ui-verifier run.

## Step 1 — remove the per-cluster colour

| File | Change |
|---|---|
| `environment.rs` | Rename `ClusterColor` → `EnvironmentColor`, `cluster_color` → `palette_color`. Drop `ALL` and `name()` (unused now; step 3 brings them back). Keep `of(environment)` (renamed in step 2). Update the module doc (no "title-bar top border"). |
| `cluster_registry.rs` | Drop `ClusterEntry.color`, `ClusterProfile.color`, the `entry_mut` and `profile()` lines for it. |
| `cluster_form.rs` | Drop `color_to_store` and the `entry.color.is_some()` line of `edit_entry`. |
| `clusters_page.rs` | Drop the `Color` `form_row`, `set_cluster_color`, `color_swatches`, and their imports; General = name, environment, namespace. |
| `write_guard.rs` | `test_guard`: drop `color`. |
| `settings_tests.rs` | Allow-list loses `registry.clusters.color`; add `old_cluster_color_is_ignored` (test-plan §1). |
| `cluster_form_tests.rs`, `cluster_registry_tests.rs`, `clusters_page_tests.rs`, `environment_tests.rs`, `cluster_switcher_rows_tests.rs` | Delete the colour tests named in test-plan §1; drop `color:` from profile fixtures. |

## Step 2 — data model, persistence, every call site

| File | Change |
|---|---|
| `environment.rs` | [model.md](model.md) types: `EnvironmentTier` (old enum), `EnvironmentColor` (+`Green` → `success`), `CustomEnvironment`, `EnvironmentKey`, `Environment` + consts, `resolve_environment`, `EnvironmentTier::color` (was `ClusterColor::of`), `environment_color`/`environment_badge` take `&Environment`. `guess_environment` returns `EnvironmentTier`. |
| `cluster_registry.rs` | `registry.environments`; `entry.environment: Option<EnvironmentKey>`; `profile()` per model.md "Resolution" (tier match on `environment.tier()`). |
| `write_guard.rs` | `ConfirmMode::for_tier`; `test_guard` takes `Environment`. |
| `cluster_form.rs` | `ClusterGroup.title: SharedString`, `group_index(&Environment, &[CustomEnvironment])`, custom group titles, `ClusterRow.guessed: EnvironmentTier`, `cluster_matches` passes `&row.profile.environment`. |
| `clusters_page.rs` | `environment_menu`: built-ins via `EnvironmentTier::ALL`, separator, custom items `"{name} · like {tier}"`, values `Option<EnvironmentKey>`, label = resolved `profile.environment.name()` when set. `confirm_menu`: `for_tier(..tier())`. `DraggedCluster.group_title: SharedString`. Badge `&`. |
| `cluster_switcher_rows.rs` | `SwitcherSection.title: SharedString`; `row.profile.environment.clone()`; `search_text(.., &Environment, ..)`. |
| `cluster_switcher.rs`, `title_bar.rs`, `port_forward_page.rs` | `environment_badge(&..)`, `environment_color(&..)`. |
| `clusters_page_import.rs`, `kubeconfig_import.rs` | `ContextPreview.environment: EnvironmentTier`; badge of `Environment::BuiltIn(..)`. |
| `command_palette.rs` | `environment: Environment` fields cloned; badge `&`; fixture `Environment::DEVELOPMENT`. |
| `confirm_dialog.rs`, `drain_dialog.rs` | fields stay `Environment` (cloned in); `fn environment(&self) -> &Environment`; badge `&`; drain fixture consts + `for_tier`. |
| `port_forwards.rs`, `port_forward_dialogs.rs`, `port_forward_open.rs` | clone instead of copy (`describe` closure, `origin.environment`); fixture consts; the `(label, Environment)` fallback uses `Environment::STAGING`. |
| `write_flow.rs`, `batch_write.rs`, `write_lock.rs`, `app_shell.rs` (4236) | `guard.profile.environment.clone()` / `profile.environment.clone()`. |
| `resource_edit_flow.rs`, `object_delete.rs`, `node_shell_open.rs`, `shell_open.rs`, `certificate_renewal.rs`, `screenshot.rs`, `app_shell.rs` (1988) | fixtures: consts; `ConfirmMode::for_tier(environment.tier())`; `node_shell_open` matches on `environment.tier()`. |
| `settings_window.rs` | `tier_rows(custom: &[CustomEnvironment])` from `AppSettings::get(cx).registry.environments`; built-ins via `EnvironmentTier::ALL`. |
| `*_tests.rs` (all users of `Environment::X`) | `Environment::Production` → `Environment::PRODUCTION` (or `EnvironmentTier::Production` where a tier is meant: guess tests, `for_tier`). |
| `settings_tests.rs` | allow-list + `full_settings()` with one custom environment (test-plan §2). |

`grep -rn "Environment::" crates/app/src | wc -l` was 166 at d34b896; after step 2 no `Environment::Production`-style variant path remains outside `EnvironmentTier::`.

## Step 3 — Environments page

| File | Change |
|---|---|
| `environment.rs` | `EnvironmentColor::ALL` (7) and `name()` back. |
| `environment_form.rs` (new) + `environment_form_tests.rs` | [environments-page.md](environments-page.md) "Pure functions". |
| `environments_page.rs` (new) + `environments_page_tests.rs` | the view; swatch row moved from step 1's deleted code. |
| `main.rs` | `mod environment_form; mod environments_page;` |
| `settings_window.rs` | `SettingsPage::Environments` (title, icon `Tag`, after Clusters in `PAGES`, 10), `environments_page(..)` builder, entity in `SettingsWindow`. `tier_cell` becomes `pub(crate)` if the page reuses it for built-in rows. |
| `launch_options.rs` (+ tests) | `settings-environments` screen and USAGE. |
| `settings_window_tests.rs` | page order test updated. |

## Docs (architect, done in this spec's commit)

- `docs/specs/0024-settings-store/persisted-prefs.md`: `environment` value, `registry.environments` row, `color` removed.
- `docs/specs/0043-settings-pages/clusters-list.md`: Colour section marked removed by 0053.

## Guardrails

- No hex, `rgb(`, or `hsla(` in `crates/app/src`; colours only via `palette_color`.
- No `unwrap`/`expect` outside tests. No `#[allow(dead_code)]`: declare only the consts and methods that have a caller in that step.
- No change under `crates/cluster`; no Kubernetes call added.
