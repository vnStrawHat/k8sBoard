# 0022b · Routing, columns, first view (polish round)

[Back to index](README.md) · Modules: `topology_route.rs` (new, pure, tests in `topology_route_tests.rs`), `topology_layout.rs`, `topology_viewport.rs`. Review finding: an edge that ran behind a card looked like a link to it.

## Routes

`TopologyLayout.routes[i]` is the route of `graph.edges[i]`: a polyline in graph units with its corners rounded. It is built once per layout (so also on a drag), not per frame. Both the canvas and the export draw it.

| Edge | Route |
|---|---|
| Forward, the target at least `DIRECT_GAP` (20) right of the source | one cubic from the right side of the source to the left side of the target, flat at both ends (flattened within 0.1 units). Used only if it keeps `CLEARANCE` (4) from every other card |
| Everything else (a skipped column, another band, a backward edge, every `Mounts`) | along the lanes, below |

**Lanes.** A lane is a vertical line in the gutter, `LANE_OFFSET` (25) beside a card side, so in the middle of the 50 unit gutter between two columns. Vertical lanes are free for the whole height of a band-column: cards sit only in the column slots.

1. The edge leaves the side of the source that faces the target (right if the target is right of it), to the lane.
2. It enters the side of the target that faces that lane: the right side if the lane is right of the target's center, else the left, to that side's lane.
3. Between the two lanes it crosses in a horizontal **corridor**: the first y, in order of detour, where the whole path keeps clear of every card but its two ends. Candidates are the gaps above and below every card and band, the two port rows, and then the free strip above or below everything (`MARGIN_CORRIDOR`), which is always free. The same lane up and down needs no corridor.
4. Corners get a quadratic arc of at most `CORNER_RADIUS` (12), less where a segment is short.

A mounts edge therefore leaves by the side and runs down the gutter, instead of dropping through the cards below its source. The routes cross each other and share lanes (a bus), but never a card.

## Columns and card width

- `Metrics` (layout): the kind columns (`Placement::Column`) that **some node of the graph uses**, in order. A column nothing uses is dropped before the bands are laid out: `argocd` with Problems only has one column per band, and `keda` has no empty strip on the left. Config slots wrap after the used columns.
- The card width is `CARD_CHROME` (63, the bar, the chip, and three paddings) plus 7.2 per character of the longest name, within 200 to 280. Every card has it, the pitch is that width plus the gutter, and a band is as wide as its columns. A longer name is cut with an ellipsis; the tooltip has it whole.

## First view and Fit

- `Viewport::first_view` fits the whole graph when the fit zoom is at least `MIN_BADGE_ZOOM` (0.3), whatever the node count (UX walk H5; below `MIN_TEXT_ZOOM` the cards show badges only). Otherwise it opens at 0.8, anchored at the top-left, and the header says `Showing part of the graph · {zoom}%` beside Fit until Fit is pressed.
- Both the first view and Fit use the canvas without its left `CONTROLS_INSET` (60) and move the graph right by that, so the zoom panel covers no card.
- The legend, the minimap, and the zoom panel stay in the bottom strip (`OVERLAY_GUTTER`, kept clear by Fit, the first view, and the reveal); with the drawer open they move left of it ([interaction.md](interaction.md)). The legend has a `Legend` toggle (UX walk M24): without a choice of the user it is open only where `legend_width` fits `legend_room` (the canvas without the zoom panel, the minimap, and the drawer), so it collapses at narrow widths and under the drawer; the choice is kept for the session (`legend_choice`), and a legend opened by hand where it does not fit wraps instead of covering the panel.
