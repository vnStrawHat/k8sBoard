# 0022b — Topology visuals (anti-aliased edges, color, React Flow look)

Status: draft. Follows 0022 (merged, `0350adb`). Crate: `crates/app` only, with no dependency change. User request (2026-10-03): Topology is monotone; the lines are jagged; nodes and edges should look as good as React Flow. Wireframe: W11 (layout unchanged). Rules: theme tokens only, read-only, names only, English.

## Goal

- **Smooth edges and arrows.** The root cause is that GPUI paths get only 4× MSAA on Windows. The fix is feathered ribbons with analytic one-device-pixel coverage, through the existing path shader.
- **Color.** A kind → theme-token table drives the card badge columns, the minimap, and the low-zoom boxes. Edges are colored by relation. Status tones keep their tokens.
- **The React Flow look.** Snapped cards with hover and selected shadows, handle dots, hover highlight with dimming, a dot grid, a colored and masked minimap, a +/−/Fit panel, and an animated flow on the selected node's edges.
- **Export parity**: the same token table, in the rest state.

## Non-goals

- Layout, graph, checks, feeds, and edge routing (0022 open item 3). The `smoothstep` edge type; bezier stays, as React Flow's default.
- Dimming cards on hover, a resting card shadow, the React Flow "lock" control, and a theme editor.
- Analytic AA on macOS/Linux, which keep 4× MSAA (open item 3). Wiring reduced motion to the OS (open item 1).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `topology_stroke.rs`, feathered edges and arrows, width floor, LOD dashes, snapped round dot grid | 1–5 |
| 2 | `topology_colors.rs`, kind hues, badge tint, kind selection, LOD boxes, minimap colors and mask, band fill | 1–3, 6 |
| 3 | Hover emphasis and dimming, handle dots, the controls panel, `--screen topology-selected` | 1–3, 7 |
| 4 | Animated flow on the selected node's edges, frame guard, reduced motion | 1–3, 8 |
| 5 | Export parity | 1–3, 9 |

## Files

| File | Contents |
|---|---|
| [root-cause.md](root-cause.md) | how the edges paint, why they are jagged (with source lines), the fix, rejected options |
| [stroke.md](stroke.md) | step 1: the stroke API, geometry rules, edge painting, dot grid, cost |
| [routing.md](routing.md) | polish round: routes that avoid cards, dropped columns, card widths, first view, legend |
| [colors.md](colors.md) | step 2: tokens, kind and relation tables, cards, LOD, minimap, bands |
| [interaction.md](interaction.md) | steps 3–4: emphasis, handles, controls, animated flow and its repaint rule |
| [export.md](export.md) | step 5: the SVG/PNG mapping |
| [decisions.md](decisions.md) | React Flow reference with sources, numbered decisions, measurements |
| [files-to-touch.md](files-to-touch.md) | modules per step, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, the alpha-ramp test, the ui-verifier census and checklist |

## Acceptance criteria

- [x] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`, and no `Cargo.toml`/`Cargo.lock` change.
- [x] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [x] 3. The 0003 AC4 color-literal grep is clean, and every color comes from `CanvasColors`/`tone_color`. The topology modules make no `tracing::` call and hold names only. No new Kubernetes request.
- [x] 4. `ribbon_alpha_ramps_over_one_device_pixel` passes: analytic coverage spans one device px at scale 1, 1.5, and 2.
- [x] 5. ui-verifier edge pixel census on `monitoring`: v61 ≤ 3 intermediate colors; after ≥ 8, light and dark. Numbers are recorded in decisions.md. `edge_stroke_budget` asserts a vertex ceiling (not a wall-clock time); the release time is recorded.
- [x] 6. Before/after screenshots (`topology`, `topology-problems`, light and dark) show the kind colors of colors.md, readable badge text, and unchanged tone borders and ghosts.
- [x] 7. `topology-selected` (light, dark) shows the kind border and shadow, focused versus dimmed edges, handles, and the panel. +/− zoom around the centre, and Fit fits.
- [x] 8. Idle Topology runs no frame timer (`needs_flow_frame` and `a_flow_timer_runs_only_while_edges_flow`). With a node selected the flow repaints at 30 fps (a 33 ms timer; the display-rate frame request was dropped), and the view restarts the timer when the window is activated again.
- [ ] 9. The PNG of `monitoring` matches the screen's rest colors. Export tests pass, and secret safety is unchanged.
- [ ] 10. The ui-verifier reports no high-severity defect against W11 in either theme.

## Open items (user decisions; defaults apply until answered)

1. **Animated flow.** Default: on, for the selected node's edges only, static under `reduce_motion`. The alternatives are off, or a toolbar toggle. GPUI does not read the Windows reduced-motion setting; add a Settings switch if wanted.
2. **Edge colors.** Default: by relation (owns blue, routes cyan, mounts green) at 75 %. The alternative is neutral grey edges like React Flow, with color on nodes only.
3. **macOS/Linux AA.** Default: MSAA only, as today. An `s`-varying variant for Metal/WGSL needs a check on those platforms first.
- Engineering note: dash ends have no feather (butt caps under MSAA). Add cap feathering only if the census or the verifier flags dash ends.
