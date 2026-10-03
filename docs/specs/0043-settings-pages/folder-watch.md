# 0043 · Watch a kubeconfig folder

[Back to index](README.md) · Step 5 · Modules: `cluster_catalog.rs`, `kubeconfig_folder.rs` (new: scan, diff, bounded read; no GPUI), `app_shell.rs` (start), `cluster_health.rs` (probes), `clusters_page.rs`, `cluster_form.rs` (`RowOrigin`), `launch_options.rs` (`standalone_files`); cluster crate `kubeconfig.rs` (`parse_file`). Wireframe: W2 note 2, Add cluster menu `Watch a kubeconfig folder…`.

**Read-only file watching.** k8sBoard never writes, renames, or deletes anything in a watched folder; Stop watching only edits `settings.json`. No Kubernetes call is made **on its own** for a folder file: it never starts a session and is never probed without a user action (below).

## Model

- `registry.kubeconfig_folders: Vec<PathBuf>` (absolute). Folder files are **not** copied into `registry.kubeconfigs`: the folder is the source of truth, so a file deleted on disk leaves the list by itself.
- Each folder file loads **standalone** like a registry file (0024 decision 18). A file that is a chain file or in `registry.kubeconfigs` loads there, not twice (`same_path_text`). Precedence: chain, registry files, then folders in folder order, files by name.
- `RowOrigin::Folder` (new): `Remove from k8sBoard` disabled, reason `Comes from the watched folder {folder}; stop watching it or delete the file.`
- Per-context overrides stay keyed by file path; a vanished file keeps its entry (0024 open item 4) and gets name, colour, and order back when it returns.

## Untrusted by default (a file anyone can drop)

| Path | Rule |
|---|---|
| **Start** (`AppShell::on_catalog_changed` → `resolve_start`, today falling back to `kubeconfigs.first()`) | a folder context starts only when it **is** the user-picked `last_used` (exact `ClusterRef`). `resolve_start` gets the chain and registry kubeconfigs only (`ClusterCatalog::start_kubeconfigs()`), so `--context`, `current-context`, and the first-file fallback never pick a folder file. With no start candidate the shell stays without a session, shows a new empty state `No cluster selected. Pick one in the switcher.`, and opens the switcher. A folder file appearing later while no session runs never starts one, except that same `last_used` |
| **Health probe** (`HealthBoard::due` → `is_probed_automatically`) | `ProbeCandidate` gains `origin: RowOrigin`; `due` skips `RowOrigin::Folder` whatever its auth kind (a dropped file can point `tokenFile` at a real token and `server` at any host). Folder rows are probed only by the explicit per-row action, as exec rows are today |
| **Connect** | only an explicit pick: switcher click or Enter, Ctrl 1–9, palette `@`, Test connection |

## Candidates and bounded reads (`kubeconfig_folder.rs`)

```rust
pub(crate) const MAX_FOLDER_FILES: usize = 50;
pub(crate) const MAX_FILE_BYTES: u64 = 1024 * 1024;   // the paste limit (0025 decision 10)
pub(crate) const RESCAN_DEBOUNCE: Duration = Duration::from_millis(500);
pub(crate) const RESCAN_MAX_WAIT: Duration = Duration::from_secs(2);
pub(crate) struct FolderFile { pub(crate) path: PathBuf, pub(crate) len: u64, pub(crate) modified: Option<SystemTime> }
pub(crate) fn scan_folder(folder: &Path) -> io::Result<FolderScan>;  // read_dir + metadata, non-recursive
pub(crate) struct FolderScan { pub(crate) files: Vec<FolderFile>, pub(crate) skipped_over_cap: usize }
pub(crate) struct FolderChange { pub(crate) added: Vec<PathBuf>, pub(crate) changed: Vec<PathBuf>, pub(crate) removed: Vec<PathBuf> }
pub(crate) fn diff_scan(before: &[FolderFile], after: &[FolderFile]) -> FolderChange; // by path, then len/modified
/// `File::open` + `take(MAX_FILE_BYTES + 1)`; more than the cap → `TooLarge`; then `Kubeconfig::parse_file`.
pub(crate) fn load_folder_file(path: &Path) -> Result<Kubeconfig, KubeconfigError>;
// crates/cluster kubeconfig.rs
/// `parse(text, path)` plus kube's `read_from` path rule: relative `certificate-authority`, `client-certificate`,
/// `client-key`, `tokenFile`, and an exec `command` with a separator become absolute against the file's folder.
pub fn parse_file(text: &str, path: &Path) -> Result<Kubeconfig, KubeconfigError>;
```

- Never `kube::config::Kubeconfig::read_from` (unbounded read) for a folder file.
- A candidate: regular file, name not starting with `.`, extension `yaml`, `yml`, `conf`, `config`, `kubeconfig`, or none, size ≤ 1 MiB at scan time (the bounded read re-checks). Sorted by name; the first 50 kept.

## Watching (`ClusterCatalog`)

| Event | What happens |
|---|---|
| App start / folder added | background `scan_folder` + `load_folder_file` per candidate, then one `notify::RecommendedWatcher` (`NonRecursive`) per folder in `_folder_watchers` (drop = stop) |
| Events | the watcher callback (notify's thread) only sends `FolderEvent { folder: usize }` on an unbounded channel. The catalog task rescans a folder **500 ms after its last event**, and **at least every 2 s** while events keep coming (`RESCAN_MAX_WAIT`); timers from `background_executor().timer` |
| Rescan | `diff_scan`: `added` and `changed` load in the background; `removed` drop; one `cx.notify()` per rescan |
| File removed or renamed away | its rows leave the list and the switcher; a session already running on it **keeps running** until the user switches (0025 decision 24) |
| Changed file no longer parses or is over 1 MiB | the **last good** part stays; `Skipped kubeconfig: {error}` notice keyed by path; cleared by the next good load |
| New file does not parse | not listed; counted in the folder line; `tracing::debug!` with the path only |
| Folder missing or `read_dir` fails | its parts drop; notice `Watched folder {path} is missing ({kind}); checked again at the next start`; the folder stays in settings |
| Over 50 candidates | first 50 by name; the folder line says `{n} more files not loaded` |

- `notify` 7.0.0 is already in `Cargo.lock` (through `gpui-component`): `crates/app` adds `notify = "7"`; AC: no new `[[package]]`.
- **Testability**: the task reads from the channel, not from notify. `ClusterCatalog::folder_events_for_test()` returns the sender (`#[cfg(test)]`); tests send events and drive the timers with the GPUI fake clock (`executor().advance_clock`). Real notify runs only in the coder-lite and ui-verifier checks.
- Every `read_dir`, `metadata`, read, and parse runs on the background executor.
- The catalog observer of `registry.kubeconfigs` (0025) also follows `kubeconfig_folders`.

## UI

| Where | What |
|---|---|
| Add cluster menu | `Watch a kubeconfig folder…` → `prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false })` → `std::path::absolute` → appended unless already there (`same_path_text`) |
| Clusters page, with the catalog notices | per folder: `Watching {path} · {k} kubeconfig files{, m not kubeconfigs}{, n more files not loaded}` + ghost `Stop watching` (no dialog: nothing is deleted) |
| Switcher rows from a folder | as other rows, with no automatic health dot until the user probes it |

## Not done (lean on purpose)

Recursive folders, glob filters, auto-registering folder files, re-watching a missing folder before the next start, a polling fallback for network drives, watching single registry files or the launch chain (0025 decision 24).
