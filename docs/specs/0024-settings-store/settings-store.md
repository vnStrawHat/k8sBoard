# 0024 · Settings store

[Back to index](README.md) · Step 2 · Modules: `settings.rs` (new: model + `AppSettings` GPUI global), `settings_store.rs` (new: file I/O, no GPUI). Decisions 1–16, 31, 32.

## Location (`settings_store.rs`)

```rust
pub(crate) const CONFIG_DIR_ENV: &str = "K8SBOARD_CONFIG_DIR";
const SETTINGS_FILE: &str = "settings.json";   // + ".bak", ".tmp" siblings
/// flag > non-empty env value > fallback; the result is made absolute (`std::path::absolute`).
pub(crate) fn config_dir(flag: Option<PathBuf>, env_value: Option<OsString>,
    fallback: Option<PathBuf>) -> Option<PathBuf>;
/// Debug builds: `<CARGO_MANIFEST_DIR>/../../.tmp/config`; release: `dirs::config_dir()/k8sboard`.
pub(crate) fn default_config_dir() -> Option<PathBuf>;
```

`None` (no home, no flag) → `main` builds `LoadedSettings { defaults, WriteMode::Disabled, Some(NoConfigDir) }`.

## Schema (`settings.rs`)

```rust
pub(crate) const SETTINGS_VERSION: u32 = 1;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] #[serde(default)] // manual Default
pub(crate) struct Settings {
    pub(crate) version: u32,                         // Default: SETTINGS_VERSION
    pub(crate) theme: ThemePreference,               // persisted-prefs.md
    pub(crate) registry: ClusterRegistry,            // step 3, cluster-registry.md
    pub(crate) tables: BTreeMap<String, TablePrefs>, // step 4, persisted-prefs.md
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemePreference { #[default] System, Light, Dark }
```

- Container-level `#[serde(default)]` only on `Settings`, `ClusterRegistry`, and `TablePrefs`. `ClusterRef` and `ClusterEntry` have none: `kubeconfig` and `context` are required, only their `Option` fields default (decision 8).
- `Option` fields use `skip_serializing_if = "Option::is_none"`; empty `Vec`/`BTreeMap` fields are skipped, so a default file is `{"version": 1, "theme": "system", "registry": {}}` (from step 3). Example: [persisted-prefs.md](persisted-prefs.md).

**Key allow-list** (AC 6, test `settings_keys_are_the_allow_list`, grows per step): step 2 `version`, `theme`; step 3 `registry.{kubeconfigs, clusters, last_used}`, entry `{kubeconfig, context, display_name, environment, read_only, default_namespace}`, `last_used.{kubeconfig, context}`; step 4 `tables.<screen>.{sort.{column, direction}, hidden}`. Later specs extend the list in their own spec and test.

## Load (`settings_store.rs`)

```rust
pub(crate) struct LoadedSettings { pub(crate) settings: Settings, pub(crate) writes: WriteMode,
    pub(crate) notice: Option<SettingsNotice> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WriteMode { Enabled(PathBuf) /* the config dir */, Disabled }
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SettingsNotice { Reset { backup: PathBuf }, NewerVersion { version: u32 },
    Unreadable { path: PathBuf, kind: io::ErrorKind }, NoConfigDir, WriteFailed { path: PathBuf, kind: io::ErrorKind } }
pub(crate) fn load_settings(dir: &Path) -> LoadedSettings;   // blocking, called from main
```

| File state | Result | Writes | Notice |
|---|---|---|---|
| missing (NotFound) | defaults | Enabled | none |
| other read error | defaults | Disabled | `Unreadable` |
| invalid JSON; `version` missing, 0, or not a `u32`; or a `Settings` deserialize error | rename to `.bak` (a failing rename → treat as `Unreadable`), defaults | Enabled | `Reset { backup }` |
| `version` == 1 | parsed | Enabled | none |
| `version` > 1 | parsed leniently; a deserialize error → defaults | Disabled | `NewerVersion` |

Two passes: `serde_json::from_slice::<serde_json::Value>` → `version` via `as_u64` + `u32::try_from` → `serde_json::from_value::<Settings>`. Trace only `error.classify()`, `line()`, `column()` and the path (decision 16). `main.rs` sets `writes = Disabled` for `--screenshot` (decision 13).

Notice texts (`impl Display`): `Settings were unreadable and were reset; the old file is {backup}` · `Settings were saved by a newer k8sBoard (version {v}); changes are not saved this session` · `Cannot read {path} ({kind}); changes are not saved this session` · `No config folder found; settings are not saved` · `Cannot save settings to {path} ({kind})`.

## Write (`settings_store.rs`)

```rust
/// Serializes writes; remembers the newest generation on disk.
#[derive(Default)] pub(crate) struct WriteGate { written: Mutex<u64> }
/// Under the gate: skip when `generation` <= written; create `dir`; write `settings.json.tmp`;
/// `sync_all`; rename over `settings.json` (one retry after 50 ms); remove the temp on failure.
/// Blocking: background executor, or the main thread only in the quit flush.
pub(crate) fn write_settings(dir: &Path, bytes: &[u8], generation: u64, gate: &WriteGate) -> io::Result<()>;
pub(crate) fn serialize_settings(settings: &Settings) -> Option<Vec<u8>>; // pretty JSON + newline
```

- Lock with `lock().unwrap_or_else(PoisonError::into_inner)` (no `unwrap`).
- The rename retry carries `// ponytail: one retry covers transient Windows sharing violations (antivirus, indexer, editor); add backoff if WriteFailed shows up in practice`. A second failure → `Err` → `WriteFailed` notice.
- `serialize_settings` → `None` on the unreachable `serde_json` error, after `tracing::warn!("cannot serialize settings")` without content (decision 32).

## GPUI global (`settings.rs`)

```rust
pub(crate) struct AppSettings { settings: Settings, writer: Option<SettingsWriter>,
    last_sent: Vec<u8>, generation: u64, notice: Option<SettingsNotice>, _quit_flush: Option<Subscription> }
struct SettingsWriter { dir: PathBuf, sender: UnboundedSender<(u64, Vec<u8>)>, gate: Arc<WriteGate> }
impl Global for AppSettings {}
impl AppSettings {
    /// Sets the global. `WriteMode::Enabled(dir)` spawns the writer task and registers the quit flush.
    pub(crate) fn install(loaded: LoadedSettings, cx: &mut App);
    pub(crate) fn get(cx: &App) -> &Settings;
    /// Applies `change`, serializes, and sends `(generation + 1, bytes)` unless equal to `last_sent`.
    pub(crate) fn update(cx: &mut App, change: impl FnOnce(&mut Settings));
    pub(crate) fn notice(cx: &App) -> Option<&SettingsNotice>;
    pub(crate) fn dismiss_notice(cx: &mut App);
    /// Writes `last_sent` synchronously when `generation > 0`. Called by the quit hook.
    pub(crate) fn flush(cx: &mut App);
}
```

- `install` sets `last_sent` to the serialized loaded settings, so an unchanged session never writes. Writes off is one signal: `writer: None`.
- Writer: `cx.spawn(async move |cx| while let Some(mut next) = rx.next().await { while let Ok(Some(newer)) = rx.try_next() { next = newer; } let result = cx.background_executor().spawn(write_settings(&dir, &next.1, next.0, &gate)).await; on Err: tracing::warn!(path, kind) and update_global(notice = WriteFailed) })`.
- Quit: `_quit_flush = Some(cx.on_app_quit(|cx| { AppSettings::flush(cx); async {} }))`. The gate drops any older in-flight background write that finishes later (decision 31). A flush error is only traced (the app is closing).
- `update` uses `cx.update_global`, so `observe_global::<AppSettings>` observers (the shell, later the 0025 window) re-render.
- Tests install `LoadedSettings { Settings::default(), WriteMode::Disabled, None }`.

## Secret rules

- No field may hold kubeconfig content, tokens, certificates, keys, or Secret values; only paths, context names, namespaces, column names, and enums.
- No `tracing` call captures `Settings` or a field of it with `?`/`%` (AC 6 grep); paths and error kinds are fine.
- Files are created with default permissions (no secrets inside).
