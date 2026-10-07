# 0034 · Drain dialog (W6)

[Back to index](README.md) · Step 3a · Modules: `drain_plan.rs` (new, pure) + `drain_plan_tests.rs`, `drain_dialog.rs` (new). Decisions 15–24, 37–38. Wireframe: W6 and its notes 1–5; W5 note 5 ("Drain always opens the dialog").

## Opening

- Triggers: menu `Drain…`, key D, and the palette all end in the `Drain` arm of `run_available_row_key` (cursor node, its own slot); selection bar `Drain…` (ticked nodes, the 0032 `batch_intent` selection rules: one cluster, ≤ 50). Every trigger opens the dialog; nothing runs from a key.
- Gate: `action_availability(Drain, guard)` (`CreatePodEviction`), then the shell check: a drain running on this cluster → `A drain is already running on {cluster}`.
- Loading, on the runtime, on `slot_live(&cluster)?.connection()` (never the primary): `drain_pods(node)` per node and one `list_pod_disruption_budgets()`; rows show `Loading pods…`. A failure shows `Could not list pods on {node}: {error}` and Drain stays disabled.

## Plan (pure)

```rust
pub(crate) struct DrainOptions { pub(crate) ignore_daemon_sets: bool /* default true */, pub(crate) delete_empty_dir: bool,
    pub(crate) force_unmanaged: bool, pub(crate) grace: GracePeriod, pub(crate) timeout: Duration /* default 5 min */ }
pub(crate) enum DrainOption { IgnoreDaemonSets, DeleteEmptyDir, ForceUnmanaged }
pub(crate) enum PodVerdict { Evict(Budget), Needs(DrainOption), Refused(SharedString), Terminating, Skip(SkipReason) }
pub(crate) enum Budget { None, Allows { name: String, allowed: u32 }, Waits { name: String, allowed: u32 }, Blocked { name: String, cause: BlockCause } }
pub(crate) enum SkipReason { DaemonSet, Mirror }
pub(crate) fn pod_verdict(pod: &DrainPod, budgets: &[PodDisruptionBudgetSummary], options: &DrainOptions, rank_in_budget: u32) -> PodVerdict;
pub(crate) fn node_plan(node: &str, pods: &[DrainPod], budgets: &[PodDisruptionBudgetSummary], options: &DrainOptions) -> NodePlan;
pub(crate) fn drain_blocker(plans: &[NodePlan]) -> Option<SharedString>;
```

`pod_verdict`, first match wins (kubectl drain filters, plus the 0013 PDB state):

| # | Pod | Verdict |
|---|---|---|
| 1 | mirror (static) | `Skip(Mirror)`, always |
| 2 | terminating | `Terminating`: not evicted, awaited |
| 3 | finished (Succeeded/Failed) | `Evict(Budget::None)`: the API deletes terminal pods without a PDB check; checked before row 4, so a finished DaemonSet pod is evicted, as kubectl does |
| 4 | controller kind `DaemonSet` | option on → `Skip(DaemonSet)`; off → `Needs(IgnoreDaemonSets)` |
| 5 | no controller | option off → `Needs(ForceUnmanaged)` |
| 6 | `has_empty_dir` | option off → `Needs(DeleteEmptyDir)` |
| 7 | PDBs of its namespace whose `Selector` matches its labels | pending (not yet running) → `Evict(None)`: the eviction API skips the PDB check for pending pods; else 0 → `Evict(None)`; ≥ 2 → `Refused("Matches {n} PDBs; the API refuses to evict it")`; 1 → by `disruption_state()` (0013): `Blocked(cause)` → `Blocked`; `Allowed(n)`: rank on this node ≤ n → `Allows`, else `Waits`; `NoPods` → `Evict(None)` |

Rank: the pod's 1-based position among this node's pods of the same PDB, in plan order. It explains why the second pod of a budget that allows one waits.

## Preview list (W6 note 3: blocked pods float to the top)

Order: `Refused`, `Blocked`, `Needs`, `Waits`, `Allows`, `Evict`, `Terminating`, then one grouped row per skip reason. Scrollable, max 240 px; multi-node plans get one header per node.

| Verdict | Result text | Tone |
|---|---|---|
| `Refused` | the text above | bad |
| `Blocked` | `Blocked by PDB {name} (0 allowed)` | bad |
| `Needs(ForceUnmanaged)` | `No controller, needs Force` | warn |
| `Needs(DeleteEmptyDir)` | `Uses emptyDir, needs Delete emptyDir data` | warn |
| `Needs(IgnoreDaemonSets)` | `DaemonSet pod, needs Ignore DaemonSet pods` | warn |
| `Waits` | `Waits on PDB {name} (allows {n})` | warn |
| `Allows` | `PDB {name} allows {n}` | ok |
| `Evict` | finished (Succeeded or Failed) → `Finished · removed` (muted); unmanaged → `Will not come back` (warn); emptyDir → `Loses local data` (warn); else `Will be rescheduled` (ok) |
| `Terminating` | `Already terminating` | muted |
| skips | `{n} DaemonSet pods · Skipped`, `{n} static pods · Skipped` | muted |

## Options (W6 note 2: kubectl flags with consequences and counts)

| Checkbox | kubectl | Small text (counts from the pods, whatever the state) |
|---|---|---|
| `Ignore DaemonSet pods` (on) | `--ignore-daemonsets` | `{n} pods stay on the node` |
| `Delete emptyDir data` | `--delete-emptydir-data` | `{n} pods lose local scratch data` |
| `Force unmanaged pods` | `--force` | `{n} pods have no controller and will not come back` |
| `Skip PodDisruptionBudgets` (danger) | `--disable-eviction` | `Deletes pods directly. Can cause downtime.`; **disabled**: `Comes in a later version` (needs 0033) |

`drain_blocker`: any `Needs(o)` → `{n} pods need "{option}": tick it, or use Cordon only`; any `Refused` → `{pod} cannot be evicted: {text}`. Drain is disabled while it is `Some`; Cordon only is not.

Row: `Grace period` select `Pod default` · `10 s` · `30 s` · `60 s` · `120 s`; `Timeout` select `2m` · `5m` · `10m` · `30m` (per node).

## Parts (W6 top to bottom)

1. Title: env badge + `Drain node wk-04?` / `Drain 3 nodes?`.
2. Steps strip: `1 Cordon: stop new pods` · `2 Evict {n} pod(s)` · `3 Wait until done or timeout`. n counts only pods the drain moves: `Evict` verdicts that are neither finished nor pinned by a volume (see below); pods that cannot move follow as `· {k} cannot move`. A strip over nodes that are all cordoned drops step 1 and renumbers. The preview header counts the same way (`Pods to evict · {n} · {k} cannot move`).
3. Options, grace and timeout, preview list.
4. HEADS UP, one line per finding in one box (only with something to say): a budget that needs every pod it has (`Blocked(NoRoom)`: all healthy at the minimum) reads `{pdb} cannot lose a pod (minAvailable 2 = 2 healthy); this drain will stop at the {timeout} timeout. Options: Skip PDBs, scale {pdb}, Cordon only`; any other `Blocked` or `Waits` reads `Drain will wait on {pdb} until a replacement pod is ready elsewhere, or stop at the {timeout} timeout.` (`{pdb} and {k} more` for several); a pinned pod adds `{pod} cannot move: its volume {claim} lives on this node, so its replacement stays Pending until the node is back.`
5. Dry-run line, typed name, 0030 note checkbox. The dry-run line takes the warning tone when the server refused evictions and accepted none.
6. Buttons: `Cancel` · `Cordon only` · `Drain` (one node: its name is in the title) / `Drain 3 nodes` (danger primary).

## Volumes that live on the node (L1)

A pod that mounts a PersistentVolumeClaim bound to a volume with a `kubernetes.io/hostname` (or `metadata.name`) node affinity naming this node cannot move: its replacement stays Pending once the node is cordoned. `pin_volumes(node, pods)` (read-only: one node GET and one volume list, only when some pod mounts a claim; a failure leaves the pods unmarked) sets `DrainPod.pinned_volume`. The pod stays an `Evict` verdict (the drain still evicts it) but its row reads `cannot move: volume {claim} lives on this node` (warn; a `Blocked` budget keeps its own text), it sorts after the budget rows, and the counts above and the HEADS UP line name it. Zone affinities and finished pods never pin.

## Placement check (round 3, Q1/Q2)

The dialog also lists the cluster's nodes (read-only) and checks where the replacement of each evicted, controller-owned, unpinned pod can go: a node other than the drained ones that is Ready and schedulable, whose `NoSchedule`/`NoExecute` taints the pod tolerates and whose labels match its `nodeSelector` and required node affinity (`cluster::PodPlacement::misfit`; resources, ports, volumes and pod affinity are not checked). When no node fits, HEADS UP says `No other node fits {pod} (taints on worker2): its replacement stays Pending.` (`No other node fits 14 pods (taints on a, b): their replacements stay Pending.` for several); the line's tooltip lists the pods (12, then `and N more`) and each node's taint keys or selector. While a HEADS UP box shows, the pod list is 5 rows tall instead of 8, so the box and the dry-run line below it grow to their text inside the body's height (630 px, which keeps the buttons in view at 1320×900). A pinned pod whose own node carries a taint it does not tolerate (the cordon's `unschedulable` taint does not count) reads `... so its replacement stays Pending, and it cannot come back here either until taint workload=data is tolerated.` A node list that cannot be read leaves both out.

## Dry-runs and confirm

- After loading, one request at a time, each `checked_write(WriteStep { mode: DryRun, .. })` (no audit): 0030 `SetNodeSchedulable { false }` per node not yet cordoned, then `EvictPod` for every `Evict` pod in list order. Ticking an option dry-runs the pods it adds.
- Per-pod result: success keeps the local text, except that it **downgrades** a local `Blocked` or `Waits` guess to `Dry-run accepted` (ok tone: the server would evict it now); `TooManyRequests` → the short form `PDB {name}: 0 allowed ({has}/{needs} healthy)` read from the API message (`PDB {name}: 0 allowed` when it has no numbers, `Blocked by PDB: {message}` when it names no budget), with the full message as the cell tooltip (server truth); any other error → `Failed: {error}` (bad).
- Aggregate for `commit_block`: `Running` until all finished; `Failed` when a cordon dry-run failed or an eviction dry-run failed with anything but 429; else `Passed`. A 429 is an expected wait, not a failure.
- Dry-run line: `Server dry-run: cordon passed · 21 of 23 evictions accepted, 2 refused by PDB`.
- Confirm: risk `Destructive`; tier from the merged `confirm_step(guard.profile.confirm, ActionRisk::Destructive, expected)`. `expected` = the node name for one node (W6, 0030 decision 10), the cluster display name for several. Enter rules of 0030 (held Enter never confirms). Confirming builds 0030 `confirmed(&aggregate, typed, generation)`; `None` keeps both buttons disabled.
- `Cordon only` (gated `PatchNodes`): needs the same `Confirmed`; commits `SetNodeSchedulable { false }` per uncordoned node through `checked_write(WriteStep { intent: <cordon GuardedIntent>, mode: Commit { confirmed }, .. })`, then closes. No eviction is sent.
- `Drain …`: closes the dialog and starts the run (drain-run.md) with the plan, options, generation, the `Confirmed`, and the note. In step 3a the button is shown disabled with `Comes in a later version`; step 3b enables it.
- The drain dialog is a W6 front end, not a `GuardedKind`: it reuses `confirm_step`, `TypedMatch`, `confirmed()`, the 0030 Enter rules, and sends every request through `checked_write`, so lock, generation, and audit stay in one place.

## Preview mode (a session that cannot drain)

`Drain…` (node menu, selection bar, `D`, palette) always opens the dialog. When `action_availability(Drain)` is `Disabled` (read-only, locked, a missing right), the dialog is a **preview**: it loads the plan but sends **nothing**, not even a dry-run (`PodCheck::NotChecked`: a pod with no plan warning reads `Not checked (preview)`, one the plan already flags keeps that text, and the dry-run line says `Not checked: a preview sends no requests`), titled `Drain preview: node {name}`, but shows no typed-name field, note, or confirm buttons. The gate's reason takes their place, with only a `Close` button; `drain_block` and `cordon_block` return it, so Enter and the buttons never commit. The menu item and the bar button stay enabled; the item's hint reads `Preview only` and the full reason is the tooltip (the bar's tooltip is `Preview only · {reason}`; `BulkState::Preview`).

## Pod tags in the node drawer

The Pods section of the node drawer (above Addresses) tags each pod from facts the pods list already has: `DS` (DaemonSet controller), `emptyDir` (an emptyDir mount), `PDB 0` (a budget that selects it allows no disruption now, from the PDB condition feed), `no controller`. A finished pod has none. The tags derive from `PodSummary` and the feed, not from `drain_plan`, which reads `DrainPod`.
