# 0025 · Appearance, About, keys, and later pages

[Back to index](README.md) · Step 2b · Module: `settings_window.rs`. Decisions 3, 6, 21, 22.

## Page order

W2 nav order, keeping only pages with content. A later spec inserts its page at its W2 position and adds a row to `settings_window_tests::pages_follow_w2_order`.

| W2 nav | 0025 | Owner of the page |
|---|---|---|
| General | omitted | the first spec with a General field (0016 or 0020, below) |
| **Clusters** | yes, default page | 0025 ([clusters-page.md](clusters-page.md)) |
| **Appearance** | yes (theme) | 0025; density joins later |
| Keyboard Shortcuts | omitted | **0028**, inserted at W2 position 4 (after Appearance) |
| Safety | omitted | 0030 (confirm modes, lock defaults) |
| Terminal & Shell | omitted | 0036 |
| Logs | omitted | 0019 |
| Metrics | omitted | backlog (Prometheus) |
| Extensions | omitted | backlog |
| **About** | yes | 0025 |

## Keys (decision 3)

0025 lands before 0028, so it binds its own two keys in `settings_window::bind_keys`:

| Binding | Action | Context | Note |
|---|---|---|---|
| `secondary-,` | `OpenSettings` | none | also fires while a dialog or text input has focus; acceptable: it only opens or activates Settings, and no input uses Ctrl , |
| `secondary-o` | `ImportKubeconfig` | `SettingsWindow` | only in the Settings window |

When 0028 lands it moves both bindings into its `keymap.rs`, deletes `settings_window::bind_keys`, and adds the Keyboard Shortcuts page. Follow-ups for 0028 are listed at the end of this file.

## Appearance

`SettingPage::new("Appearance").resettable(false)`, group "Theme":

```rust
/// The one table: dropdown key = label, so there is no second string table.
const THEME_OPTIONS: [(ThemePreference, &str); 3] =
    [(ThemePreference::System, "Follow the system"), (ThemePreference::Light, "Light"), (ThemePreference::Dark, "Dark")];
fn theme_label(theme: ThemePreference) -> &'static str;   // lookup in THEME_OPTIONS
fn theme_from_label(label: &str) -> ThemePreference;      // unknown → System
SettingItem::new("Theme", SettingField::dropdown(
    THEME_OPTIONS.iter().map(|(_, label)| (label.into(), label.into())).collect(),
    |cx| theme_label(AppSettings::get(cx).theme).into(),
    |label, cx| { let theme = theme_from_label(&label); AppSettings::update(cx, |s| s.theme = theme); theme.apply(cx); },
)).description("Applies to every window at once.")
```

Both helpers live next to `ThemePreference` in `settings.rs`. A `--theme` flag on the command line is not stored; the dropdown shows the stored value.

## About

`SettingPage::new("About").resettable(false)`, group without title, `SettingItem::render` rows:

| Row | Content |
|---|---|
| Version | `k8sBoard {env!("CARGO_PKG_VERSION")}` |
| License | `Apache-2.0` |
| Settings folder | the config dir path (mono) + ghost button `Show in folder` → `cx.reveal_path(dir)`; "Not saved this session" when `AppSettings::config_dir` is `None` |
| Settings notice | the 0024 notice text when present (theme warning) |

## Reserved keys and fields not rendered in 0025

The owner spec renders each one; 0025 renders none of them because none of these specs has merged (checked at HEAD `9d5af01`).

| Key (0024 persisted-prefs.md) | Page · field | Owner |
|---|---|---|
| `secrets.clipboard_clear_seconds` | General · Clear copied secrets after | 0016 |
| `issues.watch_tls_secrets` | General · Watch TLS Secrets for expiry | 0020 |
| `logs.export_dir` | Logs · Export folder | 0019 |
| `topology.group_by` | none (kept by the Topology toolbar) | 0022 |
| `appearance.density` | Appearance · Row density 28/36 | later 0025 follow-up (needs a table row-height audit) |
| `dock.height` | none (drag only) | 0019 or later |
| `registry.clusters[].color` | Clusters · Color swatches | later (env colors cover it) |
| `.metrics_source`, `.allow_node_shell`, `.confirm`, proxy | Clusters · Metrics / Safety / Connection | backlog, 0037, 0030, later |
| `port_forward.presets` | Port Forwarding page | 0035 |

Rule for those specs: add the field with `SettingField::{switch, dropdown, number_input}` bound to `AppSettings::get`/`update`, and add the key to the 0024 allow-list test.

## Follow-ups for 0028 (orchestrator)

1. `keymap.md`: move `secondary-,` (`OpenSettings`, no context) and `secondary-o` (`ImportKubeconfig`, `SettingsWindow`) from Reserved to Bound; drop both from `RESERVED_KEYS`; `keymap::bind_keys` replaces `settings_window::bind_keys` (actions stay defined in `settings_window.rs`).
2. Own the Settings › Keyboard Shortcuts page: insert it at W2 position 4 with `shortcut_sheet(cx)`, add its row to `pages_follow_w2_order`; README non-goal "Ctrl , and the Settings page (0025)" and open item 1 change accordingly.
3. Sheet rows: "Open settings" (General) and "Import kubeconfig (Settings window)".
4. Check that WORKSPACE single keys do not fire in the Settings window (its root context is `SettingsWindow`, not WORKSPACE) and that FIELD `escape` does not fight the kit dialog Esc there.
