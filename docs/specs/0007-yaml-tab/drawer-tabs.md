# 0007 · App: tab bars in every drawer

[Back to index](README.md) · Step 2 (no YAML yet) · Modules: `drawer.rs`, `pod_drawer.rs`, `node_drawer.rs`, `kind_drawer.rs`, `workspace.rs`, `app_shell.rs`, `launch_options.rs`, `screenshot.rs`

## Tabs (`drawer.rs`)

```rust
/// Replaces `PodDrawerTab`. Step 3 adds `Yaml` between `Containers` and `Events`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawerTab { Overview, Containers, Events }

/// The tabs a drawer shows, in wireframe order.
pub(crate) fn drawer_tabs(key: &ResourceKey) -> &'static [DrawerTab];
/// `tab` when the drawer has it, else `Overview`.
pub(crate) fn shown_tab(tabs: &[DrawerTab], tab: DrawerTab) -> DrawerTab;
/// The underline tab bar shared by every drawer. A click calls `AppShell::set_drawer_tab`.
pub(crate) fn drawer_tab_bar(tabs: Vec<(DrawerTab, SharedString)>, shown: DrawerTab,
    cx: &Context<AppShell>) -> AnyElement;
```

| `ResourceKey` | Step 2 tabs | Step 3 tabs |
|---|---|---|
| `Pod` | Overview, Containers, Events | Overview, Containers, YAML, Events |
| `Node` | Overview, Events | Overview, YAML, Events |
| `Kind { kind: Events, .. }` | Overview | Overview, YAML |
| `Kind { .. }` | Overview, Events | Overview, YAML, Events |

- An event's drawer has a single tab in step 2: render no tab bar when `tabs.len() == 1`.
- `drawer_tab_bar` is the body of today's pod `tab_bar` made generic: `TabBar::new("drawer-tabs").underline()`, `selected_index` = position of `shown`, `.prefix(div().w_4())`, one `Tab::new().label(label)` per entry; `on_click` maps the index back through a captured `Vec<DrawerTab>`.
- Labels: `Overview`; `Containers {n}`; `YAML` (step 3); `events_title(list)` (0006).

## `DrawerState`

```rust
pub(crate) struct DrawerState {
    pub(crate) tab: DrawerTab,
    pub(crate) is_expanded: bool,
    pub(crate) selected_container: Option<String>,
    // step 3: pub(crate) yaml: Option<Entity<YamlView>>
}
```

- Doc comment: the tab and the expanded flag survive a subject change on the same screen; `show_screen` resets the tab to Overview.
- `AppShell::show_screen` sets `self.drawer.tab = DrawerTab::Overview` before `close_drawer`. So a `reveal` (0006) also lands on Overview.
- Every renderer uses `shown_tab(drawer_tabs(key), state.tab)`, never `state.tab` directly.

## Drawers

| Drawer | Change |
|---|---|
| `pod_drawer` | uses `drawer_tab_bar`; the pod-only `tab_bar` and its index mapping are deleted. Bodies: Overview, Containers, Events (0006) |
| `node_drawer(node, state: &DrawerState, session, cx)` | tab bar; Overview = today's body; Events = `recent_events(list, cx)`. The 0006 Events section is removed from Overview. Gains `ExpandToggle`; width `state.width()` |
| `kind_drawer(kind, row, state: &DrawerState, live, session, cx)` | same: Overview = sections, related pods, labels (no Events section); Events tab only when `event_subject(&key)` is `Some`. Gains `ExpandToggle`; width `state.width()` |
| `workspace.rs` `render_drawer` | passes `&self.drawer` to all three |

- Remove the "Only the Overview exists, so there is nothing to expand" and "One column is enough for a node" comments with their `expand: None`.
- `DrawerHeader.expand` becomes a plain `ExpandToggle` (no caller passes `None` any more) and `header_row` drops its `when_some`. Style rule: no obsolete paths.

## Launch screens (`launch_options.rs`)

```rust
pub(crate) enum LaunchScreen {
    Pods, Nodes, LogsDock, LogsZoomed, Kind(ResourceKind),
    PodDrawer(DrawerTab),               // pod-drawer, pod-containers, pod-events
    NodeDrawer(DrawerTab),              // node-drawer, node-events
    KindDrawer(ResourceKind, DrawerTab), // <plural>-drawer, <plural>-events
}
impl LaunchScreen { pub(crate) fn drawer_tab(self) -> Option<DrawerTab>; }
```

- This folds 0006's `PodContainers` and `PodEvents` into `PodDrawer(tab)`. Parse: the fixed pod and node names first, then `<plural>-drawer` → `Overview`, `<plural>-events` → `Events`. `events-events` is accepted and opens on Overview through `shown_tab` (harmless).
- `AppShell::new`: `drawer.tab = screen.drawer_tab().unwrap_or(Overview)`; `is_expanded` only for `PodDrawer(Containers)` (W4b, as today).
- `apply_pending_launch_screen` and `settle_input` match the new variants; behavior is unchanged.
- `USAGE` lists `pod-drawer|pod-containers|pod-events|node-drawer|node-events|<kind>-drawer|<kind>-events`.
