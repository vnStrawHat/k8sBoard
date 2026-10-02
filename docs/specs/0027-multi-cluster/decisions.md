# 0027 · Decisions

[Back to index](README.md). Defaults picked without a user round-trip (user instruction); amended after the advisor review.

| # | Decision | Rationale |
|---|---|---|
| 1 | Multi mode is entered and left **only** through the title-bar switcher; no rail, no other entry point | user decision recorded in the wireframe intro and W1 |
| 2 | One `ClusterSession` per viewed cluster (C4); the view is an ordered list of slots | sessions already own their watches; no shared store to invent |
| 3 | **Apply once**: ticks are a draft; "View {n} clusters" or Enter (when ticks differ from the view) applies | W1 note 4: no re-watching while choosing |
| 4 | Applying a set **keeps** sessions that stay, releases removed ones, then connects added ones in one `cx.defer` (0026 decision 1 per slot); a name click / Ctrl 1–9 is 0026 single switch | no reconnect for clusters that stay; peak = max(old, new) |
| 5 | At most **5** viewed clusters (`MAX_VIEWED_CLUSTERS`); a sixth tick is refused with `View at most 5 clusters at once.` | C13 budget; mirrors the 5-namespace cap |
| 6 | Slots are in switcher display order. **Primary** = the current single (or primary) cluster when it is in the new set, else the first in display order. `riskiest()` is used only for the border | the cluster the user stood on stays primary; filters, `last_used`, and the trigger label follow it |
| 7 | Trigger: primary badge + label + muted `+{n−1}` chip (W1 `.ctx .plus`) | W1 note 1 |
| 8 | Top border: `environment_color(max(slot environments))` | 0024 decision 26 for several clusters; `Environment: Ord` by risk |
| 9 | Space toggles the highlighted row's tick, bound on `ClusterSwitcher` and `ClusterSwitcher > Input`; Space never types in the filter, and the filter ignores whitespace | W1 note 2; both nodes are deeper than the kit `Popover` (`space → Confirm`), so depth decides |
| 10 | Ticks start as the viewed set in both modes (single: `[current]`). The footer and Enter-apply appear only when ticks differ from the viewed set; otherwise Enter is the 0026 switch to the highlight | one model; no footer when nothing would change |
| 11 | Filters are kept when the primary stays, else reset (0009 decision 11) | a new primary is a new context |
| 12 | One namespace scope for every slot; a slot going Live re-applies the view scope if it differs. The picker lists the union over Live slots with a ready namespace list, plus a muted `Loading namespaces of {label}…` for the rest | W1 chips apply across clusters |
| 13 | Rows are `Clustered { slot, label: switcher_label, item }`; the **Cluster column is appended last** in multi mode | W1 shows Cluster last; other logical indices stay stable |
| 14 | Persist only the primary as `last_used`; restart opens one cluster | cheap restart; open item 2 |
| 15 | Selection, row checks, reveal, pending subjects, YAML view, and log tabs carry a `ClusterRef` (`ClusterObject { cluster, key }`) | same names exist in several clusters |
| 16 | A failed slot shows a banner (Retry, Remove from view); an interrupted slot shows `Live updates interrupted in {label}`; the error view only when every slot failed | the other clusters stay useful |
| 17 | Every drawer, YAML, logs, and Monitor read uses `slot_live(&selected.cluster)`, never `live(cx)` | the drawer subject's cluster owns its data |
| 18 | Log tabs keep one stream per pod; titles get ` · {label}` in multi mode; releasing a slot closes its tabs | merged logs are later; correctness first |
| 19 | Sidebar counts are the sum over slots that know the count; tooltip per slot | C11 counts already per session |
| 20 | Status bar: `Watching {k} resource types · {n} clusters`; the `API {min}–{max} ms` range is optional (single value allowed) | one line |
| 21 | Topology stays single-cluster: a dropdown of the viewed slots (default primary) | contract for 0022 |
| 22 | Subject-scoped calls (`set_event_subject`, `set_related_subject`, `set_kubelet_demand`) go to the subject's slot only; every other slot gets `None` / `KubeletDemand::default()` | no fan-out of per-subject work |
| 23 | Cluster column sort and hidden state are session-only: `TableView::prefs` skips the Cluster column; `set_sessions` drops its index from `hidden` and from `sort` when leaving multi mode | the column does not exist in single mode |
| 24 | `has_reported_live` (0024/0026) is a field of each `ViewSlot`; only the primary's writes `last_used` | per-slot Live edges |
| 25 | Row menus capture `WeakEntity<ClusterSession>` + `ClusterRef` at build time, never a slot index | slot indices shift on apply |
| 26 | Test seams: `#[cfg(test)] ClusterSession::live_fixture` and `seed_pods` (cluster_session.rs) | 0024–0026 have no code yet; Live slots must be testable offline |
