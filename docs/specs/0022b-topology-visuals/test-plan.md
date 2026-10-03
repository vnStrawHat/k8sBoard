# 0022b · Test plan

[Back to index](README.md). Unit tests are offline, deterministic, and check one behavior each. The names are binding (AC 2). Geometry tests compare with a tolerance of 1e-3.

## Unit tests

| Step | Module | Tests |
|---|---|---|
| 1 | `topology_stroke_tests.rs` | `flattened_cubic_stays_within_a_fifth_of_a_pixel` (three curves: forward, mounts, backward, at zoom 2; exact samples against the polyline; Wang's bound), `shorter_curves_flatten_to_fewer_points`, `zero_length_segments_are_skipped`, `dash_runs_follow_the_pattern`, `a_growing_phase_moves_dashes_toward_the_end`, `trim_end_shortens_by_the_arrow_length`, `ribbon_vertices_carry_the_signed_edge_distance` (every vertex: `s == 0`, `t == half − offset`; outer at `half + feather`), `ribbon_alpha_ramps_over_one_device_pixel` (below), `zero_feather_is_the_plain_stroke`, `consecutive_segments_share_offset_vertices`, `a_sharp_join_clamps_its_miter`, `convex_fill_feathers_every_side`, `degenerate_input_builds_no_path`, `feather_is_half_a_device_pixel_on_windows_only` (both `PathCoverage` values on every OS), `the_renderer_version_the_coverage_relies_on_is_pinned` (Cargo.lock names gpui-pre-windows 0.3.7), `a_negative_gap_is_a_solid_dash`, `a_dashed_stroke_equals_the_ribbons_of_its_runs`, `edge_stroke_budget` (500 dashed edges at zoom 1: a deterministic vertex ceiling; the time is only printed) |
| 1 | `topology_canvas_tests.rs` | `dashes_turn_solid_below_text_zoom`, `edge_width_has_a_screen_floor`, `arrow_head_uses_its_half_width`, `arrow_head_points_at_target`, `the_arrow_follows_the_last_segment_of_a_bent_route`, `dots_skip_below_the_minimum_spacing`; `topology_viewport.rs`: `snap_rounds_to_device_pixels` |
| 2 | `topology_colors.rs` | `every_kind_has_one_hue` (the full table of colors.md), `workload_kinds_share_the_workload_hue`, `pods_and_groups_use_the_pod_hue`, `a_ghost_edge_takes_its_check_tone`, `edge_color_follows_the_relation`, `kind_hues_avoid_the_tone_tokens` (the token per hue is never `red`/`yellow`; a table test, no theme needed) |
| 2 | `topology_card_tests.rs` | `an_object_card_carries_a_trace_of_its_kind_and_a_ghost_does_not`, `the_selection_glow_is_the_kind_color_and_follows_the_theme`, `the_card_text_is_large_enough_to_read`, `the_card_chrome_matches_what_the_layout_reserves`, `a_box_card_is_filled_below_badge_zoom`, `selection_wins_over_tone_borders`, `a_hovered_card_takes_the_kind_border`, `the_level_of_detail_steps_down_with_the_zoom` |
| 2 | `topology_canvas_tests.rs`, `topology_colors.rs` | `handles_never_shrink_below_their_minimum`, `an_edge_arrow_is_solid_at_rest_and_faint_when_dimmed`, `minimap_mask_covers_the_outside_of_the_viewport` (four rects whose union with the viewport is the minimap); colors: `chip_text_contrasts_with_every_kind_color` (4.5 : 1), `a_light_card_is_plain_and_a_dark_card_is_raised`, `the_dark_glow_is_stronger_and_wider` |
| 3 | `topology_canvas_tests.rs` | `no_focus_leaves_every_edge_at_rest`, `focus_emphasizes_its_edges_and_dims_the_rest`, `handles_sit_on_both_route_ends`, `handles_show_at_text_detail_only` |
| 3 | `topology_viewport.rs` | `button_zoom_keeps_the_view_center`, `button_zoom_is_clamped_like_the_wheel`, `reveal_pans_a_hidden_card_into_the_free_area`, `a_small_graph_opens_whole_while_its_text_shows` |
| 3 | `topology_view.rs` (gpui test) | `hover_notifies_only_on_change`, `hover_during_a_drag_is_recorded_and_painted_when_it_ends`, `a_namespace_change_clears_the_hover`, `a_flow_timer_runs_only_while_edges_flow` |
| 3 | `launch_options.rs` | `screen_topology_selected_parses` |
| 3 | `topology_view.rs` | `topology_selected_picks_the_first_deployment` (next to the private `first_deployment`) |
| 4 | `topology_canvas.rs` | `only_the_selected_nodes_edges_animate`, `hover_alone_animates_nothing`, `nothing_animates_below_text_zoom`, `a_flowing_edge_keeps_its_own_dash_and_a_solid_one_gets_long_dashes`, `flow_phase_wraps_with_the_dash_period`, `no_frame_request_when_idle_reduced_or_inactive` (`needs_flow_frame` truth table) |
| polish | `topology_route_tests.rs` | `no_edge_runs_through_a_card_it_does_not_join` (keda and monitoring fixtures, app and component bands), `a_route_leaves_and_enters_by_the_side_of_its_cards`, `neighbouring_columns_are_joined_by_one_smooth_curve`, `a_mounts_edge_leaves_by_the_side_and_runs_in_a_gutter`, `an_edge_that_skips_a_column_bends_around_the_cards_between`, `rounded_corners_keep_the_ends_and_cut_the_corner`, `crosses_tells_a_segment_through_a_rect_from_one_beside_it`, and more |
| polish | `topology_layout_tests.rs` | `a_column_no_node_uses_is_dropped`, `the_columns_that_are_used_keep_their_order`, `the_card_width_follows_the_longest_name_up_to_a_cap` |
| 5 | `topology_export_tests.rs` | `svg_badge_columns_use_kind_colors` (chip, bar, surface), `svg_edges_use_relation_colors_at_rest_opacity`, `svg_has_a_marker_for_every_edge_color`, `svg_handles_sit_on_edge_ends`, `svg_bands_are_filled`; the existing secret and escaping tests still pass |

### `ribbon_alpha_ramps_over_one_device_pixel`

A test-only `fn hlsl_path_alpha(s: f32, t: f32, ds: (f32, f32), dt: (f32, f32)) -> f32` copies `shaders.hlsl` l. 1004–1012 line for line. For scale factors 1, 1.5, and 2, build a straight horizontal ribbon (width 1.5, `feather(scale)` as on Windows). Then sample a vertical cross-section every 0.1 device px, interpolating `t` from the triangle vertices:

- alpha ≥ 0.99 at distance ≤ `half − 0.5 dpx`;
- alpha ≤ 0.01 at distance ≥ `half + 0.5 dpx`;
- alpha falls monotonically, and is 0.5 ± 0.05 at the nominal edge (the test asserts `0.05 + 1e-3` for the 0.1 dpx sampling step).

This is the measurable anti-aliasing check that runs in the gate. The pixel census below confirms it on the GPU.

## ui-verifier (per step, light and dark, `--context readonly@Monitor`)

Before = `.tmp/ui-shots/v61-topology-{light,dark}.png`, `v61-topology-monitoring-light.png`, `v61-topology-problems-dark.png`. After = the next free `vNN` with the same names and commands, plus `topology-selected` from step 3 on.

1. **Edge pixel census (step 1, measurable).** In the `monitoring` shot, pick three shallow edge crossings away from dots and cards. At each, read a 20 × 7 px window centred on the edge and count the distinct colors strictly between the canvas background and the edge's core color. Expected: v61 ≤ 3 (4× MSAA quantizes coverage to 1/4, 2/4, 3/4); after ≥ 8 at two of the three. Use a throwaway stdlib-only Python PNG reader under `.tmp/`. Record the numbers in decisions.md "Measurements".
2. Step 1: no stair steps or broken dashes at 100 % zoom; dashes are solid in a zoomed-out shot; dots are round and even.
3. Step 2: kind colors follow the colors.md table in both themes; badge text is readable (no washed-out light-theme text); Bad/Warn borders and ghosts are unchanged; the minimap is colored and masked; bands are faintly filled.
4. Step 3: in `topology-selected`, the selected card has the kind border and shadow, its edges are full strength, others are faint, handles sit on the edge ends, and the panel is at bottom left without covering cards at Fit.
5. Step 4: `topology-selected` (reduced motion) shows static flow dashes on the selected edges only. A manual check that the dashes move toward the target is noted, along with the frame time in decisions.md.
6. Step 5: an exported PNG of `monitoring` matches the screen's rest colors (side-by-side screenshot).
7. Every step: no high-severity defect against W11, and nothing hard-codes a color.
