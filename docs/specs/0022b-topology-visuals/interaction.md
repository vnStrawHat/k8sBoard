# 0022b · Hover, handles, controls, animated flow (steps 3–4)

[Back to index](README.md) · Modules: `topology_canvas.rs`, `topology_card.rs`, `topology_view.rs`, `topology_viewport.rs`. Decision 25 of 0022 ("no zoom buttons") is replaced by decision 9 here.

## Focus and edge emphasis (step 3)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Emphasis { Rest, Focused, Dimmed }
pub(crate) fn edge_emphasis(edge: &TopologyEdge, focus: Option<usize>) -> Emphasis;
```

- `focus = hovered.or(selected)`, where `hovered: Option<NodeId>` is a new `TopologyView` field. Card `on_hover(|is_hovered, ..|)` sets or clears it, and notifies **only when it changes**. During a drag the change is recorded but not painted (the cards move under the pointer); `finish_drag` repaints once. A `hovered` node that is not in `visible_nodes` is ignored.
- `hovered` is cleared on a namespace change, a rebuild that drops the node, and Escape.
- No focus: every edge is `Rest`. Focus: the edges touching the focus node are `Focused`, all others `Dimmed`. The `Focused` edges are painted last (`sort_focused_last`), over the others (UX walk M22).

| Emphasis | Alpha | Width |
|---|---|---|
| Rest | `EDGE_REST_ALPHA` 0.8 | relation width (1.5) |
| Focused | 1.0 | + `FOCUS_EXTRA_WIDTH` 1.0 (was `SELECTED_EDGE_EXTRA`) |
| Dimmed | `EDGE_DIM_ALPHA` 0.1 | relation width |

The arrow uses the same alpha. Cards are not dimmed; React Flow dims nothing by default, and only edges are cheap to change.

## Handles (step 3)

React Flow draws 6 px round handles where edges attach. Here:

- A second canvas, `handle_canvas`, is placed **after the cards** in the canvas div. It has no hitbox, so it takes no mouse events. It paints a dot at the start and the end of the route of every visible edge, only at `Text` detail.
- Each dot is a round `paint_quad`, `HANDLE_SIZE` 6 × zoom (never below 6 px), origin snapped. The fill is the kind color of the node it sits on, with a 1 px `background` border (React Flow: dark fill, light border). Duplicate dots on a shared point are drawn again; they are opaque and identical.
- A pure `handle_points(route) -> [GraphPoint; 2]` keeps the attach rule in one place for the export (step 5).

## Controls panel (step 3)

- Bottom left of the canvas (React Flow `Controls` default `BottomLeft`, vertical): three kit `Button`s, ghost, xsmall, icon-only, with tooltips. `IconName::Plus` "Zoom in", `IconName::Minus` "Zoom out", `IconName::Maximize` "Fit" (all three are in the gpui-component default icon set).
- Styling: `background`, 1 px `border`, radius 6, `shadow_sm()`. The first view and Fit keep the graph `CONTROLS_INSET` (60 px) from the left edge, so the panel covers no card. It stops the mouse-down from propagating, so a click does not start a pan.
- Zoom in and out: `viewport.zoom_at(view_w / 2, view_h / 2, ±ZOOM_BUTTON_STEPS)`, with `ZOOM_BUTTON_STEPS` = 2 (×1.21, close to React Flow's 1.2). The clamp and the below-floor rule (decision 38) come from `zoom_at`.
- Fit: the existing `fit`. The header `Fit` stays (W11 shows it).

## Animated flow (step 4)

React Flow animates an edge with `stroke-dasharray: 5` and `dashdraw 0.5s linear infinite` from `stroke-dashoffset: 10` (`xyflow` `packages/system/src/styles/init.css`). That is 20 px/s toward the target.

| Rule | Value |
|---|---|
| Which edges | `Focused` edges of the **selected** node only (not hover), at `Text` detail (zoom ≥ 0.55) |
| Look | the dash of the relation × zoom (routes (7, 4), mounts (4, 3)); a solid owns edge flows in long dashes (16, 4), so the three stay apart. The relation color, alpha 1 |
| Phase | `flow_phase(elapsed, period, zoom) = ((elapsed_secs × FLOW_SPEED) mod period) × zoom`, `FLOW_SPEED` 20 graph units/s, `period` = on + off of the dash; `elapsed` since `TopologyView::created` (`Instant`) |
| Next frame | the paint closure calls `view.sync_flow(needs_flow_frame(animated, cx.reduce_motion(), window.is_window_active()))`. `sync_flow(true)` starts one `cx.spawn` timer (`FLOW_FRAME` 33 ms, about 30 fps) that notifies the view; `sync_flow(false)` drops it |
| `needs_flow_frame` | `animated > 0 && !reduce_motion && is_active` (pure, tested) |
| Reduced motion | `App::reduce_motion()` (gpui-pre `app.rs` l. 1141): the dashes show with phase 0 and no frames are requested |

- **Idle cost is zero.** No selection, a selection with no visible edge, zoom below 0.55, a hidden Topology (not painted), or an inactive window all mean no frame request, so nothing repaints.
- **Running cost:** the timer notifies `TopologyView` about 30 times a second while a node is selected, not at the display rate (`request_animation_frame` would repaint the whole window every display frame). Only culled cards are built (0022). The view observes the window activation and repaints when it comes back, which restarts a timer that an inactive window stopped.
- `cx.reduce_motion()` is not wired to the Windows setting in this GPUI version (no platform caller). The tests and screenshots set it. Open item 1.

## Screenshot

`--screen topology-selected` (step 3): Topology with the first Deployment selected (drawer open). It is captured with reduced motion on, so the frame is deterministic. It shows the emphasis, the handles, the panel, and the static flow dashes (step 4).

## Overlays and the drawer (polish round)

- The minimap and the legend sit in the bottom strip that Fit keeps clear (`OVERLAY_GUTTER`). While the drawer is open they move left by its width (`DRAWER_WIDTH`), so both stay visible.
- A click on a card brings it and its direct neighbours into the part of the canvas the drawer leaves free (`Viewport::reveal_group`, UX walk M22): the least pan when they all fit at the current zoom; else centered at the largest grid zoom below the current one that fits them and still shows the card text (`MIN_TEXT_ZOOM`); else the nearest neighbours that fit with the node (the node alone, when it fills the area). `pending_focus` centers the node in that part and then does the same. When the canvas size becomes known and the first view is made again, a selected node is revealed the same way. The port label on `routes to` edges was skipped: `TopologyEdge` carries no port, and the graph build reads none.
- The legend draws a 38 px swatch per relation with the stroke code of the edges (same dash, color, and arrow) and an 11 px label.
