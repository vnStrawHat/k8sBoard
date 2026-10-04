# 0040 · Decisions

[Back to index](README.md). Architect defaults. 0030, 0033, 0034, 0036, 0037, 0046 decisions apply unless replaced here.

## Reuse (no new write surface)

| # | Decision | Rationale |
|---|---|---|
| 1 | **No new `WriteOperation`.** Restart pod = 0033 `DeleteObject { uid, Background }` on a Pod; Evict = 0034 `EvictPod { uid, PodDefault }`; skip-PDB drain = `DeleteObject` per pod; bulk labels = `SetNodeLabels` per node. `checked_operation` keeps its existing decisions for these four arms (EvictPod uid check, label key/value check, DeleteObject uid in `WriteRequest::new`) and gains no arm | every wire shape is already pinned by fake-transport tests and reviewed; a restart is a delete by definition |
| 2 | Restart and Evict reuse the 0033 start (gate → uid read → batch dialog → `checked_write`) through a `Removal` enum in `object_delete.rs`; scope is always the cursor pod | one uid-pin path; 404 and 409 already read as "gone" and "recreated" |
| 3 | **Attach lives in `debug_shell.rs`**: `attach_shell` with a new `AttachWait::Container`. `pod_shell.rs` stays exec-only | `debug_shell.rs` holds the one `attach` call site (`attach_process`) and its named exception; reusing it adds no clippy row, no `#[allow]`, no allow-list row (the `pods/attach` row gains the spec number 0040). The file name is historic; its module comment is widened |
| 4 | Attach reuses `ConnectIntent` with a new `ConnectOpen::Attach` (permit `AttachPermit`), `Dock::open_attach`, and the 0036 Shell tab with `ShellKind::Attach` | the guarded connect core (gate re-check, tier, permit at confirm, audit per start) is the 0036/0037 one |

## Attach

| # | Decision | Rationale |
|---|---|---|
| 5 | Attach only to a running Main or Sidecar container whose spec has `stdin: true` and `tty: true` (`ContainerTerminal::{Interactive, InteractiveOnce}`); else off: `The container has no terminal (stdin and tty); use View logs` | kubectl drops TTY and stdin to match the container; `drive_process` needs the terminal pipes, and output-only attach duplicates View logs |
| 6 | `ContainerSummary` gains `terminal: ContainerTerminal`, read from `spec.stdin`, `spec.stdinOnce`, `spec.tty` | the menu must know before the dialog; three spec booleans, no churn |
| 7 | A picks the default attach container: first such running Main, else first such running Sidecar. The pod menu item has no submenu (W4 draws none); the container ⋯ menu attaches to its container | W4 "Attach A", W4b note 3 |
| 8 | Risk `Change`, the cluster's own tier (PROD types the cluster name); warnings name the main-process hazard and `stdinOnce` | attach writes to PID 1's stdin; Ctrl C can stop the container |
| 9 | Reconnect on an attach tab re-attaches the same container through the same intent; `is_debug_tab` becomes true for `Debug` and `NodeShell` only | an attach creates nothing, so repeating it is safe; today `!is_exec()` would send Attach to the debug options dialog |

## Restart pod (bare pods: refused)

| # | Decision | Rationale |
|---|---|---|
| 10 | **Restart pod is refused for a pod with no controller** (`Not managed by a controller; it would not come back. Use Delete pod…`), not offered with a stronger warning | "Restart" promises a new pod. A bare pod deleted is gone: that is Delete, which already exists with its typed name and its `will not come back` warning. One honest action per outcome beats a dialog that relabels a delete |
| 11 | Also refused: a static pod (controller kind `Node`: `Static pod: the kubelet owns it; restart the kubelet's manifest instead`), a finished pod (`The pod has finished; its controller does not restart it`), a terminating pod (`Already terminating`) | deleting a mirror pod restarts nothing; a finished pod is not restarted by Jobs or ReplicaSets |
| 12 | Allowed for any other controller kind (ReplicaSet, StatefulSet, DaemonSet, Job, custom); warnings per owner ([pod-removal.md](pod-removal.md)) and always `Restart deletes the pod without checking PodDisruptionBudgets; Evict checks them` | custom controllers recreate too; the PDB line points to the safer action |
| 13 | Restart and Evict are `Destructive`, typed **pod name** on TypeName tiers, no single key (unbound row actions; menu and palette only) | they end a running pod; W4 shows no key; Del stays the only destructive single key |

## Evict

| # | Decision | Rationale |
|---|---|---|
| 14 | Evict is refused only for a static pod (`Static pods cannot be evicted`); a bare pod and a DaemonSet pod get warnings, not refusals | the eviction API is the PDB-respecting delete; kubectl's drain filters are drain policy, not eviction policy |
| 15 | 429 on the dry-run blocks the commit and shows the cause (`refused for now: The disruption budget api-pdb needs 2 healthy pods…`); no automatic retry; the user evicts again later | the drain owns backoff (0034 decision 27); a single action should not wait silently |
| 16 | Grace is the pod default | W4 draws no grace choice; the drain keeps its grace select |

## Drain `Skip PodDisruptionBudgets` (shipped)

| # | Decision | Rationale |
|---|---|---|
| 17 | **Ship it.** W6 draws it; it is kubectl `--disable-eviction`; it is the only way a drain finishes past a pod matched by two PDBs (the API refuses it for good, HTTP 500) or a budget that can never allow (`minAvailable` = replicas, `maxUnavailable: 0`, unhealthy pods). The alternatives (edit someone's PDB and restore it, or delete each blocked pod by hand under its own typed name) are slower and easier to get wrong | small cost: one option, one request switch in `drain_writes.rs`, existing `DeleteObject` and tier |
| 18 | Strong friction: enabled only when `delete pods` is allowed; **off on every open, never remembered**; risk `Privileged` while ticked, so the name is typed in **every** tier (node name for one node, cluster name for several); a danger HEADS UP names the bypassed budgets and the pods they protect; the steps strip reads `Delete {n} pods` | the user sees exactly what loses protection and types it even on DEV |
| 19 | While ticked, every pod is deleted (kubectl semantics), not only the blocked ones | one request kind per run; "Allows" can turn into a 429 mid-run, which would stall a mixed run |
| 20 | The grace select is fixed to `Pod default` while ticked (`Deletes use each pod's own grace period`) | `DeleteObject` sends no grace (0033 decision 6); adding one changes a pinned wire format. Ceiling noted in [drain-skip-pdbs.md](drain-skip-pdbs.md) |

## Bulk labels

| # | Decision | Rationale |
|---|---|---|
| 21 | The W5 header `Edit labels`: one ticked node → the 0034 editor; 2–50 → the bulk editor; none → `Tick nodes first` | W5 draws one header button; W5 note 4 is the multi-select model |
| 22 | The bulk editor lists **changes only** (Set key=value, Remove key), not the nodes' labels, which differ per node | per-key merge patches are independent (0034 decision 6) |
| 23 | Per node, changes that are already true are dropped; a node with none left is skipped (`already labelled`) | the dialog lists only real writes; the patch would be a no-op anyway |
