# 0053 · Test plan

[Back to index](README.md) · Each test checks one behaviour, offline, writing only under `std::env::temp_dir()`.

## 1. Step 1 — colour removed

| File | Test | Checks |
|---|---|---|
| `settings_tests.rs` | `old_cluster_color_is_ignored` | a file whose entry has `"color": "teal"` and `"display_name": "x"` loads with no notice (not `Corrupt`), the entry keeps `display_name`, and `serialize_settings` output has no `"color"` |
| `settings_tests.rs` | `settings_keys_are_the_allow_list` (updated) | no `registry.clusters.color` |
| `environment_tests.rs` | `cluster_color_uses_theme_tokens` → `palette_color_uses_theme_tokens`, `cluster_color_serializes_lowercase` → `environment_color_serializes_lowercase` | renamed only (still six colours here) |

Deleted: `a_color_equal_to_the_environment_stores_nothing`, `a_stored_color_keeps_its_entry` (cluster_form), `profile_color_prefers_the_entry`, `a_color_follows_the_environment_when_none_is_stored` (cluster_registry), `picking_the_environment_colour_stores_none` (clusters_page). `cluster_color_defaults_to_the_environment` stays (renamed `environment_color_defaults_to_the_tier`) minus its `ALL`/`name` asserts, which return in step 3; step 2 turns it into `tier_colors`.

## 2. Step 2 — model and call sites

`environment_tests.rs`:

| Test | Checks |
|---|---|
| guess tests (existing) | unchanged tables, now `EnvironmentTier` |
| `tier_colors` | Production Red, Staging Amber, Development Blue, Local Gray |
| `environment_key_round_trips` | `BuiltIn(Production)` ↔ `"production"`, `Custom("QA")` ↔ `"QA"`; `"Production"` parses as `Custom` |
| `custom_environment_round_trips` | `{"name":"QA","color":"green","tier":"staging"}` ↔ struct |
| `resolve_finds_custom_by_name` | key `Custom("QA")` with a `QA` entry → `Custom(..)`, tier and colour from the entry |
| `missing_custom_resolves_to_production` | key `Custom("Gone")`, empty list → `Environment::PRODUCTION` |
| `custom_badge_is_upper_case_name` | `Pre-prod` → `PRE-PROD`; a built-in badge is unchanged |
| `palette_color_uses_theme_tokens` (gpui) | seven colours, Green = `theme.success` |
| `environment_color_follows_custom_color` (gpui) | custom Teal → `theme.cyan` |

`cluster_registry_tests.rs`:

| Test | Checks |
|---|---|
| `production_tier_custom_gets_production_defaults` | `read_only`, `TypeName`, node shell off |
| `staging_tier_custom_counts_as_set` | unlocked, `Click`, node shell **on** (set explicitly) |
| `missing_custom_is_locked` | profile is Production, `read_only`, `TypeName` |
| `stored_overrides_beat_the_tier` | a custom Production entry with `read_only: false`, `confirm: click` keeps them |
| `old_builtin_entry_loads_unchanged` | `"environment": "production"` → `BuiltIn(Production)` |

Other files:

| File | Test | Checks |
|---|---|---|
| `write_guard_tests.rs` | `confirm_mode_follows_tier` (renamed) | only Production types the name |
| `cluster_form_tests.rs` | `custom_groups_follow_builtins_in_list_order` | groups: Production, Staging, Development · Local, then `QA`, `DR` in `environments` order; empty ones dropped |
| `cluster_form_tests.rs` | `filter_matches_custom_badge` | query `qa` keeps the `QA` row |
| `cluster_switcher_rows_tests.rs` | `sections_include_custom_groups` | section titles carry the custom name; shortcuts run on across them |
| `settings_window_tests.rs` | `tier_rows_list_custom_environments` | `DR` (Production tier) in the type-name row, `QA` (Staging) in the click row, after the built-ins |
| `clusters_page_tests.rs` (gpui) | `picking_a_custom_environment_stores_its_name` | the menu value `Custom("QA")` is stored; the row moves to the `QA` group |
| `settings_tests.rs` | allow-list (updated) | gains `registry.environments`, `.name`, `.color`, `.tier`; `full_settings()` has one custom environment and round-trips |
| `settings_tests.rs` | `default_file_is_minimal` (unchanged) | no `environments` key |

## 3. Step 3 — `environment_form_tests.rs` (pure)

| Test | Checks |
|---|---|
| `name_rules` | table: `""`, `"   "`, 17 chars, `"a\tb"`, `"production"`, `"PROD"`, `"Auto"`, `"Development · Local"`, `"qa"` with `QA` taken → each message of environments-page.md; `" QA "` → `Ok("QA")` |
| `rename_to_other_case_is_allowed` | `own = Some("qa")`, text `QA` → `Ok` |
| `add_uses_purple_and_production` | appended last |
| `rename_rewrites_references` | two entries on `QA` become `QA2`; a built-in entry is untouched |
| `delete_moves_clusters_to_the_tier` | entries on `QA` (Staging tier) become `BuiltIn(Staging)`; their profiles' `confirm`/`read_only` equal those before the delete |
| `step_environment_moves_and_stops_at_ends` | up/down swap; up at 0 and down at last are no-ops |
| `clusters_using_counts_unloaded_entries` | counts entries whatever their kubeconfig |
| `delete_dialog_text_counts` | 0, 1, 3 → the three bodies |

## 4. Step 3 — gpui and launch

| File | Test | Checks |
|---|---|---|
| `environments_page_tests.rs` | `add_saves_and_clears_the_input` | settings gain the environment, input empty, no error |
| | `invalid_rename_keeps_the_stored_name` | typing `PROD` shows the reserved message; settings unchanged |
| | `valid_rename_follows_in_entries` | typing `QA2` renames and rewrites the entry |
| | `swatch_and_tier_save` | `edit_environment` via the click handlers sets colour and tier |
| `settings_window_tests.rs` | page order (updated) | `PAGES[2] == Environments`, 10 pages, titles listed |
| `launch_options_tests.rs` | `settings_environments_screen` | parses to `Settings(Environments, Standard)` |

## 5. ui-verifier (step 4)

Seed `.tmp/config-0053/settings.json` with `QA` (Teal, Staging), `DR` (Red, Production), and `readonly@Monitor` set to `QA`. Run with `--config-dir .tmp/config-0053`, `--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor`. No write is attempted.

1. `--screen settings`: no `Color` row; `QA` group after the built-ins; dropdown shows `QA · like Staging`.
2. `--screen settings-environments` (Default light, Default dark, `--color-theme zed-one --theme dark`): built-in rows, two custom rows, swatches, counts, add row; badges legible.
3. Main window: title-bar `QA` badge in teal; switcher `QA` section; the Unlocked frame (toggle the lock) is teal.
4. Safety page (click it in the sidebar): `DR` beside Production, `QA` beside the click tiers.
