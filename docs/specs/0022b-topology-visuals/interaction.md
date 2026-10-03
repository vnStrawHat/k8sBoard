# 0022b · Hover, handles, controls, animated flow (steps 3–4)

[Back to index](README.md) · Modules: `topology_canvas.rs`, `topology_view.rs`, `topology_viewport.rs`. Decision 25 of 0022 ("no zoom buttons") is replaced by decision 9 here.

## Focus and edge emphasis (step 3)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Emphasis { Rest, Focused, Dimmed }
pub(crate) fn edge_emphasis(edge: &TopologyEdge, focus: Option<usize>) -> Emphasis;
```

- `focus = hovered.or(selected)`, where `hovered: Option<NodeId>` is a new `TopologyView` field. Card `on_hover(|is_hovered, ..|)` sets or clears it, and notifies **only when it changes**. While `drag` is not `None`, hover changes are ignored.
- `hovered` is cleared on a namespace change, a rebuild that drops the node, and Escape.
- No focus: every edge is `Rest`. Focus: the edges touching the focus node are `Focused`, all others `Dimmed`.

| Emphasis | Alpha | Width |
|---|---|---|
| Rest | `EDGE_REST_ALPHA` 0.75 | relation width |
| Focused | 1.0 | + `FOCUS_EXTRA_WIDTH` 0.6 (was `SELECTED_EDGE_EXTRA`) |
| Dimmed | `EDGE_DIM_ALPHA` 0.2 | relation width |

The arrow uses the same alpha. Cards are not dimmed; React Flow dims nothing by default, and only edges are cheap to change.

## Handles (step 3)

React Flow draws 6 px round handles where edges attach. Here:

- A second canvas, `handle_canvas`, is placed **after the cards** in the canvas div. It has no hitbox, so it takes no mouse events. It paints a dot at `curve.start` and `curve.end` of every visible edge, only at `Text` detail.
- Each dot is a round `paint_quad`, `HANDLE_SIZE` 6 × zoom, origin snapped. The fill is the kind color of the node it sits on, with a 1 px `background` border (React Flow: dark fill, light border). Duplicate dots on a shared point are drawn again; they are opaque and identical.
- A pure `handle_points(curve) -> [GraphPoint; 2]` keeps the attach rule in one place for the export (step 5).

## Controls panel (step 3)

- Bottom left of the canvas (React Flow `Controls` default `BottomLeft`, vertical): three kit `Button`s, ghost, xsmall, icon-only, with tooltips. `IconName::Plus` "Zoom in", `IconName::Minus` "Zoom out", `IconName::Maximize` "Fit" (all three are in the gpui-component default icon set).
- Styling: `background`, 1 px `border`, radius 6, `shadow_sm()`. The panel sits inside `OVERLAY_GUTTER`, so Fit never hides it. It stops the mouse-down from propagating, so a click does not start a pan.
- Zoom in and out: `viewport.zoom_at(view_w / 2, view_h / 2, ±ZOOM_BUTTON_STEPS)`, with `ZOOM_BUTTON_STEPS` = 2 (×1.21, close to React Flow's 1.2). The clamp and the below-floor rule (decision 38) come from `zoom_at`.
- Fit: the existing `fit`. The header `Fit` stays (W11 shows it).

## Animated flow (step 4)

React Flow animates an edge with `stroke-dasharray: 5` and `dashdraw 0.5s linear infinite` from `stroke-dashoffset: 10` (`xyflow` `packages/system/src/styles/init.css`). That is 20 px/s toward the target.

| Rule | Value |
|---|---|
| Which edges | `Focused` edges of the **selected** node only (not hover), at `Text` detail (zoom ≥ 0.55) |
| Look | `FLOW_DASH` (5, 5) × zoom, replacing the relation dash; the relation color, alpha 1 |
| Phase | `flow_phase(elapsed) = (elapsed_secs × FLOW_SPEED × zoom) mod (period)`, `FLOW_SPEED` 20 graph units/s; `elapsed` since `TopologyView::created` (`Instant`) |
| Next frame | in the canvas paint closure: `if needs_flow_frame(animated, cx.reduce_motion(), window.is_window_active()) { window.request_animation_frame() }` |
| `needs_flow_frame` | `animated > 0 && !reduce_motion && is_active` (pure, tested) |
| Reduced motion | `App::reduce_motion()` (gpui-pre `app.rs` l. 1141): the dashes show with phase 0 and no frames are requested |

- **Idle cost is zero.** No selection, a selection with no visible edge, zoom below 0.55, a hidden Topology (not painted), or an inactive window all mean no frame request, so nothing repaints.
- **Running cost:** `request_animation_frame` notifies `TopologyView`, so the view re-renders at the display rate while a node is selected. Only culled cards are built (0022), and the drawer is open at the same time. The step 4 AC measures the frame time; if it misses, fall back to a 30 fps `cx.spawn` timer with the same guard (decision 10).
- `cx.reduce_motion()` is not wired to the Windows setting in this GPUI version (no platform caller). The tests and screenshots set it. Open item 1.

## Screenshot

`--screen topology-selected` (step 3): Topology with the first Deployment selected (drawer open). It is captured with reduced motion on, so the frame is deterministic. It shows the emphasis, the handles, the panel, and the static flow dashes (step 4).
