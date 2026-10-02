# 0027 — Multi-cluster views (W1)

Status: amended after the advisor review (M1–M3, S1–S10, N1–N6), HEAD `d5ccbd0`. Crate: `crates/app` only. Read-only: each viewed cluster runs the same list/watch/metrics calls a single session runs today; no new request kind. Prerequisites: 0024, 0025 steps 1–3, 0026 (all steps), 0009. Applies C4, C11, C13. Wireframe: W1 (pins 1, 2, 4, 6; notes 1–6), the "multi-cluster on the title bar" principle (no cluster rail).

User decision (wireframe intro and W1): single or multi cluster is chosen **only** through the title-bar dropdown; there is no cluster rail.

## Goal

- **Tick to view several clusters**: checkboxes (and Space) in the 0026 switcher, a footer `{n} selected · Clear · View {n} clusters ⏎`, applied once (W1 note 4). A name click still switches to that one cluster (0026, break before make).
- **Several live sessions at once**, one `ClusterSession` per viewed cluster (C4), at most 5. Applying a new set keeps sessions that stay, drops removed ones, then connects added ones.
- **Merged tables**: Pods, Nodes, and every kind screen list the rows of all viewed clusters with a **Cluster** column (badge + label), filterable and sortable (W1 note 6); header `{n} clusters · {count} {plural}`.
- Title bar: trigger `primary label +N` (W1 note 1), top border in the **riskiest** viewed environment (0024 decision 26 rule for several clusters).
- Drawer, YAML, Monitor, events, related lists, row menus, and log tabs work on the row's own cluster.

## Non-goals

- Overview, Issues, Topology merging: their specs (0020–0022) are not implemented; the contract they must follow is in [aggregated-views.md](aggregated-views.md) "Screens that land later".
- Persisting the viewed set across app runs (only the primary is `last_used`), per-cluster namespace scopes inside one multi view, merged log streams in one tab, cross-cluster bulk actions (0032–0034).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `ClusterView` model, apply plan, several sessions, deferred connects, primary rule, fan-out, scope for all, title-bar `+N` and riskiest border, test seams in `cluster_session.rs` | 1–5 |
| 2 | Switcher ticks: checkboxes, Space, footer, Enter applies, limit 5 | 1, 2, 6, 7 |
| 3a | Merged rows, Cluster column (session-only), sort and filter, header count | 1, 2, 8 |
| 3b | `ClusterObject` identity (selection, checks, reveal, pending subjects), per-slot banners, summed sidebar counts | 1, 2, 9, 10 |
| 4 | Wrong-cluster sites (`app_shell.rs` drawer/YAML/logs/Monitor reads, `YamlView`, `LogDock`, subjects, kubelet demand, menus); budget; `--screen pods-multi`; ui-verifier | 1, 2, 11–13 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [view-model.md](view-model.md) | `ClusterView`, slots, apply diff, primary, scope, settings, status bar |
| [switcher-multi.md](switcher-multi.md) | ticks, Space and the kit `space → Confirm` collision, footer, Enter, trigger, border |
| [aggregated-views.md](aggregated-views.md) | merged rows, Cluster column, selection, drawer, logs, failures, counts, later screens |
| [budget.md](budget.md) | per-extra-session watches, polls, memory, limits, measurement |
| [files-to-touch.md](files-to-touch.md) | files per step, follow-ups |
| [test-plan.md](test-plan.md) | unit, headless, live, ui-verifier |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`. `Cargo.lock` unchanged.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [ ] 3. Applying {A, B} → {B, C} keeps B's session entity (same id), releases A's before C's connect starts, and never runs more than 5 sessions; the primary is the current cluster when it stays in the set, else the first in display order.
- [ ] 4. With 2+ viewed clusters the trigger reads `{primary} +{n−1}` and the top border uses the riskiest viewed environment; with 1, 0026/0024 behavior is unchanged.
- [ ] 5. The namespace scope applies to every viewed session; the picker lists the union of their namespaces.
- [ ] 6. Ticks start as the viewed set; ticking (click or Space) never reconnects; the footer and Enter-apply appear only when ticks differ from the viewed set. A name click or Ctrl 1–9 switches to a single cluster.
- [ ] 7. Space ticks the highlighted row in the switcher filter and on rows; it never reaches the kit `Popover` `Confirm`.
- [ ] 8. In multi mode every table has a `Cluster` column (last), sortable, filterable by text and by the row-menu `Equals` chip; it is absent in single mode and never stored in prefs.
- [ ] 9. Two pods with the same namespace/name in two clusters are separate rows; selecting, checking, and opening the drawer address the right cluster.
- [ ] 10. One failed cluster shows a banner with Retry and "Remove from view", an interrupted one `Live updates interrupted in {label}`, while the others' rows stay.
- [ ] 11. Every site in [aggregated-views.md](aggregated-views.md) "Wrong-cluster sites" reads the subject's slot, with its test; log tab titles carry the cluster label in multi mode.
- [ ] 12. Budget: recorded per-extra-session numbers in [budget.md](budget.md); the closed-drawer watch count per session equals the single-session count.
- [ ] 13. Screenshot `pods-multi` (light, dark; settles when every slot is Live with a loaded list or Failed) matches W1's table (Cluster column, header count, `+1` trigger), no high-severity defect.

## Open items

1. Only one test cluster exists (UAT); multi mode is verified with UAT plus unreachable fixtures. A second reachable cluster (kind/k3d, risks R2) is needed for full live checks.
2. The viewed set is not restored after restart (decision 14).
3. The cap of 5 is a guess until [budget.md](budget.md) has numbers from a second real cluster.
4. Overview, Issues, and Topology must follow "Screens that land later" when their specs are implemented.
