# 0022 — Topology, Resources mode (W11, read-only)

Status: amended after the advisor review (must-fix 1–4, should-fix 5–13, nice-to-haves 14–23), HEAD `62b9731`. Crate: `crates/app` only (no cluster-crate change); small amendments to 0012 (`service_health` core) and 0021 (`export_file_name` sanitizing). Wireframe: W11 (pins 1–5), sidebar top item Topology, "Show in Topology" in the W7 Service and Ingress menus. Applies C1, C6, C9, C11; settles the C6 row "Topology layout". Prerequisites are in [files-to-touch.md](files-to-touch.md).

## Goal

- A **namespace-scoped resource graph** built from live lists. Nodes: Ingress, Service, Deployment, StatefulSet, DaemonSet, ReplicaSet, Pod, ConfigMap, Secret (name only), PVC, HPA. Edges come from owner refs, selectors, ingress backends, volume, env and pull-secret refs, and HPA targets.
- **Config checks** drawn in place (W11 pin 2): ghost nodes for missing objects, red nodes and edges, and a checks chip with a dropdown.
- **Hand-written layered layout**: kind columns, a config row under each band, barycenter ordering. It is deterministic, and new pods appear in place (W11 pin 3).
- A **canvas** with wheel zoom, pan, Fit, node drag with remembered positions, a minimap, and a legend. Large pod sets collapse into one node. A click opens the drawer over the graph.
- **Filters**: kind chips (which also start and stop their watches), Problems only, and Group by (app by default). **Export PNG** (or SVG) goes through the save dialog (C9).

## Non-goals

- Traffic mode (backlog; segment disabled), the RBAC chip (disabled), multi-namespace graphs, and any mutation.
- Jobs, CronJobs, NetworkPolicies, PDBs, EndpointSlices, and custom resources (0018) as nodes.
- Persisting positions (0024), edge routing around nodes, zoom buttons, Collapse pods, and animation.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `Screen::Topology`, sidebar item, namespace choice, `TopologyFeeds`, `row_of`, graph, checks, layout with `previous`, canvas (wheel zoom, pan, Fit), select → drawer, double-click → reveal, too-large states, `--screen topology`, screenshot | 1–9 |
| 2 | Node drag and pins, Reset positions, minimap, legend, kind chips, Problems only, checks dropdown, Group by, pod-group expand, Show in Topology | 1–3, 5–10 |
| 3 | Export PNG/SVG (`resvg` edge, `topology_export.rs`), full ui-verifier run | 1–3, 11, 12 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with one-line rationale; measurements |
| [graph-model.md](graph-model.md) | kinds, node ids, edge rules, ref aggregation, pod groups, input cap, filters |
| [config-checks.md](config-checks.md) | check rules, chip labels, reuse of 0012–0016 diagnosis, coverage, Problems only |
| [layout.md](layout.md) | columns, config row, bands, ordering, `previous` seeding, pins, sizes, budget |
| [canvas.md](canvas.md) | element tree, painting, viewport math, interactions, minimap, legend, LOD |
| [screen-and-session.md](screen-and-session.md) | screen wiring, toolbar, namespace, feeds and watch budget, async, drawer, Show in Topology |
| [export.md](export.md) | step 3: SVG builder, resvg rasterizing, save flow |
| [files-to-touch.md](files-to-touch.md) | prerequisites, modules per step, Cargo change, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [ ] 3. Read-only: new requests are `list`/`watch` through the existing `ResourceKind::watch_rows` only. The 0001 grep still finds only the SSAR `create`. No change to `crates/cluster`.
- [ ] 4. Steps 1–2 change neither `Cargo.toml` nor `Cargo.lock`.
- [ ] 5. Watches: with Topology hidden, the status-bar count is unchanged. While it is visible, the count grows by `TopologyFeeds::open_count()` (≤ 10; only the kinds of the enabled chips). Covered by an `open_watch_count` test.
- [ ] 6. Graph, checks, and layout are pure (no GPUI context), deterministic, and tested. `topology_budget` runs in the normal (debug) gate: 40 Services × 3,000 pods × 100 ReplicaSets, build + layout ≤ 80 ms. The release timing is recorded in [decisions.md](decisions.md) (target ≤ 8 ms).
- [ ] 7. The 0003 AC4 color-literal grep is clean. Every color comes from the theme or `tone_color`.
- [ ] 8. Secret safety: topology modules have no `tracing::` call. Secret nodes carry names only; the graph, SVG, and PNG never hold a value.
- [ ] 9. On UAT (`readonly@Monitor`), for one namespace: ReplicaSet → Pod edges match `kubectl get rs,pods`, and each Service's pod edges match `kubectl get pods -n <ns> -l <selector> --context readonly@Monitor`. A click opens the drawer; a double-click reveals the row.
- [ ] 10. On UAT, a pod set above `POD_GROUP_LIMIT` shows one group node, and a click expands it. Adding a pod (rollout) leaves the siblings in place.
- [ ] 11. Export writes only after the dialog confirms. The `.png` output is a valid PNG of the whole graph, and a `.svg` path gets SVG text. Paths are never traced.
- [ ] 12. Screenshots `topology` and `topology-problems` exist. The ui-verifier reports no high-severity defect against W11.

## Open items

1. Optional refs (`optional: true`) are flagged as missing, because summaries do not carry the flag. Add `optional` to `EnvSource`/`VolumeSource` if they cause noise.
2. Feeds stop when Topology is hidden, so a click → reveal → back re-lists (about 1 s). Add a linger timer if this is slow.
3. Edges that span columns can cross nodes. Add dummy-node routing only if graphs prove unreadable.
4. Positions are memory-only until the 0024 settings store exists.
5. `row_of` covers the drawer, menu, and YAML sites only (decision 26). Over Topology, the Monitor tab and related lists (Deployment revisions) of a kind drawer show their empty text.
