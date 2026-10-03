# 0025 · Window and shared state

[Back to index](README.md) · Steps 2a, 2b (catalog file operations: step 4) · Modules: `cluster_catalog.rs` (new), `settings_window.rs` (new), `main.rs`, `app_shell.rs`, `title_bar.rs`, `screenshot.rs`, `launch_options.rs`. Decisions 1–6, 21–25, 27.

## Verified APIs (vendored crates)

| API | Where |
|---|---|
| `gpui_kit::open_window(options, cx, build) -> Result<(AnyWindowHandle, Entity<V>)>` | `gpui-kit-0.7.0/src/lib.rs:144` (wraps in `base::Root`) |
| `AnyWindowHandle::update(cx, \|view, window, cx\| ..) -> Result<R>`; `Window::activate_window()`; `Window::set_window_title` | `gpui-pre-0.3.7/src/window.rs:7333, 6413, 2876` |
| `App::on_window_closed(\|cx, WindowId\|)`, `AnyWindowHandle::window_id()`, `WindowId::from(u64)` | `app.rs:2493`, `window.rs:7308, 7148` |
| `App::quit` → `platform.quit()`; the test platform's `quit` is a no-op and `on_app_quit` runs only in `shutdown()` | `app.rs:1114, 1060`, `platform/test/platform.rs:436` |
| `App::prompt_for_paths(PathPromptOptions { .. })` → `oneshot::Receiver<Result<Option<Vec<PathBuf>>>>` | `app.rs:1690`, `platform.rs:2485` |
| `App::read_from_clipboard()`, `write_to_clipboard(ClipboardItem)`, `reveal_path(&Path)` | `app.rs:1521, 1549, 1712` |
| `observe_global::<G>` on `Context` | `app/context.rs:168` |
| Keystroke `secondary-` = Cmd on macOS, Ctrl elsewhere | `platform/keystroke.rs:116` |
| `setting::{Settings, SettingPage, SettingGroup, SettingItem, SettingField, SelectIndex}`; `SettingField::dropdown(Vec<(key, label)>, Fn(&App) -> SharedString, Fn(SharedString, &mut App))` | `gpui-component-0.7.0/src/setting/` (`fields/mod.rs:188`) |
| `Dialog::on_close` (after OK, Cancel, Esc, overlay click), `on_ok`/`on_cancel` → `bool` | `dialog/dialog.rs:405–435, 634–650` |
| `Button::tooltip_with_action(text, &action, context)` renders the kit `Kbd` | `button/button.rs:414`, `tooltip.rs:98` |
| `switch::Switch`, `input::{InputState, Input}`, `select`, `WindowExt::open_dialog` / alert dialog | `gpui-component-0.7.0/src/` (`window_ext.rs:31`) |

Coder checks in step 2b: a headless test opens a dialog in the Settings window (the main shell has never used kit dialogs); step 4: a test proves Esc runs `on_close`.

## `ClusterCatalog` (`cluster_catalog.rs`)

```rust
pub(crate) struct ClusterCatalog { chain: CatalogPart, standalone: Vec<(PathBuf, CatalogPart)>,
    notices: Vec<CatalogNotice>, paste: PasteStatus, _settings_observer: Subscription }
pub(crate) enum CatalogPart { Loading, Loaded(Arc<Kubeconfig>), Failed(String) }
pub(crate) struct CatalogHandle(pub(crate) Entity<ClusterCatalog>);   // Global
impl ClusterCatalog {                                                  // step 2a
    /// Loads the chain once and every standalone registry file (background executor).
    pub(crate) fn new(chain: Vec<PathBuf>, cx: &mut Context<Self>) -> Self;
    pub(crate) fn kubeconfigs(&self) -> impl Iterator<Item = &Arc<Kubeconfig>>; // chain first
    pub(crate) fn chain(&self) -> Option<&Arc<Kubeconfig>>;            // step 4
    pub(crate) fn is_loading(&self) -> bool;
    pub(crate) fn is_chain_source(&self, path: &Path) -> bool;        // step 3; same_path_text
    pub(crate) fn notices(&self) -> &[CatalogNotice];
    pub(crate) fn clear_notices(&mut self, cx: &mut Context<Self>);
}
```

- **2a is a pure move**: 0024's `KubeconfigState`, the load task, and the kubeconfig notices leave `AppShell`. `AppShell` keeps the start choice, `active`, and the session; it observes the catalog (`_catalog_observer`) and runs `start_choice` once, on the first load completion. Title bar, notices, and screenshots look the same.
- `_settings_observer`: when `registry.kubeconfigs` differs from the standalone list, load added files and drop removed ones (`standalone_files` from 0024), then `cx.notify()`. The chain never reloads (decision 24).
- `main.rs` creates it before the main window: `cx.set_global(CatalogHandle(cx.new(|cx| ClusterCatalog::new(chain, cx))))`.

## Catalog file operations (step 4, decisions 25, 27)

```rust
pub(crate) enum CatalogNotice { Skipped { path: Option<PathBuf>, error: String }, Unregistered(PathBuf), DeleteFailed(PathBuf), SaveFailed(io::ErrorKind) }
pub(crate) enum PasteStatus { Idle, Saving, Added(PathBuf), Failed }
impl ClusterCatalog {
    pub(crate) fn add_pasted(&mut self, text: String, first_context: Option<String>, clipboard: ClipboardMark, cx: &mut Context<Self>);
    pub(crate) fn remove_kubeconfig(&mut self, path: PathBuf, cx: &mut Context<Self>);
    pub(crate) fn paste_status(&self) -> &PasteStatus;
    pub(crate) fn reset_paste_status(&mut self, cx: &mut Context<Self>);
}
```

- `add_pasted`: `paste = Saving`; `cx.spawn(..).detach()` (the catalog lives as long as the app, so closing Settings does not cancel it). Background: `write_pasted_kubeconfig(dir, first_context, &text)` (on a write or sync error the partial file is removed), drop `text`, then `Kubeconfig::load(&[path])`; a load error deletes the new file. Then **one** `this.update`: insert `(path, Loaded)` into `standalone` first (the observer then sees no difference), `AppSettings::update` pushes the path, clear the clipboard when its text still matches the `ClipboardMark` (decision 11; the clipboard text read for the check is `Zeroizing`), `paste = Added(path)`. Write error → `SaveFailed(kind)`, `paste = Failed`, clipboard untouched.
- `remove_kubeconfig`: app-owned path (`is_app_owned`, needs `AppSettings::config_dir`) → background `remove_file`; `Ok` → `AppSettings::update(cluster_form::remove_kubeconfig)` and drop an `Unregistered` notice for it; `Err` → `DeleteFailed(path)`, registry untouched. Other paths, or writes off → registry edit only, no delete.
- **Unregistered files**: `new` also lists the pasted-looking `<config>/kubeconfigs/*.yaml` (names `is_pasted_file_name` accepts; background `read_dir`, start only) and raises `Unregistered(path)` for each path not in `registry.kubeconfigs` (a crash between write and push). Its Remove action calls `remove_kubeconfig`.
- **Per-part notices**: every part that fails to load (and every skipped chain file) raises a `Skipped` notice, also when nothing loads at all (the error screen then shows it too). A notice holds the file path when the error names one: it is dropped when the file leaves the registry, and a notice already shown is not added again.
- Notice texts: `Skipped kubeconfig: {error}` · `Pasted file {name} is not registered` · `Could not delete {name}; it is still listed` · `Could not save the pasted kubeconfig ({kind})`. Paths and kinds only in `tracing`.
- The main title bar shows notice text (as 0024); the Clusters page shows them with their Remove action.

## Opening (`settings_window.rs`, step 2b)

```rust
gpui_kit::actions!(k8sboard, [OpenSettings, ManageClusters, ImportKubeconfig]);
pub(crate) struct SettingsWindowHandle(Option<OpenWindow>);          // Global; OpenWindow { window: AnyWindowHandle, view: WeakEntity<SettingsWindow> }
pub(crate) enum SettingsSize { Standard, Tall }                       // Tall: screenshots of the whole form
pub(crate) fn open_settings_window(page: SettingsPage, size: SettingsSize, cx: &mut App) -> Option<AnyWindowHandle>; // activate (page stays) or open
pub(crate) fn manage_clusters(cx: &mut App);                          // like the above, then show Clusters
pub(crate) fn forget_closed_window(id: WindowId, cx: &mut App);     // handle → None on match
pub(crate) fn bind_keys(cx: &mut App);   // other-pages.md "Keys"
pub(crate) struct SettingsWindow { catalog: Entity<ClusterCatalog>, clusters: ClustersPageState,
    focus_handle: FocusHandle, _observers: Vec<Subscription> }      // no Debug (paste_text)
```

- `main.rs`: `cx.on_action` for `OpenSettings` (`open_settings_window(Clusters, Standard, cx)`) and `ManageClusters` (`manage_clusters`), `settings_window::bind_keys(cx)`.
- Title bar: the Settings button is enabled, `.tooltip_with_action("Settings", &OpenSettings, None)` (kit `Kbd`, per OS); click dispatches `OpenSettings`. "Manage clusters…" dispatches `ManageClusters`: it opens the window or brings it forward, and switches an open window to Clusters (a new `Settings` key per request, so the kit selection restarts; `Ctrl ,` leaves the page alone). A new window opens on Clusters (`SelectIndex { page_ix: CLUSTERS_PAGE, group_ix: None }`); an open window only activates (see `ManageClusters`). `SettingsWindow::new` calls `reset_paste_status`.
- `SettingsWindow` observes `AppSettings` and the catalog; never `AppShell`. Nothing else holds a strong handle to it, so closing the window drops it with `ClustersPageState`.
- Root element: `.key_context("SettingsWindow")`, `.track_focus`, `on_action(ImportKubeconfig)`; a kit `TitleBar` with "Settings", then `Settings::new("settings").pages(..)`.

## Close rules (`main.rs`, step 2b)

```rust
/// Registers the close hook. `quit` is `|cx| cx.quit()` in production; tests pass a flag setter.
pub(crate) fn quit_when_main_window_closes(main: WindowId, quit: impl Fn(&mut App) + 'static, cx: &mut App);
```

Hook body: `forget_closed_window(id, cx)`; then `if id == main || cx.windows().is_empty() { quit(cx) }`. Closing Settings after the main window can call `quit` twice; that is harmless. Lives in `settings_window.rs`; replaces the "quit when no windows remain" hook.

## Theme and screenshots (step 2b)

- `ThemePreference::apply(self, cx)` in `settings.rs` replaces `main.rs` `apply_theme` (keeps `table_active = selection`). Called at start with `options.theme.unwrap_or(stored)` and by the Appearance dropdown.
- `--screen settings` / `settings-appearance` / `settings-tall` (`LaunchScreen::Settings(SettingsPage, SettingsSize)`; tall is 900 px high so a shot shows the Reset and Remove footer, which the standard 620 px window scrolls to): the main window opens as usual, then `open_settings_window` on that page; the capture targets the Settings window once the catalog has loaded and Test connection is idle. USAGE updated.

## Async contract

| Work | Where it runs | Result reaches UI by |
|---|---|---|
| kubeconfig read/parse (catalog, import preview) | `background_executor().spawn` | `cx.spawn` → `this.update` |
| file picker | platform; await the oneshot inside `cx.spawn` | `this.update` |
| pasted-file write + registry push; app-owned delete | catalog task (detached) → background executor | one catalog `this.update`; the view renders `PasteStatus` / notices |
| Test connection | `ClusterRuntime::spawn(test_connection(.., 15 s))`; `tokio::time::timeout` inside | `RuntimeTask` awaited in `cx.spawn`; dropping it aborts |
| `settings.json` writes | 0024 writer task | unchanged |
