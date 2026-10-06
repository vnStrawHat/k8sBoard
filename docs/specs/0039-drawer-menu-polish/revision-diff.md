# 0039 · Deployment revision diff

[Back to index](README.md) · Step 3 · Modules: `crates/cluster/src/object_yaml.rs`, `crates/app/src/revision_diff.rs` (new), `live_sections.rs`, `yaml_edit_panels.rs`, `app_shell.rs`, `launch_options.rs`, `screenshot.rs`. Decisions 8–11. Wireframe: W7 Deployments (Revisions list; note "history with diff and rollback").

## Cluster crate: one read helper

```rust
impl ClusterConnection {
    /// One GET of a ReplicaSet, reduced to its masked `spec.template` as YAML (no hidden-count
    /// header; `hidden_env_values` says what `EnvValues::Hidden` hid). Read-only.
    pub async fn pod_template_yaml(&self, replica_set: &ObjectRef, env: EnvValues) -> Result<ObjectYaml, ClusterError>;
}
/// Pure: `mask_object(object, env, |_| 0)`, then `/spec/template`, minus
/// `metadata.labels["pod-template-hash"]` and a null `metadata.creationTimestamp`, then `yaml_text`.
/// `Err` (fixed text) when there is no template.
pub(crate) fn pod_template_text(object: Value, env: EnvValues) -> Result<ObjectYaml, &'static str>;
```

- Uses `get_object(object, ACTION)` (the existing GET path; same RBAC as the YAML tab: `get replicasets`).
- No `Debug` on the text, no tracing of it (the `ObjectYaml` rule).
- `ObjectRef::new(ObjectKind::ReplicaSet, Some(namespace), name)`.

## Buttons (`live_sections.rs`, Revisions section)

`revision_element` gets a `DIFF_BUTTON_SLOT` (already defined for Helm) after the Roll back slot. Non-current rows show `Diff` (ghost, xsmall); the current row leaves the slot empty. The button stops propagation (the row reveals its ReplicaSet on click) and calls:

```rust
// revision_diff.rs
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RevisionSide { pub(crate) replica_set: String, pub(crate) revision: Option<u64>, pub(crate) tag: Option<String> }
// `revision` is `ReplicaSetSummary.revision` (the annotation text) parsed with `str::parse::<u64>().ok()`, as `revision_rows` does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RevisionDiffRequest {
    pub(crate) deployment: ResourceKey,     // the drawer subject
    pub(crate) older: RevisionSide,         // the lower revision number left (decision 9)
    pub(crate) newer: RevisionSide,
}
/// Orders the clicked revision and the current one into (older, newer). Pure.
pub(crate) fn diff_request(deployment: ResourceKey, clicked: RevisionSide, current: RevisionSide) -> RevisionDiffRequest;
impl AppShell { pub(crate) fn open_revision_diff(&mut self, request: RevisionDiffRequest, window: &mut Window, cx: &mut Context<Self>); }
```

The current side comes from `revision_rows` (`is_current`); with no current row the `Diff` buttons are hidden. The key is the drawer row's own (`ResourceKey::of_row`), not taken from `RollBackGate`, so Diff works where Roll back is not permitted.

## Dialog (`RevisionDiffView`, entity in `revision_diff.rs`)

`open_revision_diff` takes the connection of the live session (`None` while not connected → nothing opens), creates the view, and `window.open_dialog(cx, …)` with it as the child (width as the Edit YAML diff, height ~70 % of the window, Esc closes).

| Part | Content |
|---|---|
| Title | `Revision diff · deployment/{name}` |
| Subtitle | `rev {old} · {tag} → rev {new} · {tag}` (`(current)` after the current side) |
| Toolbar | `Show env values` / `Hide env values` toggle, shown only when either side hid some (`hidden_env_values > 0`), like the YAML tab (`shows_env_toggle`) |
| Body | Loading: spinner `Loading revisions…`; Failed: `Could not load rev {n}: {error text}` (`error_text` as the YAML tab); Same (equal texts, nothing hidden): `The pod templates of the two revisions are the same.`; Same with `hidden_env_values > 0` on either side: `No visible difference; env values are hidden`, shown next to the `Show env values` toggle (hidden values may differ); Diff: wrapping `list` of `diff_row_element(row)` |

```rust
enum DiffState { Loading { _task: Task<()> }, Failed(SharedString), Ready(Rc<[DiffRow]>) }
pub(crate) struct RevisionDiffView { request: RevisionDiffRequest, connection: ClusterConnection,
    env: EnvValues, state: DiffState, scroll: UniformListScrollHandle }
```

## Async contract

1. `cx.spawn` → `ClusterRuntime::spawn` one task that `try_join`s the two `pod_template_yaml` GETs on tokio (two requests, concurrent).
2. Back on the GPUI side: `cx.background_executor().spawn(diff_rows(&older.text, &newer.text))` (the `yaml_diff` deadline applies), then `update` the view to `Ready`; one `cx.notify()`.
3. The env toggle drops the running task (state `Loading` replaces it) and starts both GETs again with the other `EnvValues`.
4. Closing the dialog drops the entity and so the task; nothing is cached (decision 11).

## Shared diff row (`yaml_edit_panels.rs`)

`YamlEditView::diff_rows_in` builds each row inline; extract `pub(crate) fn diff_row_element(row: &DiffRow, cx: &App) -> Div` (line numbers, sign, tint from `theme.danger` / `theme.success` with `ROW_TINT`, folded text) and call it from both views. No visual change to Edit YAML.

## Screenshot

`--screen revision-diff`: a fixture `RevisionDiffView` built `Ready` from two fixed template texts (no connection call), in the dialog over the Deployments screen.
