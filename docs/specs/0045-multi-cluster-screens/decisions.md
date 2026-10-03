# 0045 · Decisions

[Back to index](README.md). Architect defaults recorded without a user round-trip; each follows the 0027 contract unless stated.

| # | Decision | Rationale |
|---|---|---|
| 1 | Issues and Overview **merge every Live slot**; Topology stays **one graph** with a cluster choice | 0027 "Screens that land later" and decision 21; a graph over clusters has no shared namespace |
| 2 | Merged issues are in **board order across slots**: (severity, since, shown), ties by slot order; a user column sort still wins | the most urgent problem is first whichever cluster has it |
| 3 | `set_issues_visible` and `set_overview_visible` go to **every slot** while the screen is shown | the boards already run per slot; Overview's change feeds are its only extra cost (budget.md) |
| 4 | Counts **sum the slots whose board summary is known** (the `sum_known` rule of 0027 decision 19); tooltips list each cluster | a slot still checking must not show as zero |
| 5 | Overview: Needs attention merged (first 6, cluster chip); Capacity and Nodes **one block per cluster**, primary first; Recent changes merged newest first; stats line summed; header `{n} clusters` | capacity percentages of different clusters do not add up; issues and changes do |
| 6 | Export: one Markdown file, one section per Live slot, joined by `---`; canceled when the viewed set changes during the dialog | the file must describe what the screen showed |
| 7 | Every click carries the **`ClusterObject` of the slot it was drawn from**; `reveal_in_primary` is deleted | the same name in two clusters must open the right one (0027 decision 15) |
| 8 | `AppShell.topology_cluster` (default primary) picks the Topology slot through a **header dropdown** shown only in multi mode; Show in Topology sets it; a release or a single switch clears it | W11 toolbar stays as built; the header already holds Fit and Export |
| 9 | Switching the Topology cluster **stops the old slot's feeds first** (`set_topology_subject(None)`) | the old slot stays alive in multi mode; feeds must never run twice |
| 10 | Failed and Connecting slots appear **only as the 0027 banners**; the `Showing {label} only` notice and the primary-only bodies are removed | one place for slot state |
| 11 | Overview and Issues get **no cursor**; Topology node clicks set the cursor in the topology cluster, so the `RowAction` key layer needs no change | keys already read `slot_live(&subject.cluster)` |
| 12 | Single mode renders **exactly as before** (no chips, no Cluster column, same header texts) | AC 11; one slot is the same code path |
| 13 | `cluster_rows::merge_rows` is reused for issues (items are the board slices); no new generic merge type | extend existing code (structure rules) |
| 14 | Test seam `ClusterSession::seed_nodes` (`#[cfg(test)]`, like `seed_pods`) so a seeded board reaches `is_core_ready` | 0027 view-model.md allows it "when a test needs them" |
