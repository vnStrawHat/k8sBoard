# 0043 · Settings model and keys

[Back to index](README.md) · Steps 1–5 · Modules: `settings.rs`, `cluster_registry.rs`, `settings_tests.rs`. Builds on 0024 ([settings-store.md](../0024-settings-store/settings-store.md)): one `#[serde(default)]` section per page, no version bump, unknown keys ignored.

## New sections (`settings.rs`)

```rust
pub(crate) struct Settings { /* 0024 fields */
    #[serde(skip_serializing_if = "is_default")] pub(crate) general: GeneralSettings,      // step 1
    #[serde(skip_serializing_if = "is_default")] pub(crate) appearance: AppearanceSettings, // step 1
    #[serde(skip_serializing_if = "is_default")] pub(crate) logs: LogSettings,             // step 2
    #[serde(skip_serializing_if = "is_default")] pub(crate) terminal: TerminalSettings,    // step 2
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] #[serde(default)]   // manual Default
pub(crate) struct GeneralSettings { pub(crate) export_dir: Option<PathBuf>, pub(crate) watch_tls_secrets: bool }
pub(crate) struct AppearanceSettings { pub(crate) density: RowDensity }       // same derives
pub(crate) struct LogSettings { pub(crate) tail_lines: u32, pub(crate) show_timestamps: bool,
    pub(crate) wrap_lines: bool, pub(crate) show_json: bool }
pub(crate) struct TerminalSettings { pub(crate) default_shell: ShellCommand,
    pub(crate) scrollback_lines: u32, pub(crate) font_size: Option<u16> }
#[serde(rename_all = "lowercase")] pub(crate) enum RowDensity { #[default] Compact, Comfortable }
fn is_default<T: Default + PartialEq>(value: &T) -> bool;   // one private helper for the four
```

- `cluster::ShellCommand` gains `Serialize, Deserialize` with `rename_all = "lowercase"` (`auto`, `bash`, `sh`); no app mirror enum (step 2).
- Each reader goes through one clamping accessor (below), so a hand-edited out-of-range value never reaches a call or an allocation. The stored value is not rewritten.
- `Option` fields keep `skip_serializing_if = "Option::is_none"` (0024 rule).

## Per-setting contract

| Key | Type · default | Read where | Takes effect | Validation (accessor) |
|---|---|---|---|---|
| `general.export_dir` | `Option<PathBuf>` · `None` = home | `file_export::start_export_with` (every Export: logs, overview report, topology) | next Export | written by the folder picker, `Use home folder` (`None`), and a confirmed save (`path.parent()`); a stored path that is not a directory falls back to home (checked on the background executor) |
| `general.watch_tls_secrets` | `bool` · true | `issue_feeds::condition_plan` (Secrets feed) | next plan: session start, cluster switch, scope change | none |
| `appearance.density` | `RowDensity` · `compact` | every `DataTable::new(..)` in `workspace.rs` (7 sites today: pods, nodes, kinds, issues, and the banner path) | live: `workspace.rs` gains `observe_global::<AppSettings>` → `cx.notify()` | enum; unknown text → file reset (0024 decision 6) |
| `logs.tail_lines` | `u32` · 1000 | `log_tab.rs` `start_streams` and the first member wave (today `POD_TAIL_LINES`); late joiners keep 50 | new tab, Reconnect, container or Previous change | `tail_lines()` clamps to 10…10,000 (the buffer keeps 10,000) |
| `logs.show_timestamps` | `bool` · true | `LogTab::new` | new tabs; the tab toggle stays per tab | none |
| `logs.wrap_lines` | `bool` · false | `LogTab::new` | new tabs | none |
| `logs.show_json` | `bool` · false | `LogTab::new` | new tabs | none |
| `terminal.default_shell` | `ShellCommand` · `auto` | `shell_open.rs` Open shell (3 `ShellCommand::Auto` sites) and `ShellTab::new` | next Open shell; the tab picker and Reconnect keep their own choice | enum |
| `terminal.scrollback_lines` | `u32` · 5,000 | `TerminalSession::new(size, scrollback)` from `ShellTab::new` | new tabs | `scrollback_lines()` clamps to 1,000…10,000 (see below) |
| `terminal.font_size` | `Option<u16>` · `None` = theme `mono_font_size` | `terminal_element::measure` | live (grid re-measured, resize sent as today) | `font_size()` clamps to 10…24 px |
| `registry.clusters[].color` | `Option<ClusterColor>` · `None` = environment colour | `title_bar.rs` top border | live | enum ([clusters-list.md](clusters-list.md)) |
| `registry.clusters[].proxy` | `Option<ClusterProxy>` · `None` = kubeconfig `proxy-url` | parsed into `ClusterProfile.proxy`; the 3 `ClusterConnection::open` callers | next connection: switch, Reconnect, Test connection, health probe | [proxy.md](proxy.md); an unparsable stored URL fails the connection (fail closed) |
| `registry.kubeconfig_folders` | `Vec<PathBuf>` · empty | `ClusterCatalog` | at once (watch starts) | absolute, a directory at pick time, no duplicate ([folder-watch.md](folder-watch.md)) |

Order of `registry.clusters` now also carries the drag order (no new key; [clusters-list.md](clusters-list.md)).

**Scrollback cap.** 0036 decision 15 sized 5,000 lines at about 8 MB per tab (200 columns, 8 bytes per cell), about 64 MB for the 8-tab cap. The 10,000-line maximum doubles that to about 16 MB per tab and 128 MB worst case; a higher cap is not offered.

**Not a setting.** The secret clipboard clear stays a fixed 30 s (0016 decision 26, C1); `secrets.clipboard_clear_seconds` stays reserved and unused until a user asks to relax it.

## Superseded reserved names (0024 persisted-prefs.md)

| 0024 reserved | 0043 key | Why |
|---|---|---|
| `issues.watch_tls_secrets` | `general.watch_tls_secrets` | one section per page; nothing reads the old name yet |
| `logs.export_dir` | `general.export_dir` | the folder serves every Export, not only logs |
| `appearance.density`, `registry.clusters[].color` | unchanged | — |

No migration: no build ever wrote the old names (structure.md: no compatibility layers).

## Allow-list additions (`settings_keys_are_the_allow_list`)

| Step | Keys |
|---|---|
| 1 | `general`, `general.export_dir`, `general.watch_tls_secrets`, `appearance`, `appearance.density` |
| 2 | `logs`, `logs.tail_lines`, `logs.show_timestamps`, `logs.wrap_lines`, `logs.show_json`, `terminal`, `terminal.default_shell`, `terminal.scrollback_lines`, `terminal.font_size` |
| 3 | `registry.clusters.color` |
| 4 | `registry.clusters.proxy`, `registry.clusters.proxy.url` |
| 5 | `registry.kubeconfig_folders` |

`full_settings()` in `settings_tests.rs` sets every new field to a non-default value, including `ClusterProxy::Url`, so each key appears. `default_file_is_minimal` stays `{"version":1,"theme":"system","registry":{}}`.

## Secret rules (0024, unchanged)

- Paths, enums, numbers, and a proxy URL **without userinfo** only. `ProxyUrl::parse` refuses `user@` / `user:pass@` ([proxy.md](proxy.md)), so the form can never store a credential.
- `ClusterProxy` has a manual `Debug` (`Direct`, `Url(scheme://host:port)`, `Url(<invalid>)`), so even a hand-edited value with userinfo never reaches a log through `Debug` of `Settings` or `ClusterEntry`.
- No `tracing` call captures a settings value with `?`/`%`; the proxy is traced as host only.
