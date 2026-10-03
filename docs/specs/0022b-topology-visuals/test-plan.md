# 0022b · Test plan

[Back to index](README.md). Unit tests are offline, deterministic, and check one behavior each. The names are binding (AC 2). Geometry tests compare with a tolerance of 1e-3.

## Unit tests

| Step | Module | Tests |
|---|---|---|
| 1 | `topology_stroke_tests.rs` | `flattened_cubic_stays_within_a_fifth_of_a_pixel` (three curves: forward, mounts, backward, at zoom 2; exact samples against the polyline), `shorter_curves_flatten_to_fewer_points`, `zero_length_segments_are_skipped`, `dash_runs_follow_the_pattern`, `a_growing_phase_moves_dashes_toward_the_end`, `trim_end_shortens_by_the_arrow_length`, `ribbon_vertices_carry_the_signed_edge_distance` (every vertex: `s == 0`, `t == half − offset`; outer at `half + feather`), `ribbon_alpha_ramps_over_one_device_pixel` (below), `zero_feather_is_the_plain_stroke`, `consecutive_segments_share_offset_vertices`, `a_sharp_join_clamps_its_miter`, `convex_fill_feathers_every_side`, `degenerate_input_builds_no_path`, `feather_is_half_a_device_pixel_on_windows_only`, `edge_stroke_budget` (500 dashed edges at zoom 1, ≤ 30 ms debug; prints the time) |
| 1 | `topology_canvas.rs` | `dashes_turn_solid_below_text_zoom`, `edge_width_has_a_screen_floor`, `arrow_head_uses_its_half_width`, `snap_rounds_to_device_pixels`, `dots_skip_below_the_minimum_spacing` (updated) |
| 2 | `topology_palette.rs` | `every_kind_has_one_hue` (the full table of palette.md), `workload_kinds_share_the_workload_hue`, `pods_and_groups_use_the_pod_hue`, `a_ghost_edge_takes_its_check_tone`, `edge_color_follows_the_relation`, `kind_hues_avoid_the_tone_tokens` (the token per hue is never `red`/`yellow`; a table test, no theme needed) |
| 2 | `topology_canvas.rs` | `badge_column_is_tinted_at_text_detail`, `a_box_card_is_filled_below_badge_zoom`, `selection_wins_over_tone_borders` (kept), `minimap_mask_covers_the_outside_of_the_viewport` (four rects whose union with the viewport is the minimap) |
| 3 | `topology_canvas.rs` | `no_focus_leaves_every_edge_at_rest`, `focus_emphasizes_its_edges_and_dims_the_rest`, `handles_sit_on_both_curve_ends`, `handles_show_at_text_detail_only` |
| 3 | `topology_viewport.rs` | `button_zoom_keeps_the_view_center`, `button_zoom_is_clamped_like_the_wheel` |
| 3 | `topology_view.rs` (gpui test) | `hover_notifies_only_on_change`, `hover_is_ignored_during_a_drag`, `a_namespace_change_clears_the_hover` |
| 3 | `launch_options.rs` | `screen_topology_selected_parses` |
| 3 | `screenshot.rs` | `topology_selected_picks_the_first_deployment` |
| 4 | `topology_canvas.rs` | `only_the_selected_nodes_edges_animate`, `hover_alone_animates_nothing`, `nothing_animates_below_text_zoom`, `flow_phase_wraps_with_the_dash_period`, `no_frame_request_when_idle_reduced_or_inactive` (`needs_flow_frame` truth table) |
| 5 | `topology_export_tests.rs` | `svg_badge_columns_use_kind_colors`, `svg_edges_use_relation_colors_at_rest_opacity`, `svg_has_a_marker_for_every_edge_color`, `svg_handles_sit_on_edge_ends`, `svg_bands_are_filled`; the existing secret and escaping tests still pass |

### `ribbon_alpha_ramps_over_one_device_pixel`

A test-only `fn hlsl_path_alpha(s: f32, t: f32, ds: (f32, f32), dt: (f32, f32)) -> f32` copies `shaders.hlsl` l. 1004–1012 line for line. For scale factors 1, 1.5, and 2, build a straight horizontal ribbon (width 1.5, `feather(scale)` as on Windows). Then sample a vertical cross-section every 0.1 device px, interpolating `t` from the triangle vertices:

- alpha ≥ 0.99 at distance ≤ `half − 0.5 dpx`;
- alpha ≤ 0.01 at distance ≥ `half + 0.5 dpx`;
- alpha falls monotonically, and is 0.5 ± 0.05 at the nominal edge.

This is the measurable anti-aliasing check that runs in the gate. The pixel census below confirms it on the GPU.

## ui-verifier (per step, light and dark, `--context readonly@Monitor`)

Before = `.tmp/ui-shots/v61-topology-{light,dark}.png`, `v61-topology-monitoring-light.png`, `v61-topology-problems-dark.png`. After = the next free `vNN` with the same names and commands, plus `topology-selected` from step 3 on.

1. **Edge pixel census (step 1, measurable).** In the `monitoring` shot, pick three shallow edge crossings away from dots and cards. At each, read a 20 × 7 px window centred on the edge and count the distinct colors strictly between the canvas background and the edge's core color. Expected: v61 ≤ 3 (4× MSAA quantizes coverage to 1/4, 2/4, 3/4); after ≥ 8 at two of the three. Use a throwaway stdlib-only Python PNG reader under `.tmp/`. Record the numbers in decisions.md "Measurements".
2. Step 1: no stair steps or broken dashes at 100 % zoom; dashes are solid in a zoomed-out shot; dots are round and even.
3. Step 2: kind colors follow the palette.md table in both themes; badge text is readable (no washed-out light-theme text); Bad/Warn borders and ghosts are unchanged; the minimap is colored and masked; bands are faintly filled.
4. Step 3: in `topology-selected`, the selected card has the kind border and shadow, its edges are full strength, others are faint, handles sit on the edge ends, and the panel is at bottom left without covering cards at Fit.
5. Step 4: `topology-selected` (reduced motion) shows static flow dashes on the selected edges only. A manual check that the dashes move toward the target is noted, along with the frame time in decisions.md.
6. Step 5: an exported PNG of `monitoring` matches the screen's rest colors (side-by-side screenshot).
7. Every step: no high-severity defect against W11, and nothing hard-codes a color.
