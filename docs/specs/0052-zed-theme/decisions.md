# 0052 · Decisions

[Back to index](README.md)

## 1. How the kit loads themes (gpui-component 0.7.0, verified)

| Fact | Source | Consequence |
|---|---|---|
| `Theme` holds `light_theme: Rc<ThemeConfig>` and `dark_theme: Rc<ThemeConfig>` (pub). `Theme::change(mode)` applies the config of the slot for `mode`, **even if the mode is unchanged** (`edit(.., reload_mode = true)`). | `theme/mod.rs` `change`, `edit` | Put both One configs in the slots, then call the existing `Theme::change` / `sync_system_appearance`. The mode switch then picks One Light or One Dark by itself. |
| `change` resets the slots to the registry defaults only when no `Theme` global exists yet. | `mod.rs:369` | Our slots survive later mode changes. Default must put the registry defaults back explicitly. |
| `ThemeColor::apply_config` sets **every** colour: from the file, else a derived fallback, else `ThemeColor::light()/dark()`. | `schema.rs:682` | No colour leaks from the previous theme. Only `background`, `border`, `foreground`, `muted`, `primary`, `secondary`, `overlay`, and `base.*` have no derived fallback. |
| `Theme::apply_config` sets fonts, radius, and shadow, and replaces `highlight_theme`, **only when the file has them**. | `schema.rs:1064` | The file must not set `font.*`, `mono_font.*`, `radius*`, or `shadow`, or Default would keep them. It **must** have `highlight`, or the YAML views keep the previous highlight. |
| `ThemeRegistry::load_themes_from_str` and `watch_dir` exist. The registry observer re-resolves the slots by name. | `registry.rs` | Not used. The file is parsed with `serde_json::from_str::<ThemeSet>` and never registered. Names absent from the registry are left alone by the observer. |
| `selection` is clamped to alpha 0.3 and `list_active`/`table_active` to 0.2. | `schema.rs` end of `apply_config` | Zed's selection `#74ade83d` (0.24) passes unchanged. |
| Highlight keys follow Zed's `style` (`editor.*`, status colours, `syntax`). Unknown keys are ignored. | `highlighter/registry.rs:439` | `highlight` is filled from Zed's `style`, filtered (theme-colors.md). |

`include_str!` works because the kit embeds its own `default-theme.json` the same way. It is preferred over a themes folder: nothing to install, nothing to watch, and a test can check it.

## 2. Settings shape

```rust
// color_theme.rs (new)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ColorTheme {
    /// One Light / One Dark, picked by the mode.
    ZedOne,
    /// The kit's own theme: today's look. Last because serde requires `other` on the last variant.
    #[default]
    #[serde(other)] // any unknown string loads as Default instead of resetting settings.json
    Default,
}
// settings.rs
pub(crate) struct AppearanceSettings { pub(crate) density: RowDensity, pub(crate) color_theme: ColorTheme }
```

- JSON: `"appearance": { "density": "compact", "color_theme": "zed-one" }`. The section is still omitted when it is all defaults. No version bump.
- Lenient parsing: `#[serde(other)]` maps any unknown string to `Default`. The architect checked this with serde 1 in a scratch crate: `"solarized"` gives `Default`, and the round trip gives `"zed-one"` / `"default"`. A non-string value is still a corrupt file, as for every other enum.
- `theme` (System/Light/Dark) is unchanged and stays top-level.

## 3. Apply flow

```rust
impl ColorTheme {
    /// Puts this family's light and dark configs in the theme slots; the next `Theme::change` loads one.
    pub(crate) fn install(self, cx: &mut App);
}
const ZED_ONE: &str = include_str!("../themes/zed-one.json");
/// One Light and One Dark; `None` only if the embedded file is broken (a unit test guards it).
fn zed_one_family() -> Option<ThemeFamily>;
struct ThemeFamily { light: ThemeConfig, dark: ThemeConfig }

impl ThemePreference {
    pub(crate) fn apply(self, colors: ColorTheme, cx: &mut App); // was apply(self, cx)
}
```

- `install`, for `Default`: copy `ThemeRegistry::global(cx).default_light_theme()` and `default_dark_theme()` into the slots. For `ZedOne`: `zed_one_family()` wrapped in `Rc`. If it returns `None`, log `tracing::warn!("the embedded Zed One theme does not parse")` and install Default.
- Write the slots with `Theme::global_mut(cx)`. The following `Theme::change` does `sync_base` and refreshes the windows, so there is one refresh.
- `ThemePreference::apply` calls `colors.install(cx)`, then today's match (`change` / `sync_system_appearance`), then today's `table_active = selection` patch.
- `zed_one_family` picks the themes by `mode` (one light, one dark). Parsing on each apply is cheap: it runs at start and on a dropdown change only.
- Callers: `main.rs` (start), `change_theme` (Mode dropdown), and the new `change_color_theme` (Theme dropdown). Each passes the other half from `AppSettings::get(cx)` or the launch override.

## 4. Appearance page UX

| Item | Control | Options (label = key) | On change |
|---|---|---|---|
| **Theme** (new) | dropdown, `COLOR_THEME_OPTIONS: OptionTable<ColorTheme>` | `Default`, `Zed One` | save `appearance.color_theme`, then `settings.theme.apply(new, cx)` |
| **Mode** (renamed from "Theme") | the existing dropdown | `Follow the system`, `Light`, `Dark` | save `theme`, then `theme.apply(settings.appearance.color_theme, cx)` |

- Group "Theme". Group description: "Applies to every window at once." The Theme item's description: "Zed One is One Light or One Dark, following Mode."
- Both items use `SettingField::dropdown` directly, not `table_dropdown`, because they must also re-apply the theme. The Theme item uses the `OptionTable` for `choices` / `label` / `value`. The unlisted closure is never reached (every value is listed) and returns `"Default"`.

## 5. Launch and screenshots

- `--color-theme default|zed-one` → `LaunchOptions.color_theme: Option<ColorTheme>`, parsed like `--theme`, with an error naming the flag. It is never saved. USAGE line: `--color-theme default|zed-one  colour family (default: the saved one, else Default)`.
- `main.rs`: `options.theme.unwrap_or(saved.theme).apply(options.color_theme.unwrap_or(saved.appearance.color_theme), cx)`.
- Why a flag instead of a seeded `--config-dir` file: shots keep the user's real registry (environments, PROD badge), so Default and Zed One compare like for like.

## 6. Licence and source

- Source: Zed `assets/themes/one/one.json` at commit `a3f6ef252b6de19d22a1223952fc335253163642` (2026-07-17): <https://github.com/zed-industries/zed/blob/a3f6ef252b6de19d22a1223952fc335253163642/assets/themes/one/one.json>.
- That folder has its own `LICENSE`: **MIT, Copyright (c) 2014 GitHub Inc.** This is Atom's One theme lineage. Zed's GPL/AGPL code licence does not apply to it.
- We take colour values only, rewritten in the kit schema. No Zed code is used. The `highlight.syntax` map is the one block copied nearly verbatim, so the file keeps the notice. The kit ignores unknown keys, so these can go in the file:
  - `"author": "Zed Industries; colours from Atom One"`
  - `"url"`: the pinned link above
  - `"_license": "MIT, Copyright (c) 2014 GitHub Inc. (zed assets/themes/one/LICENSE)"`
