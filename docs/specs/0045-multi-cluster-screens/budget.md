# 0045 · Performance with N clusters

[Back to index](README.md) · Steps 1–3 · Builds on [0027 budget.md](../0027-multi-cluster/budget.md) (cap `MAX_VIEWED_CLUSTERS` = 5, ≤ 40 MB per extra slot).

## Cost per extra viewed cluster

| Screen | Watches | CPU per rebuild or render | Memory |
|---|---|---|---|
| Issues | **0 new**: every slot already runs its pods, nodes, and `IssueFeeds` watches and its board (`ISSUE_TICK`) | `merge_rows` + stable sort: O(I log I), I = issues over all slots (hundreds) | one `Vec` of refs + 8 bytes per `RowAddress` |
| Title bar, sidebar | 0 | O(N) sums per render | — |
| Overview | **+2 per slot while Overview is shown** (the rollout and rescale change feeds, 0021); 0 when hidden | `capacity_model` and `heat_cells` per slot per render: O(pods + nodes) each, as for the primary today; `merge_attention` O(I log I); `merge_changes` O(C log C) | per-render temporaries only |
| Topology | **0**: feeds run in the topology slot only; switching stops the old slot's first | unchanged (one graph) | unchanged |
| Export | 0 | one `live_report` per slot, on demand | the report string |

Worst case at the cap: Overview shown with 5 slots adds 8 change-feed watches over single mode; Issues adds none.

## Rules that keep it bounded

- No new list, watch, or poll is started for a screen that is not shown (`set_overview_visible`, `set_issues_visible`, `set_topology_subject`).
- No cross-cluster cache: merges borrow each slot's own lists for the length of one rebuild or render.
- Repaint: a slot's board notifies only when its issues or coverage change, or on the Issues time refresh (0020); the table rebuild runs once per frame however many slots notified (GPUI coalesces `cx.notify`).
- Attention shows 6 rows and Recent changes keeps the 0021 window; neither renders the whole merged list.

## Targets

| Item | Target | How |
|---|---|---|
| Issues rebuild, 5 slots × 400 issues | ≤ 16 ms release | `issues_merge_budget` (debug gate ≤ 80 ms, prints the time) |
| Overview render data, 5 slots × 1,000 pods × 60 nodes | ≤ 16 ms release | `overview_merge_budget` over `capacity_model`, `heat_cells`, `merge_attention`, `merge_changes` (debug gate ≤ 80 ms, prints the time) |
| Watches, Overview shown | single + 2 × (N − 1) | `overview_feeds_run_on_every_slot` (headless, live_fixture ×2) |
| Watches, Topology cluster switch | old slot back to its closed count before the new one starts | `switching_topology_cluster_stops_old_feeds` |

## Measurement (step 3, coder-lite, read-only)

1. UAT + unreachable fixture, `--screen overview-multi`, 2 minutes idle: RSS and `Watching {k}` versus `--screen overview` single. Expect +0 for the failed slot.
2. The same for `issues-multi` and `topology-multi`.
3. Release build: run the two budget tests with `--release -- --nocapture`; record the times.

## Results

To be filled by step 3.

| Run | Slots | RSS | Watches | Time |
|---|---|---|---|---|
| | | | | |
