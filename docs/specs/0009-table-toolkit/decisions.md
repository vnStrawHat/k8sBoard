# 0009 · Decisions

[Back to index](README.md). Architect defaults; the user asked not to stop for questions. Amended after advisor review (M1–M2, S1–S7, N1–N7).

## Filter and sort

| # | Decision | Rationale |
|---|---|---|
| 1 | Filter and sort run **on the main thread**, over rows borrowed from the session, into a `Vec<usize>` view; only the visible table rebuilds | no row copies, no race between a view and a newer snapshot, instant feedback on typing; the budget below holds |
| 2 | One `TableView` per table (one per kind for the explorer), owned by its delegate; one `TableRow` trait with three impls (`PodSummary`, `NodeSummary`, `KindRow`) | one filter model for every kind; three real implementations justify the trait |
| 3 | Filter = quick text **and** every chip **and** the preset | AND is what the wireframe chips read as; no query language to learn |
| 4 | Quick text: ASCII case-insensitive substring over namespace, name, every column's text (shown or hidden), and label terms | "CrashLoop", "payments/api", and "app=api" all work with no syntax |
| 5 | Chips: `Unhealthy` (tone Warn, Bad, or Info), `Equals { column, value }`, `Label(LabelQuery)`. Presets: `HideInactive`, `Nodes(NodeGroup)` | tone-based status works for every kind; `Equals` serves Filter similar and View pods on node |
| 6 | A label chip is typed as `label:k=v,k2!=v,k3` in the `/` input and created on Enter | matches the wireframe's mono `label:app=api` chip; no dialog |
| 7 | Sort is our own header click (asc, desc, none) with the kit's `sortable(false)`; stable sort over source order | the kit sorts only from its icon and keeps state that `refresh` resets; one source of truth in the view |
| 8 | Text sorts in natural order (digit runs as numbers) | counts are `KindCell::Text` ("10" after "7"), plus `pod-2` < `pod-10`, `v1.29.5`, IPs |
| 9 | Absent values sort last in both directions; Status sorts by severity (Ok, Done, Info, Warn, Bad); Age ascending = youngest first | users sort to see values and problems; "ascending" follows the displayed number |
| 10 | Columns ▾ hides any column except the flexible one; it sits at the right of the filter bar on every screen, Nodes included (W5 draws it in the header) | the flexible column fills the width (0003); one place for every table, as in W7 |
| 11 | Filters, sort, and hidden columns stay per table (per kind) for the app run; a context switch resets filters to the kind's default | wireframe principle: filters stay while switching pages; a filter in a new cluster surprises |
| 12 | A filter that hides the drawer's row closes the drawer | same rule as 0006 Warnings only; no drawer for an invisible row |
| 13 | `/` binds with context `AppShell && !Input`; the root element has `.key_context("AppShell")` | works without table focus, never steals `/` from an input; the full keymap is 0028 |
| 26 | ReplicaSets start with **Hide inactive on** (the kind's default filter) | W7 shows the toggle for a list cluttered with old revisions; supersedes 0005 decision 23. UX walk H13: the header toggle is gone (it duplicated the `Hide inactive ×` chip); the Filter menu keeps a checkable Hide inactive item to turn it back on; ReplicaSets add a numeric Revision column |

## Scope, screens, selection

| # | Decision | Rationale |
|---|---|---|
| 14 | Multi-namespace = **one watch per namespace**, merged in the crate (`NamespaceScope::Several`), not All plus a client filter | works for users who may list only some namespaces (the main reason to pick several); no cluster-wide download |
| 15 | At most `MAX_NAMESPACES` = 5 | bounds watches (2N + 3 ≤ 13 per session) and SSARs (16N + 3 ≤ 83 per scope change) |
| 16 | A merged list shows every namespace that answers; a failing namespace is an interruption naming it ([cluster-scope.md](cluster-scope.md)) | partial data with a visible reason beats hiding the healthy namespaces |
| 17 | Access review of `Several`: namespaced checks per namespace, AND-combined; cluster checks once | conservative **gating** of menus and sidebar items only, never data filtering |
| 18 | Picker = popover with checkboxes and Apply; clicking a name picks only that namespace | W1 "apply once" (no re-watch while ticking) and W1 "click the name to switch" |
| 19 | Namespace chips mirror the scope; × removes a namespace; `Namespace: all ▾` is a second trigger of the same popover | one state; the W7 filter bar chip and the title bar open the same picker |
| 20 | `PodSummary.labels` moves forward from 0012 | the label chip is drawn on Pods (W4) |
| 21 | Nodes summary chips are one single-choice preset; skew = any version other than the most common one | W5 note 1; "All 12" is the cleared state |
| 22 | Pause stream holds the newest snapshot in the session's `KindList` and shows the frozen rows; every list restart unpauses | rows and selection stand still (fixes 0006 open item 4); at most one extra snapshot in memory |
| 23 | C11 sidebar counts are **deferred** | not in the caller's scope, independent of tables, needs a new crate API |
| 24 | Multi-select = checkbox column, Ctrl+click toggle, Shift+click range (through `render_tr` click modifiers), and the W5 selection bar: count, the screen's bulk actions **disabled** ("Read-only mode"), ✕; pruned to visible rows | the wireframe implies multi-select; real bulk actions are 0032–0034 and must never touch hidden rows |
| 25 | Launch flags `--filter <text>`, `--namespace a,b`, `--screen pods-selected` and `nodes-selected` | the ui-verifier cannot click; these reach the new states |

## Supersedes

0005 decision 23 (inactive ReplicaSets shown) → decision 26. 0006 open item 1 (Pause, Filter similar) and open item 4 (selected event moves) → decision 22 and [screen-filters.md](screen-filters.md). 0006 decision 19 (event menu without Filter similar) → Filter similar added after Go to object. 0008 decision 18 (no View pods on node) → added; the node drawer's Pods section stays.

## Performance budget (decision 1)

| Item | Cost |
|---|---|
| Rebuild triggers | session notify (batched 100 ms), a toolkit change, a screen switch; visible table only |
| Filter | O(n × c) substring checks, c ≈ 6–8 values; no allocation except owned `CellValue`s |
| Sort | keys computed once (n `value` calls), then a stable sort of `(key, index)` pairs: O(m log m) |
| Expected | ≤ 2 ms for 5,000 rows in a **release** build; debug builds are several times slower and do not count |
| Evidence | `rebuild_view` emits `tracing::trace!(rows, kept, elapsed_us)` (counts only); coder-lite records the maximum on UAT Pods and Events in a release build |
| Rejected | tokio pipeline: each filter change would round-trip, rows would need `Arc` sharing, and views would race snapshots |

## Known ceilings

- Rebuilds also run on notifies of other lists (namespaces, access); measure before adding a generation counter.
- A merged snapshot from healthy namespaces clears the interruption until the failing namespace's next retry fails again (watcher backoff), so the banner may blink.
- A merged emit clones the other namespaces' latest lists (N ≤ 5, at most once per window).
- Events with `Several`: each namespace store keeps 2,000, the merge trims to the newest 2,000.
- A Ctrl/Shift click also lets the kit select that row, so the drawer follows it.
