# 0022 — Topology, Resources mode (W11, read-only)

Status: amended after the advisor review (must-fix 1–4, should-fix 5–13, nice-to-haves 14–23), HEAD `62b9731`. Crate: `crates/app` only (no cluster-crate change); small amendments to 0012 (`service_health` core) and 0021 (`export_file_name` sanitizing). Wireframe: W11 (pins 1–5), sidebar top item Topology, "Show in Topology" in the W7 Service and Ingress menus. Applies C1, C6, C9, C11; settles the C6 row "Topology layout". Prerequisites are in [files-to-touch.md](files-to-touch.md).

**Amendment 2026-10-03 (steps 4a, 4b, not built): RBAC layer**, [rbac-layer.md](rbac-layer.md), audit gap 8, against main `a50264c`. **Read-only: no new mutating calls**; four more `list`/`watch` feeds while the RBAC chip is on.

## Goal

- A **namespace-scoped resource graph** built from live lists. Nodes: Ingress, Service, Deployment, StatefulSet, DaemonSet, ReplicaSet, Pod, ConfigMap, Secret (name only), PVC, HPA. Edges come from owner refs, selectors, ingress backends, volume, env and pull-secret refs, and HPA targets.
- **Config checks** drawn in place (W11 pin 2): ghost nodes for missing objects, red nodes and edges, and a checks chip with a dropdown.
- **Hand-written layered layout**: kind columns, a config row under each band, barycenter ordering. It is deterministic, and new pods appear in place (W11 pin 3).
- A **canvas** with wheel zoom, pan, Fit, node drag with remembered positions, a minimap, and a legend. Large pod sets collapse into one node. A click opens the drawer over the graph.
- **Filters**: kind chips (which also start and stop their watches), Problems only, and Group by (app by default). **Export PNG** (or SVG) goes through the save dialog (C9).
- **RBAC layer** (steps 4a, 4b): the RBAC chip draws workload → ServiceAccount → binding → role, with three access checks (W11 chip, W7 access kinds).

## Non-goals

- Traffic mode (built in 0049, not part of this spec), multi-namespace graphs, and any mutation. RBAC: User and Group subjects as nodes, ClusterRole rules (rbac-layer.md).
- Jobs, CronJobs, NetworkPolicies, PDBs, EndpointSlices, and custom resources (0018) as nodes.
- Persisting positions (0024), edge routing around nodes, Collapse pods. Zoom buttons and animation: see 0022b.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `Screen::Topology`, sidebar item, namespace choice, `TopologyFeeds`, `row_of`, graph, checks, layout with `previous`, canvas (wheel zoom, pan, Fit), select → drawer, double-click → reveal, too-large states, `--screen topology`, screenshot | 1–9 |
| 2 | Node drag and pins, Reset positions, minimap, legend, kind chips, Problems only, checks dropdown, Group by, pod-group expand, Show in Topology | 1–3, 5–10 |
| 3 | Export PNG/SVG (`resvg` edge, `topology_export.rs`), full ui-verifier run | 1–3, 11, 12 |
| 4a | RBAC graph, pure: five `TopologyKind`s, `Relation::Access`, `KindFilter::Rbac`/`DEFAULT`, `BindingIndex` join, three `CheckRule`s, `Placement::AccessRow`, `KindHue::Access` and the export arms | 1–3, 13–15 |
| 4b | RBAC feeds and chip (`TOPOLOGY_FEED_KINDS` +4), the `access` legend entry, click-to-reveal for feed-less rows, `--screen topology-rbac`, ui-verifier | 1–3, 16–18 |

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
| [rbac-layer.md](rbac-layer.md) | steps 4a, 4b: RBAC nodes, edges, chip, feeds, checks, layout, colors |
| [as-built-rbac.md](as-built-rbac.md) | steps 4a, 4b as built: deviations, UAT counts |
| [files-to-touch.md](files-to-touch.md) | prerequisites, modules per step, Cargo change, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [x] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [x] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [x] 3. Read-only: new requests are `list`/`watch` through the existing `ResourceKind::watch_rows` only. The 0001 grep still finds only the SSAR `create`. No change to `crates/cluster`.
- [x] 4. Steps 1–2 change neither `Cargo.toml` nor `Cargo.lock`. (Built in one pass, the only change is step 3's `resvg` edge: one line in each `Cargo.toml` and exactly `"resvg 0.46.0",` in `Cargo.lock`.)
- [x] 5. Watches: with Topology hidden, the status-bar count is unchanged. While it is visible, the count grows by `TopologyFeeds::open_count()` (≤ 10; only the kinds of the enabled chips). Covered by an `open_watch_count` test.
- [x] 6. Graph, checks, and layout are pure (no GPUI context), deterministic, and tested. `topology_budget` runs in the normal (debug) gate: 40 Services × 3,000 pods × 100 ReplicaSets, build + layout, asserting node and edge count ceilings (time printed only). The release timing is recorded in [decisions.md](decisions.md) (target ≤ 8 ms).
- [x] 7. The 0003 AC4 color-literal grep is clean. Every color comes from the theme or `tone_color`.
- [x] 8. Secret safety: topology modules have no `tracing::` call. Secret nodes carry names only; the graph, SVG, and PNG never hold a value.
- [ ] 9. On UAT (`readonly@Monitor`), for one namespace: ReplicaSet → Pod edges match `kubectl get rs,pods`, and each Service's pod edges match `kubectl get pods -n <ns> -l <selector> --context readonly@Monitor`. A click opens the drawer; a double-click reveals the row. Partly checked: `kubectl` is not installed here, so the `keda` namespace (3 Deployments, 3 ReplicaSets, 3 pods, 3 Services, 1 Secret) was checked against the probe listing, and the screenshot shows one ReplicaSet to one pod and each Service to its pod. The click, drawer, and double-click were not driven.
- [ ] 10. On UAT, a pod set above `POD_GROUP_LIMIT` shows one group node, and a click expands it. Adding a pod (rollout) leaves the siblings in place. No UAT namespace has a set that large, so group nodes are covered by the unit tests only.
- [x] 11. Export writes only after the dialog confirms. The `.png` output is a valid PNG of the whole graph, and a `.svg` path gets SVG text. Paths are never traced.
- [ ] 12. Screenshots `topology` and `topology-problems` exist (light and dark, `.tmp/ui-shots/v58-topology-*.png`). The ui-verifier reports no high-severity defect against W11. The ui-verifier has not run yet.
- [x] 13. (4a, W11 chip, W7 ServiceAccounts) With RBAC on, each account a namespace pod runs as is drawn once, linked from its top visible workload, then to every binding that names it directly, then to that binding's role; group-only grants appear only as the `+{n} via groups` caption.
- [x] 14. (4a, W11 pin 2, W7 ClusterRoleBindings) The three access checks (missing ServiceAccount Bad, binding to a missing Role Warn, account with cluster-admin Warn, also through a group, by `roles_held`) appear in the checks chip with the texts of rbac-layer.md; ClusterRole refs are never checked.
- [x] 15. (4a, W11 pin 3) The access row reads account → binding → role left to right under each band; with `previous`, adding a pod moves no RBAC card; `topology_budget` still passes with RBAC rows added; only ClusterRoleBindings naming the namespace's accounts count toward `RAW_LIMIT`.
- [x] 16. (4b, W11 toolbar) RBAC is a working chip, off by default; turning it on raises the watch count by exactly the started RBAC feeds (≤ 4; a denied list is Off and draws `not checked`), off lowers it back; `open_count()` ≤ 14.
- [x] 17. (4b) A click on an account, binding, or Role opens the drawer over the graph, and row keys act on it; a ClusterRole click does the same (its drawer-only ClusterRoles feed, decision 45). Read-only: no new request kind; colors from tokens (`cyan_light`), legend and export show `access`. Checked by unit tests (`card_click`, the feed rows through `row_of`); the click was not driven on UAT.
- [x] 18. (4b) Screenshot `topology-rbac` (light, dark) has no high-severity defect against W11.

## Open items

1. Optional refs (`optional: true`) are flagged as missing, because summaries do not carry the flag. Add `optional` to `EnvSource`/`VolumeSource` if they cause noise.
2. Feeds stop when Topology is hidden, so a click → reveal → back re-lists (about 1 s). Add a linger timer if this is slow.
3. Edges that span columns can cross nodes. Add dummy-node routing only if graphs prove unreadable.
4. Positions are memory-only until the 0024 settings store exists.
5. `row_of` covers the drawer, menu, and YAML sites only (decision 26). Over Topology, the Monitor tab and related lists (Deployment revisions) of a kind drawer show their empty text.
6. RBAC layer: bindings to User and Group subjects, ClusterRole contents, and SA token Secrets are not drawn (rbac-layer.md "Not in this layer"). Built: [as-built-rbac.md](as-built-rbac.md).
