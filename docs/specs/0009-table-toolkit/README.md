# 0009 — Table toolkit (read-only)

Status: amended after advisor review. Crates: `crates/cluster` (step 1), `crates/app`. Based on HEAD `becadca` (0007 done); 0008 lands first, and 0009 builds on top. Wireframes: W1 (filter bar), W4 (`38 of 1,284 match`, chips, checkboxes), W5 (summary chips, Columns ▾, selection bar), W7 (filter bar, Columns ▾, Events Pause stream / Filter similar, ReplicaSets Hide inactive), the keyboard map (`/`), and the title bar `ns: payments, web`.

## Goal

Every table (Pods, Nodes, the 11 explorer kinds) gets one shared toolkit:

- a `/` quick filter, filter chips (Status, label query, column value), and `N of M match`;
- column sort from a header click, and Columns ▾ to show or hide columns;
- Nodes summary chips used as filters, plus a version skew tint; ReplicaSets Hide inactive;
- a multi-namespace scope picked in the title bar and shown as Namespace chips;
- Events: Pause stream and Filter similar. Node menu: View pods on node;
- row checkboxes, Ctrl/Shift click, and the W5 selection bar (count, bulk actions shown disabled, clear).

## Non-goals

Enabled bulk or mutating actions (0032–0034), a group row menu (W4 note 1, in 0032), the keyboard map beyond `/` (0028), persisted settings (0024), sidebar counts for every kind (C11, deferred: decision 23), the Cluster column (0027), a typed namespace entry, and saved filter presets.

## Implementation steps

Each step is one coder pass that passes the full gate on its own. Step 5 can move to 0032 without touching steps 1–4.

| Step | Scope | ACs |
|---|---|---|
| 1 | `crates/cluster`: `NamespaceScope::Several`, merged scoped watches and lists, combined access review, `PodSummary.labels`, probe `--namespace a,b`; app match arms only | 1, 2, 3, 5 |
| 2 | App core: `TableView`, `TableRow`, filter, sort, column layout; three delegates read through the view; filter bar, `/`, chips, + Filter, Columns ▾, `N of M`; `--filter` | 1, 2, 3, 4, 6, 9 |
| 3 | Screen extras: Nodes summary chips, roles, skew; Hide inactive; View pods on node; Pause stream; Filter similar | 1, 2, 4, 7, 8, 9 |
| 4 | Multi-namespace picker, Namespace chips, `--namespace a,b` | 1, 2, 4, 5, 7, 9 |
| 5 | Checkbox column, Ctrl/Shift click, selection bar with disabled bulk actions, `pods-selected` and `nodes-selected` screens | 1, 2, 4, 9, 10 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with one-line rationales, superseded decisions, performance budget, ceilings |
| [cluster-scope.md](cluster-scope.md) | step 1: `Several` scope, merge semantics, access review, pod labels, probe |
| [table-view.md](table-view.md) | step 2: `TableView`, `TableRow`, `CellValue`, filter and sort rules, delegate wiring |
| [filter-bar.md](filter-bar.md) | step 2: header, filter bar, chips, `/` input and binding, sort header, column layout, Columns ▾, empty state |
| [screen-filters.md](screen-filters.md) | step 3: Nodes summary, Hide inactive, View pods on node, Pause stream, Filter similar |
| [namespace-picker.md](namespace-picker.md) | step 4: picker popover, chips, launch flag, watch and RB effects |
| [row-selection.md](row-selection.md) | step 5: checkbox column, Ctrl/Shift click, selection bar |
| [files-to-touch.md](files-to-touch.md) | modules per crate and step, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [ ] 1. The quality gate passes, and so does `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes, offline.
- [ ] 3. No kube or k8s-openapi type in a public signature. The 0001 read-only grep still finds only the SSAR `create`. The crate never spawns tasks. The app gains no kube dependency. No `Cargo.lock` package change.
- [ ] 4. The 0003 AC4 color-literal grep is clean; chips, the summary tint, and the selection bar use theme tokens.
- [ ] 5. On UAT, probe `--namespace a,b` prints a pod count equal to the sum of the two single-namespace runs; the app with `--namespace a,b` shows both namespaces' pods and the label `ns: a, b`. A denied namespace in the set leaves the others visible with a banner naming it.
- [ ] 6. On UAT (release build): `/` (pressed before any click) text, a `label:` chip, a Status chip, sort on three columns, and hiding a column work on Pods, Nodes, Deployments, and Events; the header reads `N of M match`; a drawer whose row is filtered out closes (review plus spot check); the logged rebuild time stays within the decision 1 budget.
- [ ] 7. Watches per session stay at most `2N + 3` (namespaces, nodes, object events, N pods, N explorer) for N picked namespaces (N ≤ 5), and drop back when the scope changes (review of `set_scope`).
- [ ] 8. Pause stream holds the Events rows still while new events arrive; Resume shows them; a scope change or Warnings only toggle unpauses.
- [ ] 9. The step's screenshots exist; the ui-verifier reports no high-severity defect against W4, W5, W7.
- [ ] 10. A checkbox click never opens or retargets the drawer; every selection bar action is disabled with "Read-only mode" (spot check).

## Open items

1. C11 sidebar counts for every kind (inventory N3) need a new crate count API; move to 0012 or 0020.
2. A user who cannot list namespaces can pick several only through `--namespace a,b`; a typed entry in the picker would fix it.
3. Filters, sort, and hidden columns live in memory; 0024 persists sort and columns.
4. If a release trace shows a view rebuild above 4 ms (expected with 0018 custom resources), move it to the tokio row pipeline (decision 1).
