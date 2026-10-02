# 0024 · Persisted prefs

[Back to index](README.md) · Steps 2 and 4 · Modules: `settings.rs`, `main.rs`, `launch_options.rs`, `table_view.rs`, `pod_table.rs`, `node_table.rs`, `kind_table.rs`, `app_shell.rs`. Decisions 28, 29.

## Example file

```json
{
  "version": 1,
  "theme": "dark",
  "registry": {
    "kubeconfigs": ["D:/kube/onprem.yaml"],
    "clusters": [
      { "kubeconfig": "D:/TrungKFC-Research/Rust/k8sBoard/monitor-uat-readonly.yml",
        "context": "readonly@Monitor", "display_name": "uat-monitor",
        "environment": "production", "read_only": true, "default_namespace": "monitoring" }
    ],
    "last_used": { "kubeconfig": "D:/TrungKFC-Research/Rust/k8sBoard/monitor-uat-readonly.yml",
                   "context": "readonly@Monitor" }
  },
  "tables": {
    "pods": { "sort": { "column": "Restarts", "direction": "descending" }, "hidden": ["Node"] },
    "deployments": { "hidden": ["Strategy"] }
  }
}
```

## Theme (step 2)

- `ThemePreference` replaces `launch_options::ThemeChoice`. `--theme system|light|dark`; USAGE text updated.
- `main.rs`: `apply_theme(options.theme.unwrap_or(AppSettings::get(cx).theme), cx)`; `System` keeps `Theme::sync_system_appearance`.
- No writer in 0024 (0025 Appearance writes `theme`).

## Table prefs (step 4, 0009)

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)] #[serde(default)]
pub(crate) struct TablePrefs { pub(crate) sort: Option<SavedSort>, pub(crate) hidden: Vec<String> }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedSort { pub(crate) column: String, pub(crate) direction: SortDirection }
// SortDirection gains Serialize/Deserialize, rename_all = "lowercase".
pub(crate) fn screen_key(screen: Screen) -> &'static str;  // "pods" | "nodes" | kind.plural()
impl TableView {
    /// Column names are the plan's `KindColumn::name`s, by logical index.
    pub(crate) fn apply_prefs(&mut self, prefs: &TablePrefs, plan: &ColumnPlan);
    pub(crate) fn prefs(&self, plan: &ColumnPlan) -> TablePrefs;
}
```

- `apply_prefs`: map each name to its first logical index; unknown names are dropped; `plan.flexible` is never hidden; a sort on an unknown column → `None`.
- `prefs`: hidden in logical order. The saved list replaces the screen's defaults (Pods hides CPU by default); an empty entry shows every column. Known ceiling: a column that later becomes hidden by default stays visible for users who already have a saved entry.
- **Read**: when a view is created. Pods and Nodes: `AppShell::new` passes `AppSettings::get(cx).tables` entries to the delegate constructors. Kinds: `KindTableDelegate` keeps a startup copy `saved: BTreeMap<String, TablePrefs>` and applies it in `new_view(kind)`. A view created later in the session already holds the latest state, so the startup copy is enough.
- **Write**: `AppShell::cycle_sort` and `toggle_column` call `persist_table_prefs(cx)` after `update_view`: read the visible view and plan → `AppSettings::update(cx, |s| { s.tables.insert(key, prefs); })`. Nothing else writes table prefs.
- `reset_filter` / `clear_filter` (context switch, Clear filters) keep sort and hidden, as today.

## Reserved sections (later specs add them; API contract)

Each later spec adds one `#[serde(default)]` field to `Settings` (or to `ClusterEntry`), reads it with `AppSettings::get(cx)`, writes it with `AppSettings::update`, extends the key allow-list test, and needs no version bump. Names are fixed now so specs agree:

| Key | Type, default | Owner | Use |
|---|---|---|---|
| `secrets.clipboard_clear_seconds` | `u32`, 30; 0 = never | 0016 | replaces `CLIPBOARD_CLEAR_DELAY` |
| `logs.export_dir` | `Option<PathBuf>` | 0019 / 0021 / 0022 | start folder of the C9 save dialog; updated after a confirmed save |
| `issues.watch_tls_secrets` | `bool`, true | 0020 (decision 11) | opt out of the cluster-wide TLS Secrets watch (C13 budget) |
| `topology.group_by` | enum, `app` | 0022 (decision 27) | last Group by choice |
| `topology.pins` | map `"{context}/{namespace}"` → map node id → `{x, y}` | 0022 (decision 23) | dragged positions; cap 2,000 pins per key |
| `appearance.density` | `"compact"`(28) / `"comfortable"`(36) | 0025 | row height |
| `dock.height` | `Option<f32>` px | 0019 or 0025 | remembered dock height, written on drag end |
| `registry.clusters[].color`, `.metrics_source`, `.allow_node_shell`, `.confirm` | per W2 form | 0025 / 0030 / 0037 | W2 Clusters fields; `confirm` is `"type-name"` or `"click"`, absent = environment default (0030) |
| `port_forward.presets` | list | 0035 | saved forwards |

Pin keys and any new map keys use context names, namespaces, and object names only.
