# 0022 · Canvas: rendering and interaction

[Back to index](README.md) · Step 1 (edges, cards, wheel zoom, pan, Fit, click, double-click), step 2 (drag pins, minimap, legend, bands) · Modules: `topology_canvas.rs` (new; pure geometry + paint functions, tests in module), `topology_view.rs` (element tree, listeners). Decisions 24–26, 31.

## Element tree (in the workspace body, under the toolbar)

```
div#topology-canvas  relative, flex_1, overflow_hidden, rounded(theme.radius), border(theme.border), bg(theme.background)
│                    on_mouse_down (empty space: start Pan; store the window position only)
├─ canvas(prepaint: (), paint: dots → bands → edges → arrows; registers window mouse handlers)  absolute, size_full
├─ node cards (visible only)   absolute divs at to_screen(rect)
├─ legend   (step 2)           absolute bottom, right 170 px (W11 .tlg)
└─ minimap  (step 2)           absolute bottom-right 150 × 92 (W11 .mini), own canvas
```

**Window handlers in paint.** The paint closure gets `bounds`, so it registers every handler that needs them. Nothing stores bounds between frames. The closure captures the view's `WeakEntity` and an `Rc` of the graph and layout (decision 29).

| Handler (`window.on_mouse_event`) | Registered | Does |
|---|---|---|
| `ScrollWheelEvent` | always (phase Bubble, only when `bounds.contains(position)`) | `zoom_at(position − bounds.origin, ±1 step)` |
| `MouseMoveEvent` | only while `drag` is not `None` | pan by the delta, or move the dragged node (`to_graph`) |
| `MouseUpEvent` | only while `drag` is not `None` | end the drag. A node drag pins its origin; a press without movement is a click (below) |

The minimap canvas registers its own `MouseDownEvent` and, while dragging, `MouseMoveEvent`, using its own bounds.

## Viewport (pure)

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Viewport { pub(crate) origin: GraphPoint /* graph point at the top-left */, zoom_step: i32 }
impl Viewport {
    pub(crate) fn zoom(&self) -> f32;                                      // WHEEL_STEP.powi(zoom_step)
    pub(crate) fn to_screen(&self, p: GraphPoint) -> (f32, f32);          // relative to the canvas bounds
    pub(crate) fn to_graph(&self, x: f32, y: f32) -> GraphPoint;
    pub(crate) fn zoom_at(self, x: f32, y: f32, steps: i32) -> Self;       // keeps (x, y) fixed; clamps the step range
    pub(crate) fn pan(self, dx: f32, dy: f32) -> Self;
    pub(crate) fn fit(extent: GraphRect, width: f32, height: f32) -> Self; // largest grid zoom ≤ min(fit, 1.0), down to step −25, centered
    pub(crate) fn first_view(extent: GraphRect, width: f32, height: f32) -> Self; // Fit if readable, else MIN_TEXT_ZOOM at the extent top-left
    pub(crate) fn center_on(self, p: GraphPoint, width: f32, height: f32) -> Self;
}
pub(crate) fn visible_nodes(layout: &TopologyLayout, view: Viewport, width: f32, height: f32) -> Vec<usize>;
pub(crate) fn edge_curve(from: GraphRect, to: GraphRect, relation: Relation) -> EdgeCurve; // start, ctrl1, ctrl2, end
pub(crate) fn arrow_head(curve: &EdgeCurve, size: f32) -> [GraphPoint; 3];
pub(crate) fn minimap_transform(extent: GraphRect, width: f32, height: f32) -> MinimapTransform;
```

- Zoom is **quantized** (decision 25). `zoom = WHEEL_STEP^zoom_step`, with `WHEEL_STEP` = 1.1 and the step range `[−17, 7]` (≈ 0.2–1.95). One wheel notch is ±1 step. Pixel-precise deltas accumulate 50 px per step.
- **Fit and the first view** (decision 38). Fit may go below the wheel floor, down to step −25 (about 0.09), so it shows everything. The wheel never pushes a view that Fit put below its floor back up. The automatic first view of a graph is `max(fit, MIN_TEXT_ZOOM)`: the whole graph when it is readable at that zoom, else step −6 (about 0.56) anchored at the top-left of the extent; the minimap shows the rest. Both use the canvas minus the **overlay strip** (`OVERLAY_GUTTER`, the minimap height plus 16 px), so the minimap and legend do not cover what Fit shows. Before the first paint the canvas size is a default (1200 × 700), and the first view is made again once the real size is known.
- **Canvas size**: the canvas layer reports its size to the view at every paint (an assignment, no notify unless it changed), because Fit, the first view, and focus need it. This is the one thing stored from a frame.
- **Edge curve**:
  - Mounts: from the source's bottom center to the target's top center, with vertical controls at `dy / 2`. This is the config row (layout.md step 5).
  - Other forward edges (`to.x > from.x`): from the source's right middle to the target's left middle, with horizontal controls at `dx / 2` (W11 `C` curves).
  - Anything else: bottom center to bottom center, with both controls `40` below the lower point.

## Painting (canvas paint, back to front)

| Layer | How | Color |
|---|---|---|
| Dot grid | round `paint_quad` dots, gap 20, snapped to device px (0022b [stroke.md](../0022b-topology-visuals/stroke.md)) | `ring` |
| App bands | `paint_quad` rounded rect, 1 px `BorderStyle::Dashed`; the title is a card-layer text div | `border`; title `muted_foreground` |
| Edges | feathered ribbons from `topology_stroke.rs` (0022b [stroke.md](../0022b-topology-visuals/stroke.md)), along routes that never cross another card ([routing.md](../0022b-topology-visuals/routing.md)); not `PathBuilder` or one bezier | relation color (0022b [colors.md](../0022b-topology-visuals/colors.md)) |
| Arrows | `fill_convex`, feathered | the edge color |

| Relation | Width | Dash | Color |
|---|---|---|---|
| Owns | 1.4 | solid | `muted_foreground` |
| RoutesTo | 1.6 | `[5, 4]` | `ring` |
| Mounts | 1.2 | `[2, 3]` | `muted_foreground` |

The Color column is replaced by 0022b (relation colors at 0.75 alpha). An edge into a ghost uses `tone_color` of its check. The focused node's edges are full strength with `+0.6` width, the others fade (0022b [interaction.md](../0022b-topology-visuals/interaction.md)).

## Node cards (divs; W11 `.nd`)

- Size `NODE_WIDTH × NODE_HEIGHT × zoom`, radius `8 × zoom`, `background`, 1 px `border`. The badge column is tinted by node kind, and hover and selection use the kind color and a shadow: see 0022b [colors.md](../0022b-topology-visuals/colors.md).
- A grid holds a 24 px badge column (the `badge()` text, mono, `muted` fill) and two lines:
  - caption: mono 9 px uppercase, `muted_foreground`, toned when Bad/Warn;
  - name: mono 10.8 px, `text_ellipsis`.
  - Every size is multiplied by `zoom`.
- Bad → 2 px Bad border; Warn → 1 px Warn border; selected → 2 px `ring` (selection wins).
- Looks:
  - `Ghost`: dashed 1.5 px border in its check tone, transparent, no badge, tooltip = check text.
  - `Unchecked`: dashed 1 px `muted_foreground` border at 0.7 opacity, caption `{Kind} · not checked`.
  - Config kinds: 0.85 opacity (W11).
- **LOD** (decision 39): from zoom 0.55 the text lines show. From 0.3 to 0.55 the card shows its badge alone, large enough to read and in the node tone. Below 0.3 it is a plain box. Every card is opaque (the dimmer looks fade only their content), so an edge never shows through its text; edges are painted under the cards. A band title keeps a fixed 11 px size on the band edge below 0.55.
- `id` = `("topology-node", node index)`. Only `visible_nodes` are built.

## Interaction

| Input | Target | Effect |
|---|---|---|
| wheel | canvas | zoom ±1 step around the cursor |
| press + move ≥ `DRAG_SLOP` (4 px) | empty space | pan |
| press + move ≥ `DRAG_SLOP` (step 2) | object card | move the node; release pins its origin (decision 23) |
| click | object card (with `key`) | `AppShell::select_on_topology(Some(key))` → drawer over the graph |
| click | pod group | insert into `expanded` (graph-model.md) |
| click | ghost or unchecked | highlight only, with a tooltip |
| double click (`click_count == 2`) | object card | `AppShell::reveal(key)` (leaves Topology) |
| click (no drag) | empty space | `select_on_topology(None)` |
| press / drag | minimap | `center_on(minimap → graph point)` |
| hover | object card | its edges stand out, the others fade (0022b) |
| click | + / − / Fit panel | zoom around the center, or Fit (0022b) |
| (selected node) | its edges | animated flow, still under reduced motion (0022b step 4) |

`enum Drag { None, Pan { last: Point<Pixels> }, Node { index, grab: GraphPoint, moved: bool } }` lives in `TopologyView`. Card `on_mouse_down` stops propagation so the canvas does not also pan, and calls `cx.notify()` so the next paint registers the move and up handlers.

## Minimap and legend (step 2)

- **Minimap**: `muted` fill, 1 px `border`. Each node is a rect of at least 2 × 2 px, `muted_foreground` (Bad/Warn: `tone_color`). The viewport is a 1.5 px `ring` outline. It shows whenever a graph is visible.
- **Legend** (decision 31): three mono 10 px entries, `── owns`, `╌╌ routes to`, `┈┈ mounts`. The glyphs are colored like the edges (routes: `ring`) and the text is `muted_foreground`, on `background` with a 1 px `border` (W11 `.tlg`).
