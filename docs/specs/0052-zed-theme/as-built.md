# 0052 · As built

[Back to index](README.md)

Implemented in one code commit (steps 1 and 2 together; the gate was green for the whole change). Open items resolved by the user: the STG badge keeps Zed's warning value, and the dropdown label is `Zed One`.

## Deviations

- **No `appearance_page_has_theme_then_mode` test.** `SettingPage`, `SettingGroup`, and `SettingItem` keep their title fields `pub(super)` in gpui-component 0.7, so a test cannot read the item order. The order was checked on the `settings-appearance` screenshots instead (Theme, then Mode, then Row density).
- **Mode item has no description.** The spec gave one only to the Theme item and the group.
- **Highlight values keep Zed's `ff` alpha** (`#282c33ff`), copied verbatim from `one.json` by a throwaway script. The `colors` block uses the 6/8-digit values of theme-colors.md.
- **Two more struct initialisers** gained `color_theme: None` (`resource_actions_tests.rs`, `secret_values_tests.rs`) because `LaunchOptions` has a new field.
- **Settings shot shows the saved value.** `--color-theme` is never saved, so on `settings-appearance --color-theme zed-one` the Theme dropdown still reads `Default` while the window is themed One Dark/Light. The Mode dropdown behaves the same way with `--theme` today.

## Finding for the architect (no code change made)

- **One Light status tones lose their hue.** `status_tone::tone_color` mixes the light-mode tone 0.6 to 0.7 toward the foreground. With One Light's muted `success` (`#669f59`) and foreground (`#242529`), `Running` renders as a dark grey-green that reads as plain text (contrast is fine, about 5.6:1, but the green is barely visible). Default light shows a clear green. The same applies to the drain dialog result lines ("Will be rescheduled") and the Ready counts. Dark is fine. Options: a more saturated One Light `success`/`warning`/`danger` for the tone path, or a lower mix for this family. Not done, per the spec's "report back" rule.
