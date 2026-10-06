# 0004 · Dock layout: split, tabs, resize, zoom

[Back to index](README.md) · Modules: `log_dock.rs`, `workspace.rs`, `app_shell.rs`

## Workspace split (kit resizable panels)

The resize uses `gpui_kit::component::resizable::{v_resizable, resizable_panel, ResizableState}`. These are the gpui-base 0.7 panels with the kit's handle appearance. No hand-rolled drag code is needed.

```text
no tabs     upper (flex_1)                                     -> the 0003 layout, unchanged
Normal      v_resizable("workspace-split").with_state(&self.dock_split)
              .child(resizable_panel().child(upper))                                      // flexible
              .child(resizable_panel().size(DEFAULT_DOCK_HEIGHT).flex_none()
                     .size_range(MIN_DOCK_HEIGHT..dock_max_height(container)).child(log_dock))
Minimized   v_flex: upper (flex_1) + log_dock (tab bar only, 34 px, flex_shrink_0)
Zoomed      log_dock alone, size_full; `upper` is not rendered (the table and drawer entities persist)
```

- `upper` is a `v_flex().relative().size_full()` holding the header, the banner, the body, and the drawer. The drawer moves into `upper`, so it covers the table but never the dock (decision 11). Its 0003 behavior is otherwise unchanged.
- `AppShell` owns:
  - `dock_split: Entity<ResizableState>` (`cx.new(|_| ResizableState::default())`). It survives zoom and minimize, so the Normal height comes back unchanged, because the group is rebuilt with the same 2 panels;
  - `log_dock: Entity<LogDock>`, created before the tables, which receive `log_dock.downgrade()`;
  - an observer on `log_dock` that calls `cx.notify()`, because the workspace layout depends on its mode and tab count.
- **The 60% cap:**
  - `container = self.dock_split.read(cx).container_size()`, the measured workspace height from the previous frame;
  - the panel's `size_range` applies the cap both to dragging (`panel_size_range`) and at render (`max_h`). When the window shrinks, the next prepaint adjusts the sizes to the container.
- **The handle** is the kit's own: a hairline with a pill on hover or drag (W8 note 1). It is drawn by the panel, so there is no separate grip element and no `on_drag_move` listener.
- Double-click reset was dropped here (the kit handle has no click hook; `ResizableState::reset_panel` keeps the current size). Done by 0044 through a custom handle renderer that calls `resize_panel(1, 280 px)`.

```rust
const DEFAULT_DOCK_HEIGHT: Pixels = px(280.);
const MIN_DOCK_HEIGHT: Pixels = px(120.);
const MAX_DOCK_FRACTION: f32 = 0.6;
/// 60% of the measured workspace, never below the minimum (so the range stays valid);
/// `Pixels::MAX` before the first layout, when the container is still zero.
fn dock_max_height(container: Pixels) -> Pixels;
```

## `LogDock` state (`log_dock.rs`)

```rust
pub(crate) struct LogDock {
    tabs: Vec<Entity<LogTab>>,
    active: Option<usize>,      // None exactly when `tabs` is empty
    mode: DockMode,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DockMode { Normal, Minimized, Zoomed }

impl LogDock {
    pub(crate) fn new() -> Self;
    /// Activates the pod's tab if one exists, else adds one. Minimized becomes Normal.
    pub(crate) fn open(&mut self, connection: ClusterConnection, target: LogTarget,
        window: &mut Window, cx: &mut Context<Self>);
    pub(crate) fn close_tab(&mut self, index: usize, cx: &mut Context<Self>);
    pub(crate) fn close_all(&mut self, cx: &mut Context<Self>);   // context switch
    pub(crate) fn unzoom(&mut self, cx: &mut Context<Self>);      // navigation click
    pub(crate) fn set_mode(&mut self, mode: DockMode, cx: &mut Context<Self>);
    pub(crate) fn mode(&self) -> DockMode;
    pub(crate) fn has_tabs(&self) -> bool;
}
/// The new active index after closing `closed`: the right neighbor, else the left; None when empty.
fn active_after_close(active: usize, closed: usize, remaining: usize) -> Option<usize>;
```

- Tabs are matched by `(namespace, pod)` (`LogTab::is_for`). Dropping a `LogTab` entity drops its subscription, which aborts the stream ([log-tab.md](log-tab.md)).
- Closing the last tab resets `mode` to `Normal`.
- `LogDock::render` draws the tab bar, then the active tab (`flex_1`, `min_h_0`). In Minimized mode it draws the tab bar only. It has a top border, and a shadow when Normal.

## Tab bar (34 px, `theme.muted` background, bottom border)

| Item | Look | Behavior |
|---|---|---|
| One tab per `LogTab` | `FileText` icon, a status dot, `{workload}-{suffix}/{container}` (`pod_tab_name`; a StatefulSet pod keeps its ordinal) in mono `text_xs` (max 260 px, end ellipsis), ✕ (`X`, ghost xsmall). The active tab has `theme.background` and a border on three sides | click: activate, and Minimized becomes Normal. ✕: `close_tab` |
| Spacer | `flex_1` | |
| Zoom | `Maximize2`, or `Minimize2` when zoomed; tooltip "Zoom in" / "Zoom out" | Normal or Minimized → Zoomed; Zoomed → Normal |
| Minimize | `ChevronDown`, or `ChevronUp` when minimized; tooltip "Minimize" / "Restore" | Normal or Zoomed → Minimized; Minimized → Normal |

- The status dot is `tone_color(StatusTone, cx)` (reused from `status_tone.rs`):
  - Connecting: Info;
  - Streaming: Ok;
  - Ended: Done;
  - Failed: Bad.
- The kit `TabBar` may be used if its `Tab` takes a suffix element for ✕. Otherwise use plain `h_flex` tabs.

## Interactions

| Event | Dock |
|---|---|
| "View logs" in a menu | `open`. Zoomed stays zoomed and shows the activated tab |
| `AppShell::show_screen` (navigation click) | `unzoom`. Tabs are kept (W8b note 1) |
| `set_namespace` | no change. A tab streams one pod, whatever the scope |
| `start_session` (context switch) | `close_all` |
| Drawer opens or closes | no change |

## Shell later (not built)

Shell will turn `tabs` into `Vec<DockTab>` with `enum DockTab { Logs(Entity<LogTab>), Shell(Entity<ShellTab>) }`. The tab bar, split, zoom, and minimize logic stays as it is. No trait is added now, because there is only one implementation.
