# 0050 — as built: edges are always curves, four anchors per node

[Back to README](README.md). User request (2026-10-05). This note **supersedes** the README wherever they differ: the README describes the first cut with the `Elbows | Curves` option.

## Decisions

1. **Elbows removed.** `EdgeShape`, the toolbar `Edges:` dropdown, the `topology.edges` setting, and the launch screens `topology-curves`, `topology-traffic-curves`, and `topology-traffic-fixture-curves` are gone. An old settings file with `topology.edges` still loads: settings ignore unknown fields (test `the_removed_topology_edges_key_is_ignored`). The lane router is gone too: `along_lanes`, `round_corners`, `simplified`, the corridors, and the `bands` parameter of `route_edges`.
2. **Four anchors.** A card has an anchor at the middle of each side (`Side::{Left, Right, Top, Bottom}`). A curve leaves and enters along the normal of its side, so left and right anchors have horizontal tangents and top and bottom anchors vertical ones. The arrival stub (`ARRIVAL_STUB`) is perpendicular to the target side, so the arrow, the handles, the labels, and the SVG export follow the new anchors without change: they read only the route.
3. **Anchor rule** (`facing_sides`, pure): `gap_x` and `gap_y` are the gaps between the two rects (negative when they overlap on that axis). If `gap_x >= gap_y`, the edge leaves by the right side and enters by the left side (reverse when the target is on the left). Otherwise it leaves by the bottom and enters by the top (reverse when the target is above). Two cards in one column therefore join bottom to top.
4. **Blocked edges.** If the default curve runs behind another card, the other 15 pairs of sides are tried, nearest first, each also with a tight bulge (`LOOP_REACH`). The first clear curve wins. If none is clear, the curve with the fewest cards in the way is kept. Residual: a long Service to Pod edge across packed columns still passes behind cards (`monitoring`: 40 of 86 edge and card pairs in Components, 18 in App at aspect 1.7). Painting is under the opaque cards, and hover dimming separates the edges. A waypoint search is the upgrade if this matters.

## Root cause of the stray elbows

`route_edge` in `topology_route.rs` fell back to `round_corners(&along_lanes(..))` whenever the single curve was blocked by a card (spec decision 1 of the first cut: "Blocked curves fall back to the lane route"). In `monitoring` that was every edge that skipped a column, every Mounts edge to the config row, and any edge in a packed column. They were right-angle lane routes with rounded corners, so they read as elbows. Now every route is `bezier(..)` of some pair of anchors (test `every_route_is_a_curve_between_two_anchors`).

## Accent bar

The 3 px bar on the left of a card was full height and square, so its ends stuck out of the card's rounded corners. It is now a rounded pill whose length is `NODE_HEIGHT - 2 * CARD_RADIUS`, so its ends lie on the straight part of the left edge. The color is the kind color of the theme, as before (test `the_accent_bar_ends_where_the_card_corners_end`).

## Tests

`topology_route_tests.rs`: the anchor positions, the anchor rule (eight cases, a tie, overlap), vertical and horizontal stubs, a same-column edge, a backward edge, the close-neighbour case, the arrow on the end tangent for all four sides, a blocked edge taking another pair, every route a curve, routes starting and ending on anchors, and the bound on edges behind cards. Settings and launch tests changed as above. `topology_budget` is unchanged in cost (about 700 ms in the debug test profile, as before).

## Port labels on `routes to` edges (UX walk M22)

A Service to Pod edge shows the Service ports: `80→8080` (the target port only when it differs from the port), `443`, joined with commas up to 3, then ` +N` (`80,443,8080 +2`). An Ingress to Service edge shows the backend port, read from the `name:port` text of the rule. `TopologyGraph.port_labels` holds the text by `(from, to)`; `TopologyEdge` is unchanged.

Chips (`topology_port_labels.rs`) are drawn only on the `routes to` edges of the focused node (hovered, else selected), never at rest, so the graph at rest is unchanged. A chip sits on the canvas surface with the foreground tone, after the legend and minimap so it is never under them. Each chip is tried at the arc-length share 0.5, then 0.4, 0.6, 0.3, 0.7, 0.2, 0.8, and takes the first where it overlaps no card (`chip_fits`); with none clear it stays on 0.5, over the cards. Group headers are not checked. Chips may overlap each other. They show from `MIN_TEXT_ZOOM` up and not in Traffic mode, which has its own labels. Tests: label text, zoom gate, graph build, focused-only, rest draws nothing, share order, fallback.

Launch flags for screenshots: `--zoom <factor>` (0.2 to 1.95) sets the Topology first-view zoom; `--select <name>` with `--screen topology-selected` selects the node of that name instead of the first Deployment.
