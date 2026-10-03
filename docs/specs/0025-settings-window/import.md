# 0025 · Import a kubeconfig

[Back to index](README.md) · Step 4 · Modules: `kubeconfig_import.rs` (new, pure + blocking file I/O), `settings_window.rs` (dialogs), `cluster_catalog.rs` (write and delete), cluster crate `kubeconfig.rs`. Decisions 8–11, 13, 14, 25–27. C1, C2.

## Cluster crate (step 1)

```rust
impl Kubeconfig {
    /// Parses YAML text in memory (pasted content). `origin` is used in errors only; relative
    /// credential paths stay relative (the file is re-loaded with `load` after it is written).
    pub fn parse(text: &str, origin: &Path) -> Result<Kubeconfig, KubeconfigError>;
    /// Server and auth kind of a context; never a credential value.
    pub fn connection_info(&self, context: &ContextSummary) -> ConnectionInfo;
    /// Context, cluster, and user entry names, in file order.
    pub fn entry_names(&self) -> EntryNames;
}
pub struct ConnectionInfo { pub server: Option<String>, pub auth: AuthKind }
pub enum AuthKind { Token, TokenFile, ClientCertificate, Exec { command: String }, AuthProvider { name: String },
    Basic, None }                                             // impl Display: "token", "exec: aws", …
pub struct EntryNames { pub contexts: Vec<String>, pub clusters: Vec<String>, pub users: Vec<String> }
```

- `parse`: `kube::config::Kubeconfig::from_yaml(text)`, which merges every YAML document (`file_config.rs:502`), then `from_document(vec![origin], doc, &HashMap::new())`. Any error, a multi-document merge error included → `KubeconfigError::Parse { path: origin }` without the source (it can quote a token line).
- `server`: manual split (no new direct dependency): keep `scheme://host[:port]`; drop userinfo (`…@`), path, query, fragment. An `@` after the first `/`, `?`, or `#` is ambiguous (a raw password may hold one): then only `scheme://…` shows.
- `AuthKind` order of checks: exec, auth provider, client certificate (file or data), token, token file, basic (username), none. `Exec.command`: the text after the last `/` or `\` of `exec.command`, never args or env.

## Preview (`kubeconfig_import.rs`)

```rust
#[derive(Debug)] pub(crate) struct ImportPreview { pub(crate) source: ImportSource,
    pub(crate) contexts: Vec<ContextPreview>, pub(crate) collisions: Vec<NameCollision> }
#[derive(Debug)] pub(crate) enum ImportSource { File(PathBuf), Pasted { target: PathBuf } }
#[derive(Debug)] pub(crate) struct ContextPreview { pub(crate) name: String, pub(crate) server: Option<String>,
    pub(crate) auth: AuthKind, pub(crate) environment: Environment /* guessed */ }
#[derive(Debug)] pub(crate) struct NameCollision { pub(crate) kind: EntryKind, pub(crate) name: String,
    pub(crate) place: CollisionPlace }
pub(crate) enum EntryKind { Context, DisplayName, Cluster, User }
pub(crate) enum CollisionPlace { File(PathBuf), KubeconfigChain }
pub(crate) fn name_collisions(candidate: &Kubeconfig, rows: &[ClusterRow],
    chain: Option<&Kubeconfig>, standalone: &[&Kubeconfig]) -> Vec<NameCollision>;
pub(crate) fn check_new_file(path: &Path, registered: &[PathBuf], is_chain_source: bool) -> Result<(), ImportError>;
```

The derived `Debug` is safe: the preview holds names, servers without userinfo, and auth kinds only (test). The dialog lists each context (badge of the guessed env, name, server, auth kind), the target ("Adds {path}" or "Saves to {target}"), then warnings in theme warning color:

| Kind | Found by | Place | Warning |
|---|---|---|---|
| Context | a row with the same context name | `File(row.cluster.kubeconfig)` = `ContextSummary.source` | `Context '{name}' also exists in {file}. Both are listed; the switcher adds the file name.` |
| DisplayName | a row whose stored `display_name` equals the context name (case-insensitive) | `File(row source)` | `Context '{name}' matches the name shown for a cluster from {file}. Both are listed; rename one in Settings.` |
| Cluster / User | `entry_names` of the chain | `KubeconfigChain` | `{Cluster\|User} '{name}' also exists in your KUBECONFIG chain. k8sBoard reads this file on its own, so nothing is replaced; kubectl would use only the first one if both were in KUBECONFIG.` |
| Cluster / User | `entry_names` of a standalone file | `File(its source)` | same text with `in {file}` |

Warnings never block (decision 14). Buttons: `Add` (file) or `Save and add` (paste), and `Cancel`.

## Path comparison (decision 26)

```rust
pub(crate) enum PathStyle { Windows, Unix }   // PathStyle::HOST = if cfg!(windows) { Windows } else { Unix }
/// Windows: split on '/' and '\\', compare parts case-insensitively (`to_lowercase`).
/// Unix: split on '/', case-sensitive. Both skip empty and "." parts. String-level only.
pub(crate) fn same_path_text(a: &str, b: &str, style: PathStyle) -> bool;
```

Lives in `cluster_catalog.rs` (step 3, first user `is_chain_source`). Used by `is_chain_source`, `check_new_file`, `is_app_owned`, and the unregistered-file scan (`to_string_lossy` of each path).

## Import file (`Ctrl O`, "Import kubeconfig file…")

1. `prompt_for_paths { files: true, directories: false, multiple: false, prompt: Some("Import kubeconfig") }`; `None` → nothing.
2. `check_new_file` (absolute path): already in `registry.kubeconfigs` → `This file is already added.`; a chain source → `This file is already loaded from KUBECONFIG or --kubeconfig.`
3. Background `Kubeconfig::load(&[path])`: errors use the cluster-crate text (no content). Zero contexts → `This kubeconfig has no contexts.`
4. Preview → `Add` → `AppSettings::update(|s| s.registry.kubeconfigs.push(path))` (no file is written, so no orphan risk). The catalog loads it; the new rows appear and the first is selected.

## Paste ("Paste kubeconfig YAML…")

1. Dialog text: `k8sBoard reads the kubeconfig from the clipboard. Its content is never shown.` Buttons `Read clipboard`, `Cancel`. Disabled with `Settings are not saved this session, so a pasted kubeconfig cannot be stored.` when `AppSettings::config_dir(cx)` is `None` (writes off).
2. `cx.read_from_clipboard().and_then(|item| item.text())` into a local: none → `The clipboard has no text.`; > 1 MiB → `The clipboard text is larger than 1 MiB; that is not a kubeconfig.`
3. Background `Kubeconfig::parse(&text, Path::new("clipboard"))`, which returns the text with the preview: error → `The clipboard text is not a valid kubeconfig.`; zero contexts → as above. On any error the text is dropped in the task.
4. Only now `ClustersPageState.paste_text = Some(text)`; preview with `target = pasted_file_path(config_dir, first_context)`.
5. `Save and add` (`on_ok`): `ClipboardMark::of(&text)`, then `paste_text.take()` moves into `ClusterCatalog::add_pasted` (window-and-sharing.md), dialog closes. On `Added(path)` the page selects its first row and calls `reset_paste_status`.

`paste_text` is `None` again after: Save and add (moved out), Cancel, Esc or overlay click (`on_close`), a preview error, and window close (the view is dropped).

```rust
pub(crate) const PASTED_DIR: &str = "kubeconfigs";
/// `<dir>/kubeconfigs/{slug}.yaml`, slug = first context name lowercased, every char outside
/// [a-z0-9._-] → '-', trimmed to 40, empty → "pasted"; `-2`, `-3`, … while the name exists.
pub(crate) fn pasted_file_path(config_dir: &Path, first_context: Option<&str>) -> PathBuf;
/// Creates the folder, writes with `create_new` (never overwrites), `sync_all`. Unix: file 0o600, folder 0o700.
pub(crate) fn write_pasted_kubeconfig(config_dir: &Path, first_context: Option<&str>, text: &str) -> io::Result<PathBuf>;
pub(crate) fn is_app_owned(path: &Path, config_dir: &Path) -> bool;   // parent == <dir>/kubeconfigs and the name passes `is_pasted_file_name` ({slug}[-n].yaml, [a-z0-9._-]); a user file under another name is never deleted
```

`AppSettings::config_dir(cx) -> Option<&Path>` is new (0024 `WriteMode::Enabled(dir)`).

## C1 and secret rules

- The clipboard text lives in a local, then `ClustersPageState.paste_text`, then the catalog write task; never in an element, a `SharedString`, a log, a notice, or `settings.json`. `SettingsWindow` and `ClustersPageState` have no `Debug`; `ClipboardFingerprint` has none either.
- No `tracing` call in `kubeconfig_import.rs`, `cluster_catalog.rs`, or the dialogs logs text, user names, or servers; file paths and error kinds only.
- Permissions: decision 9 (Windows inherits the profile ACL; Unix 0o600 / 0o700).
