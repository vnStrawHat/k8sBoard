# 0045 — Multi-cluster Overview, Issues, and Topology

Status: **Dropped (user decision 2026-10-03: single cluster only)**; multi-cluster mode is removed by [0046](../0046-single-cluster/README.md). Kept for the record; do not implement. Was: draft, 2026-10-03, against main `a50264c`. **Read-only: no new mutating calls** and no new request kind; every slot already lists, watches, and runs its issue board. Crate: `crates/app` only. Closes 0027 open items 4–6 and the "Screens that land later" contract ([0027 aggregated-views.md](../0027-multi-cluster/aggregated-views.md)). Audit gap 1. Prerequisites: 0020, 0021, 0022, 0027 (all merged). Wireframes: W1 note 6 (Cluster column in every table), W1 pin 3 (per-cluster health), W3 pins 1–4, W11 (single graph), the title-bar `⚑ N` flag.

## Goal

- **Issues** (W1 n6, title-bar flag): one table over every viewed cluster, in board order, with the Cluster column; the flag, the sidebar count, and the header sum the clusters, with per-cluster tooltips.
- **Overview** (W3): Needs attention merged with a cluster chip; Capacity and Nodes one block per cluster; Recent changes merged by time; the stats line summed; Export writes one section per cluster.
- **Topology** (W11, 0027 decision 21): still one graph, drawn for a **chosen** viewed cluster (header dropdown, default primary); Show in Topology works from any cluster.
- **Click-through** always lands in the cluster the row came from (`ClusterObject`); `reveal_in_primary` is removed.

## Non-goals

- A merged Topology graph over several clusters; per-cluster namespace scopes (0027 non-goal).
- A cluster-wide capacity total summed over clusters (units and node pools differ; per-cluster blocks instead).
- New issue rules, Overview panels, or managedFields "who" (0021 open item 1, audit 0041).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Issues: `IssueTableDelegate` sessions, merged rows in board order, Cluster column, `issue_object` click-through, row-menu cluster filter; title-bar flag, sidebar, header sums; boards visible on every slot; `reveal_in_primary` removed; `seed_nodes` test seam | 1–5, 9 (Issues), 11 |
| 2 | Overview: `OverviewSlot`, merged attention, per-cluster Capacity and Nodes, merged Recent changes (feeds on every slot), summed stats, `{n} clusters` header, per-cluster export | 1–3, 6, 7, 9 (Overview), 11 |
| 3 | Topology: `topology_cluster`, header dropdown, old-slot feeds stop, node click and Show in Topology in the right cluster; "Showing {label} only" removed; `--screen overview-multi`, `issues-multi`, `topology-multi`; budget run; ui-verifier | 1–3, 8–12 |

## Files

| File | Contents |
|---|---|
| [issues.md](issues.md) | merged table, ordering, Cluster column, counts, click-through, keys |
| [overview.md](overview.md) | `OverviewSlot`, each panel's merge rule, stats, header, export |
| [topology.md](topology.md) | topology cluster choice, dropdown, feeds hand-over, Show in Topology, drawer |
| [budget.md](budget.md) | cost per extra cluster per screen, limits, measurement |
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [files-to-touch.md](files-to-touch.md) | files per step, docs |
| [test-plan.md](test-plan.md) | unit, headless (live_fixture ×2), live, ui-verifier |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`; `Cargo.lock` unchanged; `crates/cluster` unchanged.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [ ] 3. Read-only: no new request kind; the 0030 `disallowed-methods` lint and the read-only grep are unchanged. (W1, W3, W11)
- [ ] 4. Issues in multi mode list every Live cluster's issues in one table, ordered by severity then onset across clusters, with a sortable, filterable Cluster column (last, session-only) and a `Filter by this cluster` row item. (W1 n6)
- [ ] 5. The title-bar flag, the sidebar Issues count, and the Issues header (`{n} clusters · {total} issues`) sum the clusters whose board is ready; each tooltip lists `{label}: {count}` per cluster. (W1 pin 3, title bar)
- [ ] 6. Overview in multi mode: Needs attention shows the first 6 merged issues with a cluster chip; Capacity and Nodes show one block per Live cluster, primary first; Recent changes merge by time with a cluster chip. (W3 pins 1–4)
- [ ] 7. Export report in multi mode writes one Markdown section per Live cluster; a change of the viewed set while the dialog is open cancels the save. (W3)
- [ ] 8. Topology in multi mode has a `Cluster: {badge} {label} ▾` header dropdown of the viewed clusters (default primary); switching stops the old cluster's feeds before the new ones start (watch count test). (W11, 0027 decision 21)
- [ ] 9. Every click-through (Issues row, Overview row and buttons, heat cell, change row, Topology node, Show in Topology) opens the object in the cluster it was drawn from, also when the same namespace/name exists in another viewed cluster. (W1 n6, W3 pins 1, 3, 4, W11 pin 4)
- [ ] 10. Row keys (`RowAction`) after a click-through act on the cursor's own cluster; a Topology node click puts the cursor in the topology cluster. (keyboard map, W11 pin 4)
- [ ] 11. Single mode is unchanged: no Cluster column, no chips, same headers and texts as before. (W3, W11 as built)
- [ ] 12. Budget: [budget.md](budget.md) "Results" filled; screenshots `overview-multi`, `issues-multi`, `topology-multi` (light, dark) have no high-severity defect against W1/W3/W11.

## Open items

1. Only one reachable cluster exists (UAT): live checks use UAT plus an unreachable fixture (banner path); the merge itself is proven by headless tests with two `live_fixture` slots.
2. Default kept without asking: Capacity stays per cluster (non-goal 2). Say so if a summed total is wanted.
