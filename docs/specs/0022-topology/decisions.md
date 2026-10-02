# 0022 · Decisions

[Back to index](README.md). These are architect defaults (the user asked not to stop for questions), amended after the advisor review.

## Data

| # | Decision | Rationale |
|---|---|---|
| 1 | **One namespace at a time**, chosen inside the session scope (any namespace when the scope is All) | W11 shows `ns: payments`. It bounds the nodes and watches |
| 2 | Pods come from the session pods list, filtered to the namespace. There is no extra pods watch | the scope rule guarantees they are watched |
| 3 | The other kinds are **lazy watches** through `ResourceKind::watch_rows(conn, Named(ns), EventFilter::default())`. They run only while Topology is visible, and only for the **enabled kind chips** | no new cluster API. Rows carry `KindObject`, status, and drawer content |
| 4 | Kinds whose list check is `Known` and denied are **Off**. Their ref targets draw as `{Kind} · not checked`, and no missing-object check runs | honest checks, no 403 storms |
| 5 | **Secrets** use 0016's `watch_secrets`, only while the Config chip is on. Only `name` and `secret_type` are read | C1: `SecretSummary` holds no values (names, sizes, type). The list is paged by 0016's `secret_watch_config` (`MostRecent`, 50 per page). Values only pass through the decoder |
| 6 | Node kinds are the 11 in the README | the W11 legend covers owns, routes, and mounts |
| 7 | Inactive ReplicaSets (`desired == 0 && current == 0`) are hidden | rollout history, not topology (0009 "Hide inactive") |
| 8 | ConfigMaps, Secrets, and PVCs appear **only when referenced, or when they have a check** | unused tokens and Helm secrets would bury the graph |
| 9 | `kube-root-ca.crt` refs are ignored | every pod mounts it through the projected service-account volume |
| 10 | Secret refs include `Projected { secrets }` and the pod's `image_pull_secrets` (0016) | 0016 counts the same ways for "Used by" |

## Graph semantics

| # | Decision | Rationale |
|---|---|---|
| 11 | Three edge relations follow the W11 legend: **owns** (solid), **routes to** (dashed accent), **mounts** (dotted). HPA → target uses owns | three legend entries |
| 12 | Config refs are **aggregated to the top visible workload**. Pods without a visible owner keep their own edges | W11 draws Deployment → ConfigMap; edges stay O(workloads) |
| 13 | **Pod groups**: an owner with more than `POD_GROUP_LIMIT` (6) non-Bad pods collapses them into one node. Bad pods stay single. A click expands the group, and it stays expanded until the namespace changes | large sets stay readable and failures stay visible. There is no Collapse button (YAGNI) |
| 14 | **Two caps**: `RAW_LIMIT` (5,000 listed rows + namespace pods) is checked **before** building; `NODE_LIMIT` (500 visible nodes) after grouping and filters. Either one shows a too-large state | the build cost is bounded by its input, not only its output |
| 15 | **Checks** are Topology's own graph rules plus the `kind_diagnosis` WHY boxes (0012–0016) and the 0012 `service_health` core. The 0020 `IssueBoard` is not read | the board is cluster-wide, grouped, and grace-held. Topology shows current state per object from the same rules |
| 16 | Pod failures tone the node but add no check | the Issues screen owns pod problems; W11's chip lists config checks |
| 17 | Problems only keeps the problem nodes plus their direct neighbours | a problem without context is unreadable |
| 18 | Each check rule has a **short chip label** (`1 Service matches no pods`). The full sentence goes in the dropdown and tooltip | W11 chip wording |
| 19 | A Service that an Ingress routes to uses the first such ingress path as its caption (`Service · /v2`); otherwise its type | W11 captions |

## Layout and canvas

| # | Decision | Rationale |
|---|---|---|
| 20 | **Hand-written layout, no crate**: columns by kind, bands, and barycenter sweeps | `layout-rs` and `rust-sugiyama` are not in `Cargo.lock`. Kind columns make layer assignment and cycle removal unnecessary |
| 21 | ConfigMaps, Secrets, and PVCs sit in a **config row under each band**, x-aligned with the workload columns. Mounts edges run from the bottom center to the top center | W11 places config under the Deployment |
| 22 | `layout(.., previous)` **seeds the order from the previous layout** and appends new nodes, without sweeps. Only a namespace change, a Group by change, or Reset positions lays out from scratch | W11 pin 3: "pods appear in place" |
| 23 | Dragged positions are **pins**, kept in memory per (context, namespace) | W11 pin 4; persistence waits for 0024 |
| 24 | **Hybrid rendering**: one gpui `canvas` paints the dots, bands, edges, and arrows, and registers the window-level mouse handlers. Cards are kit-styled divs culled to the viewport. The minimap is its own `canvas` | GPUI does card hit testing and text. The kit `Plot` has no mouse input |
| 25 | Wheel zoom is **quantized** to `WHEEL_STEP^k` (0.2–2.0) around the cursor. Empty drag pans, card drag pins, click selects, double-click reveals. There are no zoom buttons | predictable steps; Fit and the wheel are enough (YAGNI) |
| 26 | A click opens **the same drawer over Topology** via `LiveCluster::row_of` (explorer, then topology feeds). `row_of` replaces only the drawer, menu, and YAML lookups | W11 pin 4. Monitor and related lists stay explorer-only (README open item 5) |
| 27 | Group by: **App by default** (`app.kubernetes.io/name`, `app`, `k8s-app`). It falls back to Components when no pod in the namespace has one of those labels. A user's choice sticks for the session | W11 shows "Group by: app"; the fallback avoids one big "Ungrouped" band |
| 28 | Kind chips are Ingress, Service, Workload, and Config. RBAC is a disabled chip and Traffic a disabled segment | W11 toolbar |
| 29 | The rebuild happens at most every `TOPOLOGY_TICK` (500 ms), and only when dirty; one notify. The view holds `Rc<TopologyGraph>` and `Rc<TopologyLayout>` | batching rule. Paint closures and export clone an `Rc`, not the graph |
| 30 | Colors: `background`, `border`, `foreground`, `muted_foreground`, `muted`, `ring` (accent), `tone_color` | theme tokens only |
| 31 | The legend is mono glyph text (`──`, `╌╌`, `┈┈`) colored like the edges | three tiny canvases are not needed |

## Export and actions

| # | Decision | Rationale |
|---|---|---|
| 32 | **PNG via `resvg`**: Topology SVG text → resvg on the background executor → `Pixmap::encode_png`. `resvg 0.46` (text, system-fonts) is a non-optional dependency of `gpui-pre`, so there is **no new package** | `Window::render_to_image` is `test-support` only and captures the viewport only |
| 33 | A save path ending in `.svg` gets the SVG text | free once SVG exists |
| 34 | Export is the whole graph at 2× scale, capped at `MAX_EXPORT_SIDE` 4,096 px. Below 1× the status reads `exported at {pct}%` | incident docs (W11 pin 5); bounded memory (64 MB pixmap) |
| 35 | File names come from 0021's `export_file_name`, which replaces path-unsafe characters (`@`, `:`, `/`, …) with `-` | one sanitizer for every exporter |
| 36 | **Show in Topology** goes in the Service and Ingress menus. It is disabled with a reason outside the scope | gap plan 0022 |

## Measurements (filled by the coder)

| Item | Value |
|---|---|
| `topology_budget` release timing (target ≤ 8 ms) | TBD |
