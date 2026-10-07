# 0007 · App: the YAML tab

[Back to index](README.md) · Step 3 · Modules: `yaml_view.rs` (new), `drawer.rs`, `pod_drawer.rs`, `node_drawer.rs`, `kind_drawer.rs`, `app_shell.rs`, `resource_kind.rs`, `Cargo.toml`

## Entity (`yaml_view.rs`)

```rust
/// The YAML tab of the open drawer. Dropping it aborts the request and frees the text.
pub(crate) struct YamlView {
    connection: ClusterConnection,
    object: ObjectRef,
    env: EnvValues,                    // what the editor shows; a new subject starts at Hidden
    editor: Entity<EditorState>,
    fetched_at: Option<jiff::Timestamp>, // None until the first success
    hidden_env_values: usize,
    request: YamlRequest,
}
enum YamlRequest { Running { _task: Task<()> }, Idle, Failed { message: String } }
/// Whether a fetch first waits `DRAWER_SUBJECT_DELAY`.
enum FetchStart { Debounced, Immediate }

impl YamlView {
    pub(crate) fn new(connection: ClusterConnection, object: ObjectRef,
        window: &mut Window, cx: &mut Context<Self>) -> Self; // builds the editor, fetch(Hidden, Debounced)
    pub(crate) fn is_for(&self, object: &ObjectRef) -> bool;
    pub(crate) fn is_loading(&self) -> bool;           // Running and fetched_at is None
    fn fetch(&mut self, env: EnvValues, start: FetchStart, window: &mut Window, cx: &mut Context<Self>);
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>);           // fetch(self.env, Immediate)
    fn toggle_env_values(&mut self, window: &mut Window, cx: &mut Context<Self>); // fetch(flipped, Immediate)
    fn copy(&self, cx: &mut Context<Self>);            // clipboard = editor text
}
/// `None` for a key the cluster crate cannot address (cannot happen for today's keys).
pub(crate) fn object_ref(key: &ResourceKey) -> Option<ObjectRef>;
/// The object the YAML tab should show, or `None` when the tab is not shown.
pub(crate) fn yaml_subject(selected: Option<&ResourceKey>, tab: DrawerTab) -> Option<ObjectRef>;
fn fetch_status(fetched_at: Option<jiff::Timestamp>, request: &YamlRequest, now: jiff::Timestamp) -> String;
fn shows_env_toggle(env: EnvValues, hidden_env_values: usize) -> bool; // Shown, or any hidden
```

- `object_ref`: `Pod` → `ObjectRef::new(ObjectKind::Pod, Some(ns), name)`; `Node` → `Node, None`; `Kind { kind, namespace, name }` → `kind.object()`.
- `yaml_subject`: `selected` and `shown_tab(drawer_tabs(key), tab) == Yaml`, then `object_ref`.

## Async contract

| Step | Rule |
|---|---|
| fetch | `request = Running { _task }` replaces (drops, so aborts) any running request or pending delay. One `cx.spawn_in(window, ..)`: `Debounced` first awaits `cx.background_executor().timer(DRAWER_SUBJECT_DELAY)`; then `runtime.spawn(async move { connection.object_yaml(&object, env).await })` is awaited |
| success | `this.update_in(cx, \|view, window, cx\| ..)`: `editor.set_value(text, window, cx)`, `self.env = env` (the requested one), `fetched_at = Some(now)`, `hidden_env_values`, `request = Idle`, `notify` |
| failure | `request = Failed { message: error_text(&error) }`; the editor, `env`, and `fetched_at` keep their last values, so a failed toggle does not flip the button |
| entity gone | the update fails silently; the runtime task is dropped with the GPUI task |

- **Debounce:** only the first fetch of a view waits (arrowing through rows on the YAML tab replaces the view before the delay ends, so no GET is sent). Refresh and the env toggle are explicit and start at once.
- `DRAWER_SUBJECT_DELAY: Duration = 250 ms` moves to `drawer.rs` in step 3 and replaces 0006's private `EVENT_SUBJECT_DELAY` in `app_shell.rs`; both debounces use it.
- `ObjectYaml.text` moves straight into the editor. No `tracing::` call in `yaml_view.rs`. Nothing is written to disk.

### Invariants (must hold; review points)

1. `sync_yaml_view` only assigns `self.drawer.yaml`. It never calls `cx.notify()` on the shell, and it is the only place that creates a `YamlView`. (It runs inside `render`; a notify there would re-render forever.)
2. `YamlView::new` never notifies synchronously, itself or the shell. Its only `notify` comes from the spawned task, after the delay and the GET.

## Render

- Toolbar (`h_flex`, `px_4 py_2`, `border_b_1`, `flex_shrink_0`): `fetch_status` muted `text_xs`, then on the right (`ml_auto`):
  - **Env values**: only when `shows_env_toggle`; `Button::new("yaml-env").label("Env values").small().outline().selected(env == Shown)`, disabled while `Running`; tooltip "Show the env values this view hides" / "Hide env values".
  - **Copy**: ghost small `IconName::Copy`, tooltip "Copy YAML", disabled while `fetched_at` is `None`.
  - **Refresh**: ghost small `IconName::RefreshCw`, tooltip "Fetch again", disabled while `Running`.
- `Failed`: `Alert::error("yaml-error", message).title("Cannot read the YAML")` under the toolbar.
- Body: `fetched_at` `None` and `Running` → centered `Spinner` + "Reading the YAML…"; `None` and `Failed` → only the alert; otherwise the editor filling the rest.

| `fetch_status` | Text |
|---|---|
| `None`, `Running` | `Loading…` |
| `Some`, `Running` | `Refreshing…` |
| `Some(t)`, `Idle` or `Failed` | `Fetched {format_age(t, now)} ago` |
| `None`, `Failed` or `Idle` | `` (empty) |

## Editor

- `EditorState::new(window, cx).language("yaml").line_number(true)`; folding and the Ctrl+F search panel stay at their code-editor defaults (on). Soft wrap stays on (narrow drawer).
- `Editor::new(&editor).readonly(true).bordered(false).text_xs()`, full remaining height.
- `Cargo.toml` (root): `gpui-kit = { version = "0.7", features = ["tree-sitter-yaml"] }` (defaults kept). Colors come from `theme.highlight_theme`; no literals.

## Drawer frame (`drawer.rs`)

```rust
pub(crate) enum DrawerBody { Scrolling(AnyElement), Filling(AnyElement) } // today's body is Scrolling
pub(crate) struct DrawerState { /* step 2 fields */ pub(crate) yaml: Option<Entity<YamlView>> }
```

- `drawer_frame(header, tabs, body: DrawerBody, width, cx)`: `Scrolling` keeps `id("drawer-body") flex_1 min_h_0 overflow_y_scroll p_4`; `Filling` is `flex_1 min_h_0` with no padding and no scroll (the editor scrolls itself).
- `DrawerTab::Yaml` (label `YAML`) is added per [drawer-tabs.md](drawer-tabs.md); every drawer's YAML body is `Filling(state.yaml.clone())` (empty `div` while `None`).

## Lifecycle (`app_shell.rs`)

- `render` calls `self.sync_yaml_view(window, cx)` before building the tree:
  `yaml_subject(..)` is `None` → `drawer.yaml = None`; same object → keep; else a new `YamlView` with the live connection (no live session → `None`).
- So the view lives exactly while the YAML tab of an open drawer is shown. Close, subject change, tab change, `show_screen`, namespace and context switches all drop it on the next render.
- Comparing by `ObjectRef` alone is enough because a context or namespace switch always closes the drawer first (`start_session`, `set_namespace`). A view therefore never outlives its connection's session.
- `pub(crate) fn open_yaml(&mut self, key: ResourceKey, cx)`: if `selected != Some(key)`, `self.reveal(key.clone(), cx)`. Then, only if `self.selected == Some(key)` (a loading kind list keeps the key; a vanished row clears it), set `drawer.tab = Yaml` and `notify`. Otherwise do nothing, so no other drawer opens on YAML.

## Kinds (`resource_kind.rs`)

`KindSpec.object_kind: &'static str` (0006) becomes `object: ObjectKind`. `ResourceKind::object()` is new; `object_kind()` returns `self.object().name()`, so 0006's `from_object_kind` and events code are unchanged.

## Clean YAML and Save as… (UX round 3, P32)

The toolbar has two more buttons beside Copy. **Copy clean YAML** puts a manifest for Git on the clipboard: what the editor shows without `status`, without `metadata.creationTimestamp`, `generation`, `resourceVersion`, `uid`, `managedFields`, and `selfLink`, keys sorted like kubectl, and without the `<hidden>` placeholders, which are dropped with their key (a Secret's `data` keys, a hidden env `value`). The line left of the buttons says how many left (`Copied clean YAML · 2 hidden values left out`). **Save as…** opens the save dialog on `{kind}-{name}-{time}.yaml`, then writes the same clean manifest to the chosen path (C9: nothing is written before a path is chosen; no path is traced); the line then reads `Saved to {file}`, and a failure is an alert under the toolbar. Both use `cluster::clean_yaml`; plain Copy still copies the text as shown.
