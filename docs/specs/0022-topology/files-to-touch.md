# 0022 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own, and every new item has a production user in its step. `crates/cluster` is not touched.

## Prerequisites (merged first)

| Step | Needs | Why |
|---|---|---|
| 1 | 0012 (all steps), including the amended `service_health` slice core ([row-model.md](../0012-kind-drawer-completions/row-model.md)) | `KindObject` variants, `cluster::Selector`, `service_health`, `kind_diagnosis`, `OpenWatches`, `owns_pod` |
| 1 | 0013 step 3, 0014 step 2, 0016 (step 1 + Secrets kind step) | `ResourceKind::{HorizontalPodAutoscalers, PersistentVolumeClaims, Secrets}` with `watch_rows` arms and `KindObject` variants; `DiagnosisInputs.tls_secrets`; `Projected.secrets`, `image_pull_secrets` |
| 1 | 0020 step 1a if in flight | `OpenWatches` fields and `PVC_PENDING_GRACE`. If 0020 has not landed, define it in `topology_checks.rs`; 0020 moves it |
| 3 | 0021 step 4 | `file_export.rs`: `ExportState`, the sanitizing `export_file_name`, the save-flow pattern |

**Navigation tests.** 0020, 0021, and 0022 each enable a top item. Whichever lands later updates `enabled_items_are_pods_nodes_and_explorer_kinds`.

## `crates/app`

| S | File | Change |
|---|---|---|
| 1 | `src/topology_graph.rs` (new) + `topology_graph_tests.rs` | `TopologyKind`, `KindFilter`, `NodeId`, `NodeLook`, `TopologyNode`, `Relation`, `TopologyEdge`, `TopologyGraph`, `TopologyBuild`, `TooLarge`, `TopologyInputs`, `FeedRows`, `TopologyFilter`, `GroupBy`, `build_topology`, `resolve_group_by`, `RAW_LIMIT`, `POD_GROUP_LIMIT`, `NODE_LIMIT` |
| 1 | `src/topology_checks.rs` (new, tests in module) | `CheckRule` (+ `chip_label`), `ConfigCheck`, `graph_checks`, `topology_coverage` |
| 1 | `src/topology_layout.rs` (new) + `topology_layout_tests.rs` | `GraphPoint`, `GraphRect`, `Placement`, `Band`, `TopologyLayout`, `layout` (with `previous`), `GraphStructure`, `structure`, sizes |
| 1 | `src/topology_viewport.rs` (new, tests in module) | `Viewport` (quantized; Fit down to step −25, `first_view`), `wheel_steps`, `is_drag`, `visible_nodes`, `minimap_transform`, `OVERLAY_GUTTER` (moved out of the canvas) |
| 1 | `src/topology_canvas.rs` (new, tests in module) | `Viewport` (quantized), `visible_nodes`, `edge_curve`, `arrow_head`, edge paint, window handlers registered in paint, node card element |
| 1 | `src/topology_feeds.rs` (new, tests in module) | `TOPOLOGY_FEED_KINDS`, `TopologySubject`, `TopologyFeeds`, `TopologyFeed`, `feed_plan`, `open_count`, `rows` |
| 1 | `src/topology_view.rs` (new) | `TopologyView` entity: state (`Rc` graph and layout), header count, toolbar (segment, namespace dropdown, coverage), click, double-click, pan, wheel, tick, observe, Fit, empty and too-large states |
| 1 | `src/kind_join.rs` (0012) | if 0012 already merged the `LiveList` signature: split it into the slice core + wrapper (0012 row-model amendment) |
| 1 | `src/cluster_session.rs` (+ tests) | `LiveCluster.topology`, `set_topology_subject`, `topology()`, `row_of`, `OpenWatches.topology`, access re-plan, scope change |
| 1 | `src/app_shell.rs`, `src/kind_drawer.rs`, `src/workspace.rs` | `Screen::Topology` and its arms; `topology` entity; `select_on_topology`; the drawer, menu, and YAML lookups go through `row_of` |
| 1 | `src/navigation.rs` (+ tests) | `screen_of("Topology")`; Topology enabled |
| 1 | `src/launch_options.rs` (+ tests), `src/screenshot.rs` (+ tests), `src/main.rs` | `--screen topology`; settle rule; `mod topology_*;` |
| 2 | `src/topology_canvas.rs`, `src/topology_view.rs` | node drag and pins, `Reset positions` (only with pins), minimap (`minimap_transform`), glyph legend, App bands, ghost tooltips, LOD |
| 2 | `src/topology_graph.rs`, `src/topology_checks.rs` | kind filters, Problems only, group keys, `expanded` |
| 2 | `src/topology_view.rs` | chips (restart feeds via `set_topology_subject`), `Group by` dropdown, checks dropdown, `pending_focus`, no-problems state |
| 2 | `src/workspace.rs` | `toggle_button` becomes `pub(crate)` (Problems only and the chips reuse it) |
| 2 | `src/kind_drawer.rs` (kind menu), `src/app_shell.rs` | `Show in Topology` for Services and Ingresses; `show_in_topology` |
| 2 | `src/launch_options.rs` (+ tests) | `--screen topology-problems` |
| 3 | root `Cargo.toml`, `crates/app/Cargo.toml`, `Cargo.lock` | the `resvg` edge (export.md) |
| 3 | `src/topology_export.rs` (new, tests in module) | `SvgStyle`, `svg_style`, `hex`, `topology_svg`, `fit_chars`, `ExportError`, `Png`, `render_png`, `export_scale`, the save flow |
| 3 | `src/topology_view.rs`, `src/main.rs` | `Export PNG` button, `export` state, `export_scale` text, error `Alert`; `mod topology_export;` |

## Docs (updated by the coder in the step that completes the row)

- `docs/roadmap/inventory-screens.md`:
  - W11-1 and W11-2 → Done (step 1; note optional refs).
  - W11-3 → Partial (step 1), then Done (step 2; positions memory-only).
  - W11-4 → Done (step 2). W11-5 → Done (step 3). W11-6 stays backlog.
- `docs/roadmap/inventory-shell.md`: N5 Topology → Done (step 1). `docs/roadmap/inventory-kinds.md`: Services and Ingresses "Show in Topology" → Done (step 2).
- `docs/roadmap/cross-cutting.md`: the C6 row "Topology layout" → hand-written (0022); add a `resvg` row (step 3).
- `docs/roadmap/README.md`: the Overview/Issues/Topology status row. `gap-plan-read-only.md` 0022 links this folder.
- [decisions.md](decisions.md) "Measurements": the release `topology_budget` time (step 1).
