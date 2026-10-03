# 0044 · Step 1: dashed line, double-click reset, remembered height

[Back to index](README.md) · Modules: `dock.rs`, `workspace.rs`, `app_shell.rs`, `settings.rs` (+ `settings_tests.rs`). Decisions 1–6. Wireframe: W8 notes 1–2, v0.5 note.

## As built

- `workspace.rs` `render_workspace`, Normal mode: `v_resizable("workspace-split").with_state(&self.dock_split)`; the dock panel is `resizable_panel().size(DEFAULT_DOCK_HEIGHT).flex_none().size_range(MIN_DOCK_HEIGHT..dock_max_height(container))`.
- Kit facts (gpui-base 0.7 `resizable`): `ResizableState::resize_panel(ix, size, window, cx)` resizes like a drag and emits `ResizablePanelEvent::Resized`; every drag end emits `Resized` (`done_resizing`); a window shrink re-clamps without an event. A group takes `with_handle_appearance(ResizeHandleRenderer)`; the renderer gets `ResizeHandleContext::state()` (`Idle`, `Hovered`, `Pressed`, `Dragging`). `gpui_kit::component::resizable::resize_handle_appearance()` is the kit look. The base band is `HANDLE_SIZE` 1 px plus `HANDLE_PADDING` 4 px on each side.

## Setting (`settings.rs`)

```rust
pub(crate) struct Settings { /* … */
    #[serde(skip_serializing_if = "DockSettings::is_empty")] pub(crate) dock: DockSettings }
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)] #[serde(default)]
pub(crate) struct DockSettings {
    /// The dock height in pixels; `None` is the default height.
    #[serde(skip_serializing_if = "Option::is_none")] pub(crate) height: Option<f32> }
```

- Key `dock.height` (0024 reserved: `Option<f32>` px, written on drag end). `settings_keys_are_the_allow_list` gains it. No version bump. No secret (0024 secret rules).

## Pure helpers (`dock.rs`, inline tests)

```rust
/// The dock height a launch starts with: the saved one when finite and positive, clamped to
/// `MIN_DOCK_HEIGHT ..= dock_max_height(viewport)`; else `DEFAULT_DOCK_HEIGHT` under the same cap.
pub(crate) fn initial_dock_height(saved: Option<f32>, viewport: Pixels) -> Pixels;
/// What a resize end stores: whole pixels, `None` for the default (a reset forgets the preference).
pub(crate) fn saved_dock_height(height: Pixels) -> Option<f32>;
/// How far above the handle the 60 % line sits; `None` before the first layout.
pub(crate) fn max_line_offset(container: Pixels, dock: Pixels) -> Option<Pixels>;
```

- `max_line_offset` = `dock_max_height(container) - dock`, at least zero; `None` when `container` is zero (`dock_max_height` is `Pixels::MAX` then).

## Restore and save (`workspace.rs`, `app_shell.rs`)

- The dock panel gets `.size(initial_dock_height(AppSettings::get(cx).dock.height, window.viewport_size().height))`. Before the first layout `container_size()` is zero and `size_range` has no upper cap, so the first frame is clamped to 60 % of the viewport here; from the next frame the kit clamps to 60 % of the measured workspace (slightly smaller: title and status bars). `render_workspace` gains a `window: &Window` argument for this. The kit uses `size` only while the state has no size of its own; afterwards `dock_split` keeps the session's height through minimize, zoom, and an empty dock (0004).
- `AppShell::new` subscribes: `cx.subscribe(&dock_split, |shell, split, _: &ResizablePanelEvent, cx| shell.save_dock_height(&split, cx))`, kept in a `_dock_split_events` field.
- `save_dock_height`: the dock panel is `sizes().get(1)` (nothing with one panel); `AppSettings::update(cx, |s| s.dock.height = saved_dock_height(size))`. `AppSettings::update` skips an unchanged file; a `--screenshot` run has writes off (0024 decision 13).
- A smaller window clamps the shown height without `Resized`, so the saved value stays the user's choice (decision 2).

## Handle renderer (`workspace.rs`)

```rust
/// The kit's divider, plus the double-click reset and, while dragging, the dashed 60 % line.
fn dock_handle_appearance(split: Entity<ResizableState>) -> ResizeHandleRenderer;
```

- The group becomes `v_resizable(..).with_handle_appearance(dock_handle_appearance(self.dock_split.clone()))`, replacing the kit appearance that `v_resizable` installs.
- The renderer returns a `relative`, full-width, 1 px wrapper holding:
  - the kit line: `resize_handle_appearance()(handle, window, cx)`, so the hairline and the pill look as before;
  - a **hit area**: an absolute child over the whole band (`top(-4 px)`, `h(9 px)`, full width) with `on_mouse_down(MouseButton::Left, ..)`: `event.click_count == 2` → `split.update(cx, |state, cx| state.resize_panel(1, DEFAULT_DOCK_HEIGHT, window, cx))`. Its `Resized` clears `dock.height` through `save_dock_height`. The hit area does not occlude, so the base drag still starts (the review confirmed it receives the press);
  - the **max line**, only when `handle.state() == ResizeHandleState::Dragging` and `split.sizes().get(1)` is `Some(dock)` and `max_line_offset(split.container_size(), *dock)` is `Some(offset)`: `deferred(div().absolute().left_0().w_full().top(-offset).border_t_1().border_dashed().border_color(theme.muted_foreground))` with a right-aligned muted `text_xs` label `max height · 60%` just above it (W8 text). Deferred so it paints over the upper panel; it exists only during a drag, when no popover is open.

## Keys

None new. Zoom (⤢, `Ctrl Shift M`), minimize (▾, `Ctrl \``), and their modes are unchanged (W8 note 4). A pop-out window (step 4) has no dock and no height.
