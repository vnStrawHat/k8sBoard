# 0021 · Decisions

[Back to index](README.md). Architect defaults (the user asked not to stop for questions), amended after the advisor review.

## Scope and data

| # | Decision | Rationale |
|---|---|---|
| 1 | Overview becomes the **default landing** in step 3, together with Needs attention (`LaunchScreen` default `overview`); `--screen pods` keeps Pods | W3 is the first top item and answers "what is broken" first; without Needs attention it is not a landing |
| 2 | Steps 1–2 need only merged specs; **step 3 requires 0020 step 1b**; there is no pre-0020 fallback panel | a stand-in rule set would duplicate 0020; capacity, nodes, and changes are useful alone |
| 3 | **No new always-on watch.** Panels read pods, nodes, and namespaces (always on), node metrics (0010), kubelet PVC usage (0011), and the issue board (0020) | the C13 budget is owned by 0020 |
| 4 | **Two watches per scope namespace, only while Overview is visible**, selected server-side: `involvedObject.kind=Deployment,reason=ScalingReplicaSet` and `involvedObject.kind=HorizontalPodAutoscaler,reason=SuccessfulRescale`; `EVENT_LIMIT` cap each; `2 × scope_multiplicity` watches | the server sends exactly the rows shown; the API keeps events ~1 h, so an on-demand watch loses nothing |
| 5 | Recent changes = those two event kinds + state rows (node Ready transition, node joined, namespace created) | each is a real change with a timestamp; state rows are free and scope-independent; other Normal reasons are churn |
| 6 | No managedFields, revision numbers, or ConfigMap edits | they need managedFields or ReplicaSet history; later, with 0031 (open item 1) |
| 7 | "Who" = event source component (text before ` on `); node rows show `kubelet` (True/False) or `node-controller` (Unknown); namespace rows show none | the only actor that events carry; human users need managedFields |
| 8 | `Last 15 min ▾` offers **15 min** and **1 h** and applies to Recent changes only (tooltip). A muted footnote reads `Events kept ~1 h by the API server`; the empty text never claims completeness | 1 h is bounded by the event TTL; capacity and issues are "now" |
| 9 | Certificate expiry has **no separate tile**; `Cert expiring`/`Cert expired` rows come from the 0020 kind rules (0016 contract) | W3 shows certificates as a Needs attention row, not a tile |

## Panels

| # | Decision | Rationale |
|---|---|---|
| 10 | Needs attention shows the **first 6** board issues in board order; `View all {n} issues →` opens Issues when n > 6; the header pill shows the total, toned by the worst severity | W3 shows 4–6 rows; the Issues screen holds the rest |
| 11 | Button: `ViewLogs` → **View logs**; target Pod → **See why** (the drawer shows WHY); any other target → **Open {Kind}** in kind casing (`Open Secret`, `Open Deployment`); no target → no button | read-only actions only (0020 decision 26); W3 wording |
| 12 | Capacity math = the 0010 node drawer math: used = Σ latest node metrics; requested = Σ `node_requests` (pods that take room, non-init containers); allocatable = Σ over **all listed nodes** | the numbers agree with the Nodes screen and drawer |
| 13 | Ceilings, stated in the row tooltip: init-container requests are undercounted, as in the node drawer; allocatable includes NotReady and cordoned nodes | honest numbers without new math |
| 14 | When the scope is not All, the **requested** layer and the **Pods** row are replaced by the notes `Requests need all namespaces` and `Pod counts need all namespaces`; used and allocatable stay (they are cluster-wide) | the pods list is scoped (0010 node drawer precedent) |
| 15 | Pods row = pods that take room / Σ `pods` allocatable | same rule as node pod counts |
| 16 | Volumes row = Σ used / Σ capacity of the newest `PvcUsage` per claim (0011), `· {n} PVCs`; note `{k} of {n} nodes polled` when Limited; `—` when the feed is not Live | 0011 decision 24 ("0021 sums volumes"); honest about the 10-node cap |
| 17 | Labels print the unit once (`104 used · 131 req · 168 cores`) through a shared-unit `Measure::format_shared`, which generalizes `format_pair` | W3 wording; one unit rule |
| 18 | Bar layers stay neutral like W3 (used `foreground`, requested `foreground` at 28 %, track `muted`); the used and requested figures are toned by `usage_tone(x / allocatable)` | wireframe look; the tone still flags overcommit |
| 19 | No per-node bars on Overview | the heatmap tooltip gives per-node numbers; the Nodes screen has bars |
| 20 | Heatmap: one fixed 24 px cell per node, in list order. Fill = `foreground` at alpha = CPU ratio, over `muted`. NotReady/Unknown → no fill and a 2 px Bad border. No sample → plain `muted`. Colored by CPU only | W3 "colored by CPU"; theme tokens only |

## Layout, refresh, launch

| # | Decision | Rationale |
|---|---|---|
| 21 | Two `flex_wrap` rows (Needs attention · Capacity; Nodes · Recent changes) with basis 560 / 360 px; both grow. Below ~930 px of content width they wrap to one column; the page scrolls | responsive without width math; ≈ 1.5 : 1 at 1320 px, like W3 |
| 22 | Header count text `{context} · Kubernetes {version}[ · {region}]`, plus a muted stats line under the header (user-requested, not in W3) | the W3 header plus the requested counts, without a new tile |
| 23 | Aggregates are computed **per render** from the snapshots; no cache, no tick | metrics repaint every 15 s, which also moves the change window |
| 24 | `--window-width <px>` launch flag (800–3840, default 1320), added in step 3 with all four panels | the ui-verifier screenshots the narrow layout once it is complete |
| 25 | Export report = Markdown through `cx.prompt_for_new_path` (step 4) with every issue (not just 6). It holds no kubeconfig data, and the path is never traced. One `ExportState` is shared with 0019 | C9 was approved in 0019; one export state type |
| 26 | A known denial of `list events` does not start the change feed; the panel says `Not permitted: list events` | no retry loop on a 403 (same gate idea as 0020 decision 7) |
| 27 | `View all →` sets the Events filter back to All (Warnings-only off), then opens Events | changes are Normal events, so a Warnings-only view would hide them |
