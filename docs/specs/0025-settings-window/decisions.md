# 0025 · Decisions

[Back to index](README.md). Defaults picked without a user round-trip (user instruction); amended after the advisor review.

| # | Decision | Rationale |
|---|---|---|
| 1 | The window is opened with `gpui_kit::open_window` (wraps the view in kit `Root`), 1080 × 620, centered, `TitleBar::window_options()`, its own kit `TitleBar` titled "Settings" | W2 label; same chrome as the main window; `Root` hosts dialogs |
| 2 | **Single instance** through a `Global` `SettingsWindowHandle(Option<OpenWindow>)` (the window handle plus a weak view): open = `handle.update(cx, \|_, window, _\| window.activate_window())`; an `Err` opens a new one. The close hook sets it to `None` when the Settings window closes | W2 note 1; a stale handle is never kept |
| 3 | `OpenSettings` is bound to `secondary-,` with **no key context** (works in both windows, also inside dialogs and inputs); `ImportKubeconfig` to `secondary-o` in `SettingsWindow`. 0025 binds them in `settings_window::bind_keys`; 0028 later moves them to `keymap.rs` | keyboard map `Ctrl ,`; 0025 lands before 0028 |
| 4 | Closing the **main** window quits the app; closing Settings does not. A second `quit` during shutdown is harmless | the app has no use without its main window |
| 5 | Loaded kubeconfigs move out of `AppShell` into an app-level **`ClusterCatalog` entity**, held in a `Global`; both windows observe it. Step 2a is that move alone | wireframe stack table: "two windows share the cluster store"; a refactor-only step is easy to review |
| 6 | The window body is the kit **`setting::Settings`** component. Appearance uses `SettingField::dropdown` bound to `AppSettings`; the Clusters page is one `SettingItem::render`; pages are not resettable | verified in `gpui-component` 0.7 `setting/` |
| 7 | Every valid edit saves at once (`AppSettings::update`); invalid edits show a message and keep the last valid stored value | W2: "changes save immediately" |
| 8 | **Import file stores the path only**; the file is never copied | 0024 decision 19 and C2 |
| 9 | **Paste writes a new kubeconfig file** `<config>/kubeconfigs/<name>.yaml` (`create_new`). Unix: file `0o600`, folder `0o700`. Windows: no explicit ACL; the file inherits `%APPDATA%\k8sboard`, which inherits the per-user profile ACL (user, SYSTEM, Administrators), the same protection kubectl gives `%USERPROFILE%\.kube\config`; the folder gets no special treatment either. Dev builds write under `<workspace>/.tmp/config` (0024 decision 4): git-ignored, project-folder ACL | a DACL needs FFI and `unsafe` (denied); the dialog shows the target path |
| 10 | Paste reads the clipboard with `cx.read_from_clipboard()` and **never renders the text**; max 1 MiB. The text sits in `ClustersPageState.paste_text` only after the size and parse checks and is cleared on Save and add, Cancel, Esc/overlay close, preview error, and window close | C1: tokens are never displayed; one owner, short life |
| 11 | After a successful paste import, the clipboard is cleared if its text still has the same keyed hash (`ClipboardMark`, the type the Secret copy uses, computed before the text moves into the write). No comparison when the write failed | C1 hygiene without a second copy of the text |
| 12 | **Remove from k8sBoard** works per registry file; launch-chain contexts cannot be removed, only Reset | W2 note 6; the chain is the user's environment |
| 13 | Remove deletes the file only when it is **app-owned**, **before** dropping its registry entries; a failed delete keeps it listed with a notice | pasted credentials must not linger unlisted |
| 14 | Collisions are **warnings**, never blockers: context names (with `ContextSummary.source`), display names equal to an imported context name, and cluster/user names ("your KUBECONFIG chain" or the file). Display-name uniqueness blocks user edits only | advisor S2, S3; the user decides |
| 15 | Connection info: source path, context, server (`scheme://host:port`), auth **kind** (exec shows the command's file name only) | W2 pin 5 without secrets |
| 16 | Test connection = `ClusterConnection::open` + `server_version()` in one tokio future under `tokio::time::timeout(15 s)`; latency times `server_version` only; the error line is the top-level `ClusterError` `Display` | read-only; same code path as a session; no source-chain text |
| 17 | The read-only switch shows `entry.read_only.unwrap_or(environment == Production)` (`ClusterProfile.read_only`), hint "Default: on for Production…"; only Reset returns it to the default. It does not change the title-bar Read-only badge in 0025 | the user can set it now; 0030 enforces it |
| 18 | Default namespace is a text input validated as a DNS-1123 label; empty clears it | no cluster calls for a form |
| 19 | Display names: trimmed, ≤ 64 chars, no control chars, unique (case-insensitive) among listed clusters; empty → context name | the switcher and future Ctrl 1–9 labels must be unambiguous |
| 20 | The Environment select offers "Auto ({guess})" plus the four environments; Auto stores `None` | keeps C5 guessing visible and reversible |
| 21 | Theme changes apply live through `ThemePreference::apply(cx)`; `--theme` only overrides at start; one `THEME_OPTIONS` table | one code path, one string table |
| 22 | General is omitted until a spec gives it content; Keyboard Shortcuts belongs to 0028 | no empty pages |
| 23 | Screenshots: `--screen settings` / `settings-tall` / `settings-appearance` open the main window plus Settings and capture the Settings window | ui-verifier needs W2 evidence |
| 24 | Registry changes reload only the **standalone** files; the chain loads once; a running session is never stopped by Remove | cheap; no surprise disconnect |
| 25 | The catalog owns **write + load + registry push** of a paste, and the delete of an app-owned file, as one detached task ending in one main-thread update; the view only renders `PasteStatus` and notices | closing Settings mid-save cannot leave a credential file without a registry entry |
| 26 | Path checks use `same_path_text` (string-level; Windows: `/` = `\`, case-insensitive; Unix: case-sensitive) | structure.md: no `std::path` for Windows-shaped strings; macOS case-insensitivity is a ceiling (open item 5) |
| 27 | At start the catalog lists `<config>/kubeconfigs/*.yaml` not in the registry and raises `Pasted file X is not registered` with a Remove action | catches a crash between write and push |
