# 0025 · Clusters page

[Back to index](README.md) · Steps 3–4 · Modules: `cluster_form.rs` (new, no GPUI view code: rows, validation, registry edits, Test connection future), `settings_window.rs` (view), `cluster_registry.rs` (0024). Decisions 7, 12–20. Wireframe: W2 pins 3–6.

## Layout

`SettingPage::new("Clusters")`, `.description(count_text(n, m))` (`1 cluster · 1 kubeconfig file`, `10 clusters · 2 kubeconfig files`), `.title_suffix(add_cluster_button)`, `.resettable(false)`, one group with one `SettingItem::render` that draws: catalog notices with their actions (step 4), a paste status line (`Saving the pasted kubeconfig…` while `PasteStatus::Saving`), then `h_flex`: list (300 px, own scroll) | form (flex). The add button is a `Button` + `DropdownMenu`: "Import kubeconfig file…" (`PopupMenuItem::action(Box::new(ImportKubeconfig))`, so the kit draws the key), "Paste kubeconfig YAML…", separator, disabled "Watch a kubeconfig folder…", "Scan AWS EKS", "Scan Google GKE", "Scan Azure AKS" (reason "Comes in a later version"). The W2 nav count (`Clusters 10`) is not drawn (known deviation: the kit nav has no badge). Flows: [import.md](import.md).

## List (`cluster_form.rs`)

```rust
pub(crate) struct ClusterRow { pub(crate) cluster: ClusterRef, pub(crate) profile: ClusterProfile,
    pub(crate) label: String /* switcher_label */, pub(crate) meta: String, pub(crate) origin: RowOrigin }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowOrigin { Chain, Registry, AppOwned }   // AppOwned: under <config>/kubeconfigs/
pub(crate) struct ClusterGroup { pub(crate) title: &'static str, pub(crate) rows: Vec<ClusterRow> }
pub(crate) fn cluster_groups(kubeconfigs: &[&Kubeconfig], registry: &ClusterRegistry,
    is_chain_source: impl Fn(&Path) -> bool, owned_dir: Option<&Path>) -> Vec<ClusterGroup>;
pub(crate) fn count_text(clusters: usize, files: usize) -> String;
```

- Groups in order "Production", "Staging", "Development · Local" (DEV and LOCAL share it, W2); empty groups are skipped; the header shows the count.
- Within a group: registered entries in registry order, then the rest in load order.
- Row: `environment_badge`, mono `label`, muted `meta` = `"{auth kind} · {source file name}"` (W2 `EKS · ~/.kube/config` becomes `exec: aws · config`; known deviation, the full path is in the form). The file name is the text after the last `/` or `\` of the path string (no `std::path` split; structure.md).
- Click selects (`ClustersPageState.selected: Option<ClusterRef>`); default = the active cluster, else the first row. A selection whose row disappears falls back to the first row.

## Form

| Section · field | Control | Reads / writes |
|---|---|---|
| General · Display name | `Input` | `entry.display_name` (validated) |
| General · Environment | `Select`: "Auto ({guess badge})", Production, Staging, Development, Local | `entry.environment` (`None` = Auto) |
| General · Default namespace | `Input`, placeholder "none" | `entry.default_namespace` (validated; empty → `None`) |
| Connection · Source | read-only text | `{source path} · context {name}` |
| Connection · Server | read-only text | `connection_info.server` or "—" |
| Connection · Authentication | read-only text | `connection_info.auth` label |
| Connection · Test connection | `Button` + result line | decision 16 |
| Safety · Open as read-only | `Switch` + hint `Default: on for Production. Applies when editing actions arrive (0030).` (decision 17) | `entry.read_only` (`Some(value)`); only Reset returns it to `None` |
| footer | "Reset to defaults" (ghost), "Remove from k8sBoard" (danger) | below |

Inputs are `InputState`s owned by `ClustersPageState`, recreated (with the stored values) when the selection changes; their `InputEvent::Change` runs validation, then `AppSettings::update` via `entry_mut` when valid. All writes go through `cluster_form` helpers so they are testable without GPUI.

## Validation (`cluster_form.rs`)

```rust
pub(crate) fn validate_display_name(text: &str, cluster: &ClusterRef, rows: &[ClusterRow])
    -> Result<Option<String>, FieldError>;      // Ok(None) = empty → context name
pub(crate) fn validate_namespace(text: &str) -> Result<Option<String>, FieldError>;
pub(crate) struct FieldError(pub(crate) SharedString); // shown under the field in theme.danger
```

| Field | Rule | Message |
|---|---|---|
| Display name | ≤ 64 chars after trim | `Use at most 64 characters.` |
| Display name | no control chars (`char::is_control`) | `Remove line breaks and tabs.` |
| Display name | unique, case-insensitive, among other rows' labels | `Another cluster is already shown as '{name}'.` |
| Default namespace | `^[a-z0-9]([-a-z0-9]*[a-z0-9])?$`, ≤ 63 (hand-written check, no regex dep) | `Use 1–63 lowercase letters, digits, or '-', starting and ending with a letter or digit.` |

The error clears on the next valid edit or a selection change. The stored value stays the last valid one. These rules block **user edits only**; an import may bring a context whose name equals an existing display name (a preview warning, [import.md](import.md)).

## Test connection

```rust
pub(crate) const TEST_CONNECTION_TIMEOUT: Duration = Duration::from_secs(15);
/// Runs on the tokio runtime. `timeout` (`tokio::time::timeout`) covers open + version;
/// the latency covers `server_version` only.
pub(crate) async fn test_connection(kubeconfig: Arc<Kubeconfig>, context: String, timeout: Duration) -> TestState;
pub(crate) enum TestState { Idle, Running, Connected { version: String, latency_ms: u64 }, Failed(SharedString) }
```

- `ClusterConnection::open` (no network), then `Instant::now()`, `server_version()`, `elapsed()`.
- Text: `Testing…` · `Connected · {version} · {ms} ms` (theme success) · `Failed: {error}` (theme danger). `error` is the top-level `ClusterError` `Display` only, never the `source()` chain. The timeout maps to `ClusterError::TimedOut { context, action: "testing the connection" }`.
- The kubeconfig is the row's `Arc<Kubeconfig>` from the catalog. A selection change drops the `RuntimeTask` (aborts it).

## Reset and Remove

```rust
pub(crate) fn reset_entry(registry: &mut ClusterRegistry, cluster: &ClusterRef);   // drops the entry
/// Drops the path, every entry with that `kubeconfig`, and a matching `last_used`.
pub(crate) fn remove_kubeconfig(registry: &mut ClusterRegistry, path: &Path);
```

- **Reset to defaults**: enabled when an entry exists; removes it (name, env, lock, namespace back to defaults). No dialog.
- **Remove from k8sBoard**: `Chain` rows → disabled, reason `Comes from KUBECONFIG or ~/.kube/config; edit that instead.` Other rows → alert dialog:
  - title `Remove {file name} from k8sBoard?`
  - body `{n} clusters from this file leave the list: {labels}.` plus `The file itself is not changed.` (Registry) or `k8sBoard created this file when you pasted it; it will be deleted.` (AppOwned)
  - OK `Remove` (danger) → `ClusterCatalog::remove_kubeconfig(path)` (window-and-sharing.md). AppOwned: the file is deleted **first**; entries go only after the delete succeeded; on failure the rows stay and the notice reads `Could not delete {name}; it is still listed`.
- A running session from a removed file keeps running until the user switches (decision 24).
