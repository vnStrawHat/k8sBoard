# 0052 — Zed One colour theme

Status: draft (architect, 2026-10-04). Crate: `crates/app` only, no dependency change. User request (2026-10-04, Vietnamese): "I want a theme like Zed". Read as: add Zed's default theme family, **One Dark / One Light**, as a selectable colour theme. Today's look stays as **Default**. Rules: theme tokens only, English, no Kubernetes change.

## Goal

- New setting `appearance.color_theme`: `"default"` (absent) or `"zed-one"`. It is orthogonal to the existing mode `theme: system|light|dark`. Zed One + Light = One Light, and Zed One + Dark = One Dark.
- One embedded kit `ThemeSet` file, `crates/app/themes/zed-one.json`, holds both themes in the gpui-component 0.7 schema. Its colours come from Zed's `assets/themes/one/one.json` ([theme-colors.md](theme-colors.md)).
- Settings › Appearance gets a **Theme** dropdown (`Default`, `Zed One`). The old dropdown is renamed **Mode**. Both apply to every window at once.
- `--color-theme default|zed-one` overrides the setting for one run, as `--theme` does. Screenshots use it.

## Non-goals (possible follow-ups)

- Zed fonts (Zed Plex Sans/Mono), radius, or density. The theme file sets no `font.*`, `mono_font.*`, `radius*`, or `shadow`. The monospace font is Lilex, built into the app (`crates/app/fonts/`, SIL OFL 1.1 in `OFL.txt`) and set as `mono_font_family` by `mono_font.rs` after the kit init, so a theme change keeps it.
- Other Zed themes (Ayu, Gruvbox, and so on) and a theme picker that lists the kit registry.
- User-supplied theme files, and `ThemeRegistry::watch_dir`.
- Following the OS appearance live while `System` is set. This does not happen today either.

## Files in this spec

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | settings shape, UX, how the kit loads and switches themes, apply flow, launch flag, licence |
| [theme-colors.md](theme-colors.md) | every kit token with its One Dark / One Light value and Zed source key; contrast table; gaps |
| [files-to-touch.md](files-to-touch.md) | coder steps and the files each touches |
| [test-plan.md](test-plan.md) | unit, gpui, and ui-verifier checks |

Also changed: [0024 persisted-prefs.md](../0024-settings-store/persisted-prefs.md), with the `appearance.color_theme` row.

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]` and no `Cargo.toml` change.
- [ ] 2. The 0003 colour-literal grep of `crates/app/src` stays clean: hex values live only in `crates/app/themes/zed-one.json`.
- [ ] 3. The setting round-trips. An unknown string value loads as `Default` without resetting the file. A default file has no `appearance` key.
- [ ] 4. The theme file parses into exactly one light and one dark theme. Every required token (theme-colors.md) is present in both and parses. No font, radius, or shadow key, and `highlight` is present.
- [ ] 5. gpui tests: applying Zed One + Dark sets One Dark's colours, and a mode change keeps the family. Switching back to Default restores the kit colours, fonts, and radius. `table_active == selection` holds in every combination.
- [ ] 6. The Appearance page shows Theme and Mode. Changing either re-themes the main window and the Settings window at once, and saves.
- [ ] 7. ui-verifier screenshots in One Dark and One Light (test-plan.md list): no unreadable text, status tones and PROD/STG badges legible, Topology kind colours distinct, YAML highlighted, and the terminal palette readable. Default light and dark shots are unchanged.

## Open items

1. One Light's `warning` (`#a48819`) as a badge fill under `#fafafa` text is 3.3:1. That is better than Default light (1.9:1) but below 4.5:1. It is kept to stay faithful to Zed. The user may ask for a darker amber.
2. Label wording: `Zed One` (chosen) or `One (Zed)`.
