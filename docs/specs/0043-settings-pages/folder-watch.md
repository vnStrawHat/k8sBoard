# 0043 · Watch a kubeconfig folder

[Back to index](README.md) · Step 5 · Modules: `cluster_catalog.rs`, `kubeconfig_folder.rs` (new: scan + diff, no GPUI), `clusters_page.rs`, `cluster_form.rs` (`RowOrigin`), `launch_options.rs` (`standalone_files`). Wireframe: W2 note 2 ("watch a whole kubeconfig folder, picks up new files"), Add cluster menu `Watch a kubeconfig folder…`.

**Read-only file watching.** k8sBoard never writes, renames, or deletes anything in a watched folder; Stop watching only edits `settings.json`. No Kubernetes call.

## Model

- `registry.kubeconfig_folders: Vec<PathBuf>` (absolute). Folder files are **not** copied into `registry.kubeconfigs`: the folder is the source of truth, so a file deleted on disk leaves the list by itself.
- Each folder file loads **standalone** like a registry file (0024 decision 18). Duplicates: a file that is a chain file or in `registry.kubeconfigs` loads there, not twice (`same_path_text`, 0025 decision 26). Precedence: chain, then registry files, then folders in folder order, files by name.
- `RowOrigin::Folder` (new): its row's `Remove from k8sBoard` is disabled with reason `Comes from the watched folder {folder}; stop watching it or delete the file.`
- Per-context overrides (`registry.clusters`) stay keyed by file path; a file that disappears keeps its entry (harmless, 0024 open item 4) and gets its name, colour, and order back when it returns.

## Candidates (`kubeconfig_folder.rs`)

```rust
pub(crate) const MAX_FOLDER_FILES: usize = 50;
pub(crate) const MAX_FILE_BYTES: u64 = 1024 * 1024;   // the paste limit (0025 decision 10)
pub(crate) const RESCAN_DEBOUNCE: Duration = Duration::from_millis(500);
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FolderFile { pub(crate) path: PathBuf, pub(crate) len: u64, pub(crate) modified: Option<SystemTime> }
/// Blocking: `read_dir` + `metadata` (follows file symlinks). Non-recursive.
pub(crate) fn scan_folder(folder: &Path) -> io::Result<FolderScan>;
pub(crate) struct FolderScan { pub(crate) files: Vec<FolderFile>, pub(crate) skipped_over_cap: usize }
pub(crate) struct FolderChange { pub(crate) added: Vec<PathBuf>, pub(crate) changed: Vec<PathBuf>, pub(crate) removed: Vec<PathBuf> }
pub(crate) fn diff_scan(before: &[FolderFile], after: &[FolderFile]) -> FolderChange; // by path, then len/modified
```

A candidate is a regular file, name not starting with `.`, extension `yaml`, `yml`, `conf`, `config`, `kubeconfig`, or none, size ≤ 1 MiB. Sorted by file name; the first 50 are kept, the rest counted in `skipped_over_cap`.

## Watching (`ClusterCatalog`, step 5)

| Event | What happens |
|---|---|
| App start / folder added | background `scan_folder`, load every candidate (`Kubeconfig::load(&[file])` on the background executor), then create one `notify::RecommendedWatcher` (`NonRecursive`) per folder, kept in `ClusterCatalog._folder_watchers` (drop = stop) |
| Any notify event or watcher error for a folder | send the folder index on an unbounded channel; the catalog task waits until **500 ms pass with no new event** for that folder (trailing debounce, `background_executor().timer`), then rescans once |
| Rescan | `diff_scan`: `added` and `changed` files load in the background; `removed` files drop their part; one `cx.notify()` per rescan (batched) |
| A file is removed or renamed away | its part leaves the catalog: its contexts leave the Clusters list and the switcher. A session already running on it **keeps running** until the user switches (0025 decision 24). `last_used` pointing to it is ignored at the next start (0024 decision 21) |
| A file is changed and no longer parses | the **last good** part stays loaded; notice `Skipped kubeconfig: {error}` (the 0025 `Skipped` notice, keyed by path); the next good load clears it |
| A new file does not parse | not listed; counted in the folder line, `tracing::debug!` with the path only (a folder like `~/.kube` holds non-kubeconfig files; no notice spam) |
| The folder disappears or `read_dir` fails | every part of that folder drops; notice `Watched folder {path} is missing ({kind}); checked again at the next start`; the folder stays in settings until Stop watching |
| Over 50 candidates | the first 50 by name load; the folder line says `{n} more files not loaded` |

- `notify` 7.0.0 is already in `Cargo.lock` (through `gpui-component`): `crates/app` adds `notify = "7"` with default features; AC: no new `[[package]]`.
- The watcher callback runs on notify's own thread and only sends a folder index; every `read_dir`, `metadata`, and parse runs on the background executor. Nothing blocks the GPUI thread.
- The catalog observer of `registry.kubeconfigs` (0025) also follows `kubeconfig_folders`: an added folder starts as above; a removed folder drops its watcher, its parts, and its notices.

## UI

| Where | What |
|---|---|
| Add cluster menu | `Watch a kubeconfig folder…` enabled → `prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false })` → `std::path::absolute` → appended to `kubeconfig_folders` unless already there (`same_path_text`; a repeat changes nothing, the folder line is already shown) |
| Clusters page, above the list (with the catalog notices) | one muted line per folder: `Watching {path} · {k} kubeconfig files{, m not kubeconfigs}{, n more files not loaded}` + ghost small `Stop watching` → `AppSettings::update` removes the path (no dialog: nothing is deleted, and Watch again restores everything) |
| Row meta | `{auth} · {file name}` as today |

## Not done (lean on purpose)

Recursive folders, glob filters, auto-registering folder files into `registry.kubeconfigs`, re-watching a missing folder before the next start, a polling fallback for network drives (notify's error makes the folder show as missing), watching single registry files or the launch chain (0025 decision 24: the chain loads once).
