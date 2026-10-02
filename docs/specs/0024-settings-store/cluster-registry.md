# 0024 · Kubeconfig chain and cluster registry

[Back to index](README.md) · Steps 1, 3, 5 · Modules: `crates/cluster/src/kubeconfig.rs`, `crates/app/src/cluster_registry.rs` (new), `launch_options.rs`, `app_shell.rs`, `title_bar.rs`, `resource_actions.rs`. Decisions 5, 17–22, 27.

## Cluster crate (step 1)

```rust
/// Loads `paths` in order and merges them like kubectl (first file wins per named entry and
/// for current-context). Blocking file I/O. Err only when no file loads (the first error).
pub fn load(paths: &[PathBuf]) -> Result<LoadedKubeconfig, KubeconfigError>; // replaces load(&Path)
pub struct LoadedKubeconfig { pub kubeconfig: Kubeconfig, pub skipped: Vec<KubeconfigError> }
impl Kubeconfig { pub fn sources(&self) -> &[PathBuf]; }  // the files that loaded; replaces path()
pub struct ContextSummary { /* existing fields */ pub source: PathBuf } // file that defined it
/// Private. `origins`: context name → defining file, filled while merging; a name missing
/// from it gets `sources[0]`. Tests call it with `vec!["fixture.yaml".into()]` and an empty map.
fn from_document(sources: Vec<PathBuf>, document: kube::config::Kubeconfig,
    origins: &HashMap<String, PathBuf>) -> Self;
```

- Per file: `kube::config::Kubeconfig::read_from(path)` (keeps per-file absolute credential paths). Then **pre-check** (decision 17): if `merged.kind` and `next.kind` are both `Some` and differ, or the same for `api_version`, push `Incompatible { path }` and continue. Only then `merged = merged.merge(next)`. With the pre-check, kube 4.2's `merge` has no other error; should it still return `Err`, the whole `load` returns `Incompatible { path }` (the accumulator is gone; no `expect`).
- Before merging a file, record its context names not yet in `origins` → that path.
- New variant `KubeconfigError::Incompatible { path }`: "kubeconfig '{path}' has a different kind or apiVersion; skipped".
- `ContextNotFound` and `NoContextSelected`: `path: PathBuf` → `paths: Vec<PathBuf>`, displayed joined with `, `.
- `Kubeconfig` `Debug` prints sources and context names only (unchanged rule).
- `examples/probe.rs`: `Kubeconfig::load(&[args.kubeconfig])`, prints each skipped error and `sources()`.

## Launch files (`launch_options.rs`, step 3)

```rust
/// The kubectl chain: `--kubeconfig`, else every non-empty KUBECONFIG entry, else
/// `<home>/.kube/config`. All absolute. Empty → "no kubeconfig found".
pub(crate) fn kubeconfig_chain(flag: Option<PathBuf>, kubeconfig_env: Option<OsString>,
    home: Option<PathBuf>) -> Vec<PathBuf>;
/// Registry files to load standalone: absolute, in registry order, without chain members and duplicates.
pub(crate) fn standalone_files(registered: &[PathBuf], chain: &[PathBuf]) -> Vec<PathBuf>;
```

Delete `kubeconfig_path`, `has_ignored_kubeconfig_entries`, `IGNORED_KUBECONFIG_NOTE`, `kubeconfig_error_message` and their tests. One background task loads `load(&chain)` and then `load(&[file])` per standalone file. `KubeconfigState::Loaded(Vec<Arc<Kubeconfig>>)` holds the chain first (if it loaded), then each registry file. Every skipped or failed file becomes a shell notice `Skipped kubeconfig: {error}` (decision 30). None loaded → `Failed` with the chain error.

## Registry model (`cluster_registry.rs`)

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)] #[serde(default)]
pub(crate) struct ClusterRegistry {
    pub(crate) kubeconfigs: Vec<PathBuf>,        // user-added files (0025 Import); hand-editable now
    pub(crate) clusters: Vec<ClusterEntry>,      // order = registry order (0025 drag → Ctrl 1–9)
    pub(crate) last_used: Option<ClusterRef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]  // no container default; Hash: map key (0026)
pub(crate) struct ClusterRef { pub(crate) kubeconfig: PathBuf, pub(crate) context: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]  // no container default
pub(crate) struct ClusterEntry {
    #[serde(flatten)] pub(crate) cluster: ClusterRef,
    pub(crate) display_name: Option<String>,       // each Option: default + skip_serializing_if
    pub(crate) environment: Option<Environment>,   // None → guessed
    pub(crate) read_only: Option<bool>,            // None → 0030 default (on for PROD)
    pub(crate) default_namespace: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClusterProfile { pub(crate) display_name: String,
    pub(crate) environment: Environment, pub(crate) default_namespace: Option<String> }
impl ClusterRef { pub(crate) fn of(summary: &ContextSummary) -> Self; } // clones path and name
impl ClusterRegistry {
    pub(crate) fn entry(&self, cluster: &ClusterRef) -> Option<&ClusterEntry>;
    pub(crate) fn entry_mut(&mut self, cluster: &ClusterRef) -> &mut ClusterEntry; // appends when missing
    pub(crate) fn profile(&self, summary: &ContextSummary) -> ClusterProfile;
}
/// Switcher text: the display name, plus ` · {source file name}` when another loaded
/// kubeconfig has a context with the same name.
pub(crate) fn switcher_label(profile: &ClusterProfile, summary: &ContextSummary, is_duplicate_name: bool) -> String;
```

- Matching: `entry.cluster == ClusterRef::of(summary)` (component-wise path equality: `/` and `\` match on Windows; case must match).
- `profile`: `display_name` = entry value (trimmed, non-empty) else context name; `environment` = entry value else `guess_environment(&summary.name, &summary.cluster)`; `default_namespace` = entry value.
- `read_only` has no reader in 0024 (decision 27); until 0030 only the allow-list test covers the key.

## Start selection (step 3)

```rust
pub(crate) enum StartChoice { Cluster(ClusterRef), RequestedMissing, CurrentContext }
/// `contexts` in load order (chain first). Requested name → its first match; else an exact
/// `last_used` match; else CurrentContext.
pub(crate) fn start_choice(requested: Option<&str>, last_used: Option<&ClusterRef>,
    contexts: &[&ContextSummary]) -> StartChoice;
```

`RequestedMissing` → the first loaded kubeconfig's `resolve_context(Some(name))` error (as today). `CurrentContext` → its `resolve_context(None)`. A stale `last_used` is ignored silently. With `--kubeconfig X` and no `--context`, a `last_used` in X wins over X's `current-context` (AC 9).

## Session start and switch (`app_shell.rs`, steps 3 and 5)

- `AppShell` keeps `active: Option<ContextSummary>`; `switch_context(&str)` becomes `switch_cluster(&ClusterRef)`, which finds the owning `Arc<Kubeconfig>` by `source` + name.
- When the session first becomes Live (`on_session_changed` sees `SessionPhase::Live`; a `has_reported_live` flag reset per session): build `let cluster = ClusterRef::of(summary);` first (it clones the context name and path, so no borrow of `self` or the kubeconfig crosses the `cx` borrow), then `AppSettings::update(cx, |s| s.registry.last_used = Some(cluster))`. Never in `start_session` (decision 22).
- Namespace at start: `--namespace` (first session only) > `profile.default_namespace` as `NamespaceScope::of_namespaces(vec![ns])` > today's default. `switch_cluster` uses the target's default namespace instead of `None`.
- `AppShell::new`: `_settings_observer: Subscription = cx.observe_global::<AppSettings>(|_, cx| cx.notify())`.
- The switcher lists every context of every loaded kubeconfig with `switcher_label`; checked = `ClusterRef::of(active)`.

## "Set as default namespace" (`resource_actions.rs`, step 5)

- `kind_menu` gains `default_namespace: Option<&str>` (the active profile's). For `ResourceKind::Namespaces`, before the change-action separator: an enabled item `Set as default namespace`, `checked` when the row name equals it. Click (clone the row name into the closure first) → `AppShell::toggle_default_namespace(name)`: `Some(name)`, or `None` when already set, via `entry_mut(&ClusterRef::of(active))`.
- No cluster call; the current scope does not change (the default applies at the next start or switch).
- An entry missing `kubeconfig` or `context` makes the file corrupt (decision 6).
