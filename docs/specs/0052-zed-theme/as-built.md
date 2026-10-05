# 0052 · As built

[Back to index](README.md)

Implemented in one code commit (steps 1 and 2 together; the gate was green for the whole change). Open items resolved by the user: the STG badge keeps Zed's warning value, and the dropdown label is `Zed One`.

## Deviations

- **No `appearance_page_has_theme_then_mode` test.** `SettingPage`, `SettingGroup`, and `SettingItem` keep their title fields `pub(super)` in gpui-component 0.7, so a test cannot read the item order. The order was checked on the `settings-appearance` screenshots instead (Theme, then Mode, then Row density).
- **Mode item has no description.** The spec gave one only to the Theme item and the group.
- **Highlight values keep Zed's `ff` alpha** (`#282c33ff`), copied verbatim from `one.json` by a throwaway script. The `colors` block uses the 6/8-digit values of theme-colors.md.
- **Two more struct initialisers** gained `color_theme: None` (`resource_actions_tests.rs`, `secret_values_tests.rs`) because `LaunchOptions` has a new field.
- **Settings shot shows the saved value.** `--color-theme` is never saved, so on `settings-appearance --color-theme zed-one` the Theme dropdown still reads `Default` while the window is themed One Dark/Light. The Mode dropdown behaves the same way with `--theme` today.

## Follow-up: light-mode status tones (fixed)

The first build showed One Light's `Running` and drain result lines as plain dark text, because `status_tone::tone_color` mixed every light-mode tone a fixed 40% (30% for amber) toward the foreground. That ratio suits Default's bright green but flattens a muted fill such as One Light's `success`.

- `readable_on_light` now bisects for the smallest move toward the foreground that reaches 4.5:1 (WCAG) on the theme background, capped at the old 40% / 30%. A tone that already reads is returned unchanged. There is no per-theme branch.
- `tone_color` is the only caller, and every status text, the drain dialog result lines, the traffic tone label, and the topology warn/bad edges and ghosts read it, so all of them follow. `contrast` moved from `topology_colors.rs` to `status_tone.rs` (still one definition) so both use it.
- One Light: `Running`, "Will be rescheduled", and the dry-run line keep a visible green, and the 5xx edges and CrashLoop node are clearly red instead of brown. The muted green still reads olive, as 4.5:1 on `#fafafa` requires a dark green.
- Default light: success and info are slightly lighter than before; amber is unchanged (it hits the cap and stays below 4.5:1, as before); danger is visibly brighter (lightness 0.36 to 0.54), because Default's red already reached 4.5:1 with less darkening. The test bounds the lightness change at 0.2 and requires at least the old contrast.
- Dark mode is untouched.

## Default changed (2026-10-05)

At the user's request the default colour family is now `zed-one`: new installs, settings files without `appearance.color_theme`, and unknown values (the `#[serde(other)]` variant is now `ZedOne`) get Zed One. An explicit saved `"default"` stays Default. This supersedes the "Default" fallbacks above and in decisions.md.
