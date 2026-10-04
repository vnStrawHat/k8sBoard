# 0052 · Test plan

[Back to index](README.md)

## 1. Settings (`settings_tests.rs`)

| Test | Checks |
|---|---|
| `color_theme_round_trips` | `ZedOne` ↔ `"zed-one"`, `Default` ↔ `"default"` |
| `unknown_color_theme_loads_as_default` | `{"version":1,"theme":"dark","appearance":{"color_theme":"solarized","density":"comfortable"}}` parses (not `Corrupt`) to `Default`, and `theme`/`density` are kept |
| `default_file_is_minimal` (unchanged) | no `appearance` key for defaults |
| `settings_keys_are_the_allow_list` (updated) | gains `appearance.color_theme` |

## 2. Theme file (`color_theme_tests.rs`, pure)

| Test | Checks |
|---|---|
| `zed_one_has_one_light_and_one_dark_theme` | `zed_one_family()` is `Some`. The names are `One Light` / `One Dark`, and the modes are light / dark. The file has exactly 2 themes. |
| `zed_one_sets_every_required_token` | A `REQUIRED_KEYS: [&str; N]` list in the test (the kit keys of the theme-colors.md table, the 4 `*.foreground` and 12 `base.*` keys spelled out). Read the raw `serde_json::Value`, because `ThemeConfigColors`' base fields are private. Each key is present in both themes and passes `try_parse_color`. |
| `zed_one_sets_no_font_radius_or_shadow` | Every `font_size`, `font_family`, `mono_font_family`, `mono_font_size`, `radius`, `radius_lg`, and `shadow` is `None` (they would outlive a switch back to Default). |
| `zed_one_has_a_highlight_style` | `highlight` is `Some`, `editor_background` equals the theme's `background` value, and `syntax` resolves `"string"` and `"keyword"`. |

## 3. Apply (`color_theme_tests.rs`, `#[gpui_kit::test]`, `gpui_kit::init(cx)` first)

| Test | Checks |
|---|---|
| `zed_one_dark_applies_its_colours` | `ThemePreference::Dark.apply(ZedOne)`: `theme_name() == "One Dark"`, `is_dark()`, and `background` / `foreground` / `danger` / `blue_light` equal the file values |
| `mode_change_keeps_the_family` | then the kit's own `Theme::change(ThemeMode::Light, None, cx)` (no install): `"One Light"`, and `background` equals the One Light value |
| `default_restores_the_kit_theme` | `ZedOne` then `Dark.apply(Default)`: `theme_name() == "Default Dark"`, `background == ThemeColor::dark().background`. `font_family`, `mono_font_family`, and `radius` equal their values before Zed One was applied. |
| `table_active_is_the_selection_in_every_combination` | for {Default, ZedOne} × {Light, Dark}: `table_active == selection` |
| `highlight_follows_the_family` | the `highlight_theme.name` after One Dark is `"One Dark"`, and after Default Light it is `"Default Light"` |

## 4. Appearance page (`settings_window_tests.rs`)

- `appearance_page_has_theme_then_mode`: item titles in order `Theme`, `Mode`, `Row density`.
- `color_theme_labels_round_trip`: `COLOR_THEME_OPTIONS.value(label(v)) == v` for both values, and an unknown label gives `Default`.
- `changing_color_theme_saves_and_applies` (gpui): `change_color_theme("Zed One", cx)` sets `appearance.color_theme == ZedOne`, and the theme name becomes `One Light` or `One Dark` per the saved mode.

## 5. Launch (`launch_options_tests.rs`)

- `color_theme_accepts_default_and_zed_one`, and `color_theme_rejects_unknown` (the error names `--color-theme`).
- The no-flags test (today `assert_eq!(options.theme, None)`) also checks `color_theme == None`. USAGE contains `--color-theme`.

## 6. ui-verifier (live UAT, `readonly@Monitor`, read-only)

Run each shot with `--color-theme zed-one --theme dark`, and again with `--theme light`:

| Screen | Look for |
|---|---|
| `overview` | panels, cards, chart series (blue / green), title-bar env border |
| `pods`, `pods-selected` | row selection tint, status tones (Running / Pending / CrashLoop), row borders, header |
| `pod-drawer`, `pod-yaml` | drawer surface, YAML syntax colours (One palette, not the kit's) |
| `topology`, `topology-traffic-fixture` | 8 distinct kind hues, readable chips, warn/bad ghosts, legend |
| `node-monitor` | usage chart lines and grid |
| `logs-dock` | pod prefix colours (5 distinct), selection |
| `shell-fixture` | ANSI colours, cursor = accent |
| `settings-appearance` | the Theme and Mode dropdowns, labels; badge swatches |
| `switcher` | the PROD/STG badges (seeded registry), active row |

Then the same `overview`, `pods`, and `topology` with `--color-theme default` (light and dark): unchanged from main.

Compare against Zed's look by eye: dark editor `#282c33`, panels `#2f343e`, title bar `#3b414d`, blue accent. Report any text under 3:1 with its screen and token.
