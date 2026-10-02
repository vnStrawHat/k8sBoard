# 0025 — Settings window (W2)

Status: draft, amended after the advisor review (M1–M3, S1–S12, N1–N8), HEAD `9d5af01`. Builds on 0024 (settings store, registry, environments; may not be merged yet: step 1 here starts from the 0024 code). Crates: `crates/cluster` (parse, connection info, names) and `crates/app`. Local only: no Kubernetes write; the one cluster call is the read-only `GET /version` of Test connection. Applies C1, C2, C5. Wireframe: W2 (pins 1–6), W1 "Manage clusters…", keyboard map `Ctrl ,`.

## Goal

- A **separate OS window** (one instance), opened by `Ctrl ,` (`Cmd ,` on macOS), the title-bar Settings button, and "Manage clusters…". It shares the `AppSettings` global and a new `ClusterCatalog` entity with the main window; changes apply immediately in both.
- **Clusters page**: env-grouped list; form with display name, environment, read-only lock, default namespace; read-only connection info (no credentials); Test connection; Reset; Remove from k8sBoard.
- **Add cluster**: import a kubeconfig file (picker, `Ctrl O`) by path, or paste one from the clipboard into a new file `<config>/kubeconfigs/*.yaml` (never `settings.json`); a preview with name-collision warnings first. The app-lifetime catalog owns every file write and delete, so closing the window never orphans a pasted file.
- **Appearance** (theme, live) and **About** pages.

## Non-goals

- Pages without content yet: General, Keyboard Shortcuts (0028), Safety (0030), Terminal & Shell (0036), Logs (0019), Metrics (backlog), Extensions (backlog). See [other-pages.md](other-pages.md).
- Watch a kubeconfig folder, cloud scans, drag reorder, cluster colors, proxy, density, confirm mode, node shell toggle, list search, the W2 nav count.
- Showing or editing credentials; editing kubeconfig files (except deleting an app-owned pasted file on Remove).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster crate: `Kubeconfig::parse`, `connection_info`, `entry_names` | 1–3 |
| 2a | `ClusterCatalog` refactor only: no UI change, `--screen` unchanged | 1, 2 |
| 2b | Settings window (open/activate, keys, close rules); Appearance and About; `--screen settings*` screenshots | 1, 2, 4–6, 12 |
| 3 | Clusters page: list, form, validation, Reset, Test connection | 1, 2, 7–9, 12 |
| 4 | Import file and paste (catalog-owned write), preview, collisions, Remove, unregistered-file notice; full ui-verifier run | 1, 2, 10–12 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with one-line rationale |
| [window-and-sharing.md](window-and-sharing.md) | second window, single instance, keys, close rules, `ClusterCatalog` and its file operations, kit `Settings` layout |
| [clusters-page.md](clusters-page.md) | list, form fields, validation messages, connection info, Test connection, Reset, Remove |
| [import.md](import.md) | file import, clipboard paste, preview, collisions, path comparison, pasted-file storage and permissions, C1 |
| [other-pages.md](other-pages.md) | Appearance, About, page order, keys and the 0028 hand-over, later pages |
| [files-to-touch.md](files-to-touch.md) | files per step, cluster-crate API, doc updates |
| [test-plan.md](test-plan.md) | unit and window tests, live checks, ui-verifier checklist |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`, no `unsafe`.
- [ ] 2. Every test in [test-plan.md](test-plan.md) for the step exists under that name and passes offline; file tests write only under `std::env::temp_dir()`. Step 2a leaves every existing test and `--screen` output unchanged.
- [ ] 3. No credential reaches the UI, a log, or `settings.json`: connection info shows auth kinds and the exec command's file name only; `Debug` of the new types holds no credential (tests); `SettingsWindow` and `ClustersPageState` have no `Debug`.
- [ ] 4. `Ctrl ,` twice, the Settings button, and "Manage clusters…" all leave exactly one Settings window, focused.
- [ ] 5. Closing Settings keeps the app running and clears the handle; closing the main window quits the app (Settings closes too).
- [ ] 6. Changing the theme in Appearance re-themes both windows at once and persists (`settings.json` `theme`).
- [ ] 7. Editing a display name or environment updates the main title-bar label and badge without restart.
- [ ] 8. Invalid input shows the exact message from [clusters-page.md](clusters-page.md) under the field and is not saved.
- [ ] 9. Test connection on `readonly@Monitor` shows `Connected · v1.29.5 · {n} ms` and sends only `GET /version` (0001 read-only grep unchanged; a `RUST_LOG=kube=trace` run shows one request).
- [ ] 10. Importing a file adds only its path to `registry.kubeconfigs`; its contexts appear in the switcher without restart. Colliding names show the warnings from [import.md](import.md).
- [ ] 11. Paste never renders the clipboard text; the file lands in `<config>/kubeconfigs/` (Unix: file `mode & 0o077 == 0`, folder `0o700`); `settings.json` holds only its path. Closing Settings during the save still registers the file. Remove deletes an app-owned file first and drops its entries only after the delete succeeded.
- [ ] 12. Screenshots `settings` (Clusters) and `settings-appearance`, light and dark: no high-severity defect against W2 (known deviations: no nav count, row meta shows the file name).

## Open items

1. Pasted files on Windows inherit the profile ACL like kubectl's `~\.kube\config` (decision 9). A `--config-dir` on a shared folder would need an explicit DACL (FFI; `unsafe_code` is denied today).
2. Clearing the clipboard after paste cannot remove Windows clipboard history (Win+V) entries.
3. Input fields save on every valid keystroke (kit pattern); add a debounce if `settings.json` churn shows in profiles.
4. Pages and fields listed in [other-pages.md](other-pages.md) land with their owner specs; 0028 follow-ups are listed there.
5. Path comparison treats macOS as case-sensitive (decision 26); a case-only mismatch there can load a file twice (shown with file labels, harmless).
