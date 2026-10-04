# 0052 · Files to touch and steps

[Back to index](README.md) · One coder, two steps, in order. Run the quality gate after each step.

## Step 1 — theme file, model, apply (no UI)

| File | Change |
|---|---|
| `crates/app/themes/zed-one.json` (new) | `ThemeSet` per [theme-colors.md](theme-colors.md): `name`, `author`, `url`, `_license`, and two themes. Each theme has `colors` (the required rows) and `highlight` (generated). No `font.*`, `mono_font.*`, `radius*`, or `shadow`. |
| `crates/app/src/color_theme.rs` (new) | `ColorTheme` (decisions §2), `ZED_ONE` via `include_str!("../themes/zed-one.json")`, `ThemeFamily`, `zed_one_family()`, and `ColorTheme::install` (decisions §3). Module doc: one line on what it owns. |
| `crates/app/src/color_theme_tests.rs` (new) | the file and gpui tests in [test-plan.md](test-plan.md) §2–3 (`#[path]` sibling, like `settings_tests.rs`) |
| `crates/app/src/main.rs` | `mod color_theme;`. The start-up apply passes the colour theme (decisions §5); the launch override arrives in step 2, so use the saved value until then. |
| `crates/app/src/settings.rs` | `AppearanceSettings.color_theme: ColorTheme`. `ThemePreference::apply(self, colors: ColorTheme, cx)` calls `colors.install(cx)` first. Update the `AppearanceSettings` doc ("what the Mode dropdown does not cover" → it now covers both the colour theme and density). |
| `crates/app/src/settings_window.rs` | `change_theme` passes `AppSettings::get(cx).appearance.color_theme` (behaviour unchanged in step 1) |
| `crates/app/src/settings_tests.rs` | test-plan §1. `full_settings()` sets `color_theme: ZedOne`. The allow-list gains `appearance.color_theme`. |

## Step 2 — Appearance page, launch flag, screenshots

| File | Change |
|---|---|
| `crates/app/src/settings.rs` | `COLOR_THEME_OPTIONS: OptionTable<ColorTheme>` = `[(Default, "Default"), (ZedOne, "Zed One")]`, default `Default` |
| `crates/app/src/settings_window.rs` | `appearance_page`: the Theme item (new, first) and the Mode item (renamed), with the group description (decisions §4). `change_color_theme(label, cx)`: save, then `settings.theme.apply(color_theme, cx)`. |
| `crates/app/src/settings_window_tests.rs` | test-plan §4 |
| `crates/app/src/launch_options.rs` | `color_theme: Option<ColorTheme>`, `--color-theme`, `parse_color_theme`, and the USAGE line under `--theme` |
| `crates/app/src/launch_options_tests.rs` | test-plan §5. Update the full-options fixture. |
| `crates/app/src/main.rs` | `options.color_theme.unwrap_or(saved)` |

Not touched: `status_tone.rs`, `topology_colors.rs`, `usage_chart.rs`, `environment.rs`, `terminal_session.rs`, `log_legend.rs`. They read tokens and need no change. If a ui-verifier shot shows a contrast defect, report it back to the architect rather than adding a per-theme branch.

## Docs (architect, done in this spec's commit)

- `docs/specs/0024-settings-store/persisted-prefs.md`: the reserved-sections row for `appearance.color_theme`.

## Guardrails

- No hex, `rgb(`, or `hsla(` in `crates/app/src`. Tests compare against values parsed from the embedded file (`try_parse_color(&config.colors.background…)`) or against `ThemeColor::dark()`/`light()`.
- No `unwrap`/`expect` outside tests. A broken embedded file falls back to Default with one `tracing::warn!` and no content.
- Do not register the themes in `ThemeRegistry`. Do not call `watch_dir`.
