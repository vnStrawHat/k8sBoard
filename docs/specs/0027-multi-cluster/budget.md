# 0027 · Resource and watch budget

[Back to index](README.md) · Steps 1 and 4 · Decisions 2, 5, 22. Risks R4, C4, C13.

## What one extra viewed cluster costs

Each slot is a full `ClusterSession` (no change to its internals):

| Item | Per extra slot | Source |
|---|---|---|
| Connection | one kube `Client` (HTTP/2, rustls) | `connect_cluster` |
| Access reviews at connect | the existing SSAR set for the scope | 0001/0005, `AccessState` |
| Always-on watches | namespaces + pods + nodes (pods × scope multiplicity) | `LiveCluster::watch_count` |
| Explorer watch | the visible kind (+ its companion) | 0005/0012; every slot follows `set_explorer_kind` |
| Metrics polls | pod and node metrics on the existing interval while the API is reachable | 0010 `ClusterMetrics` |
| Kind counts | the existing one-shot count lists, rate-limited per session | `refresh_kind_counts` (C11) |
| Drawer watches, YAML GET, kubelet stats, log streams | **only** for the drawer subject's / tab's slot | decision 22 |

Not added: no extra watch for the Cluster column, no cross-cluster cache, no polling of non-viewed clusters (0026 probes run only while the switcher is open).

## Limits

| Limit | Value | Where |
|---|---|---|
| Viewed clusters | 5 (`MAX_VIEWED_CLUSTERS`) | `order_wanted`, `toggle_tick` |
| Namespaces per scope | 5 (`MAX_NAMESPACES`, shared by all slots) | picker |
| Connect fan-out on apply | sequential `ClusterSession::new` calls; each session's own connect runs on tokio concurrently (≤ 5) | `view_clusters` |
| Merged rows | no cap beyond the per-session lists; `merge_rows` allocates one `Vec` of refs + one `Vec<RowAddress>` (8 bytes per row) per rebuild | `cluster_rows.rs` |

## Memory targets

- Idle single cluster stays under C13 (< 150 MB on a ~1,000-pod cluster).
- Each extra slot: **≤ 40 MB** on a ~1,000-pod cluster (summaries, watch buffers, metrics rings, client). Five slots of that size are allowed to exceed 150 MB; the cap of 5 bounds the worst case at about 310 MB.
- A rebuild of a merged 5 × 1,000-row table stays under 16 ms in release (measure with the existing `rebuild` timing log in `table_view.rs`).

## Measurement (step 4, coder-lite)

1. Single UAT session, Pods screen, drawer closed, 2 minutes idle: record RSS (Task Manager / `ps -o rss`), `watch_count`, and the status-bar resource count.
2. Add unreachable fixture slots (0026 fixtures): verify they cost only a failed connect (no watches, no polls; RSS change < 2 MB each).
3. With a second reachable cluster (kind/k3d, risks R2) when available: RSS delta per slot, merged rebuild time with both. Record the numbers in this file under "Results" and adjust the cap if one slot exceeds 40 MB.
4. Apply {UAT, fixture} → {UAT}: the fixture slot's entity is released (weak handle), UAT's session id unchanged.
5. Drawer open on a UAT pod (Monitor tab) while two slots are viewed: only UAT's watch count grows (object events, related, kubelet demand); the other slot's count and demand stay at the closed-drawer values.

## Results

(to be filled by step 4)

| Run | Pods | Slots | RSS | Watches | Merged rebuild |
|---|---|---|---|---|---|
| | | | | | |
