# 0034 · Drain run, retries, cancel, progress tab

[Back to index](README.md) · Step 3b · Modules: `drain_run.rs` (new: pure state machine + driver) + `drain_run_tests.rs`, `drain_tab.rs` (new), `dock.rs` (0036 step 3a), `app_shell.rs` / `app_shell_view.rs` (`leaving_work` line), `audit_log.rs`. Decisions 25–36, 39–41, 45. Wireframe: W6 note 5, W5 note 4.

## Phases

| Phase | What happens |
|---|---|
| 1 Cordon all | every selected node not yet cordoned gets `SetNodeSchedulable { false }`, in order (kubectl `drain a b` cordons all first, so evicted pods do not land on the next node). A failure stops the run: `Could not cordon {node}: {error}`; nodes cordoned before stay cordoned |
| 2 Per node, in order | evict and wait (below) until the node is **Drained**, **Stuck**, or the run is **Cancelled** / **Stopped** |
| 3 End | first Stuck node stops the run (W5 note 4); later nodes stay cordoned and untouched |

The plan is re-read when a node starts (`drain_pods(node)`): pods that arrived since the dialog are classified with the same options; a new pod gets one eviction dry-run before its first commit (0030 decision 5). Pods that now need an option the user did not tick become `Failed("Not evicted: needs {option}")` and make the node Stuck when everything else is done.

## State machine (pure; `now` = time since the run started)

```rust
pub(crate) enum PodProgress { Pending, Refused { attempt: u32, retry_at: Duration, message: SharedString },
    Evicted /* accepted; waiting for deletion */, Gone, Failed(SharedString), Awaited /* Terminating verdict */, Skipped(SkipReason) }
pub(crate) enum NextStep { Cordon(String), Evict(PodKey), DryRun(PodKey), Poll, Sleep(Duration), NodeDone(NodeOutcome), Finished }
pub(crate) enum NodeOutcome { Drained, Stuck { reason: SharedString } }
pub(crate) struct DrainRun { nodes: Vec<NodeRun>, current: usize, /** run-relative start of the current node; the per-node timeout counts from it */ node_started: Duration,
    last_poll: Option<Duration>, end: Option<RunEnd> /* Cancelled, Stopped(text), Finished */, confirmed: Confirmed, generation: u64 }
impl DrainRun {
    pub(crate) fn next_step(&self, now: Duration) -> NextStep;
    pub(crate) fn on_write(&mut self, sent: &NextStep, result: Result<WriteOutcome, CheckedWriteError>, now: Duration);
    pub(crate) fn on_poll(&mut self, present: &[DrainPod], now: Duration);
    pub(crate) fn cancel(&mut self);
}
pub(crate) fn retry_delay(attempt: u32, retry_after: Option<Duration>) -> Duration;
```

`next_step`, first match wins:

1. Cancelled or stopped → `Finished`.
2. Phase 1 not done → `Cordon(next uncordoned node)` (result through `on_write`; error → Stopped).
3. `now − node_started ≥ timeout` → `NodeDone(Stuck { "Timed out after {timeout}: {n} pods left" })`. `DrainRun.node_started` is set when the current node enters phase 2 (after phase 1 and the re-read), so each node gets its full timeout.
4. A `Pending` pod with no dry-run yet → `DryRun`; else the first `Pending`, or `Refused` with `retry_at ≤ now` → `Evict`.
5. Any `Evicted`/`Awaited` pod and the last poll ≥ 3 s ago → `Poll`.
6. All pods `Gone`/`Skipped` → `NodeDone(Drained)`; only `Failed`/`Gone`/`Skipped` left → `NodeDone(Stuck { first failure })`.
7. Otherwise `Sleep(min(next retry_at, next poll) − now)`.

## Eviction results (`on_write`)

| Result | Pod becomes |
|---|---|
| `Ok` (201/200) | `Evicted` |
| `TooManyRequests { retry_after, message }` (PDB, or API priority and fairness) | `Refused { attempt + 1, retry_at = now + retry_delay(attempt + 1, retry_after), message }` |
| `NotFound`, `Conflict` (uid precondition: a new pod took the name) | `Gone` |
| `OutcomeUnknown` | `Refused { message: "No answer; retrying" }` with the same delay rule: a repeat eviction carries the uid precondition, so it is safe; the poll marks the pod `Gone` if the first one landed |
| `Denied`, `Invalid`, `DryRunRejected`, `Cluster(..)` (incl. "more than one PodDisruptionBudget") | `Failed(text)`; other pods continue |
| `CheckedWriteError::Blocked(text)` (lock, session switch, reconnect) | run **Stopped**: `{text}; drain stopped` |

A `DryRun` step: success or 429 → the dry-run is recorded and the pod stays `Pending`; any other error → `Failed`. A `Cordon` step: success → next node; any error → Stopped.

`retry_delay(n, after) = min(30 s, max(after.unwrap_or(0), 5 s × 2^(n−1)))`: 5, 10, 20, 30, 30 … s; the usual PDB refusal ("needs N healthy pods") carries no hint, so the backoff alone applies; a PDB still being processed carries `retryAfterSeconds` 10, which raises a short delay to 10 s. No jitter: one client, one request at a time. Retries continue until the node timeout.

`on_poll`: an `Evicted`/`Awaited` pod whose uid is no longer present → `Gone`.

## What the drain leaves cordoned (L4, L12)

Every end line says which nodes stay cordoned: those this run cordoned and those that were cordoned before it (`· cordoned: wk-04 (was cordoned before this drain)`), also after a clean `Drained`. The `Uncordon` button offers exactly those nodes the cluster still reports cordoned: the tab watches the session's node list, so after an Uncordon from the tab or the table the header ends `· uncordoned` and the button goes.

## Cancel (exact semantics)

- `Cancel` sets the flag; the driver checks it before every request, so **no new request** (eviction, dry-run, cordon, poll) is sent after the click.
- A request already sent is not aborted (0030 decision 16); its result is applied and audited, then the run ends.
- Evicted pods keep terminating; an eviction cannot be undone. Their controllers recreate them elsewhere.
- Every node cordoned in phase 1 **stays cordoned**; nothing is uncordoned automatically.
- Pending and refused pods stay on their node; later nodes are not drained.
- The tab shows `Cancelled · {k} of {n} evicted on wk-04 · cordoned: wk-04, wk-05` and an `Uncordon {m} nodes` button (a 0032 `Batch` with its own dialog).
- Lock or session change mid-run behaves like Cancel, with the reason in the title (the next `checked_write` returns `Blocked`). A switch or slot release asks first through `leaving_work`, then stops the run and closes the tab (decision 45). App exit ends the run the same way (open item 3), with the summary line `abandoned`.

## Replacements after the end (L1)

An evicted pod that vanishes is not necessarily placed again: a StatefulSet pod whose volume lives on the node, or one that no other node can take, comes back Pending. When the run ends `Finished` and evicted a pod that has a controller, the driver keeps looking (`start_follow`, `next_follow`, `on_follow` in `DrainRun`; the tab is already closable and the end notice already shown):

- every poll interval (3 s) it lists the cluster's Pending pods (`pending_pods()`, one field-selector list), for at most 60 s, or until 15 s pass with no replacement Pending; a replacement still Pending then keeps being looked for every 10 s, for up to 30 min or until the tab closes, so the line and the rows follow an Uncordon done later;
- a Pending pod of the same namespace and controller (kind and name) whose uid was not on the node is the replacement of one evicted pod (one each); that pod's row reads `recreated · Pending: {reason}` (warn), the reason being the message of the replacement's `PodScheduled=False` condition (the `FailedScheduling` event text) in one line (`0/3 nodes are available: 1 node(s) had untolerated taint {…}, 2 node(s) didn't match Pod's node affinity/selector. preemption: …` reads `0/3 nodes: 1 taint, 2 selector`; `scheduler_summary`), or `not scheduled yet`; when the replacement stops being Pending the row reads `Gone` again;
- the header reads `Drained · checking replacements` while it looks and `Drained · {n} pending replacement(s)` after (on a stuck header the count comes before `· cordoned: …`); the dot is warn; `Uncordon` stays offered (the node is still cordoned);
- a listing failure shows as `Could not refresh pods: …` and the next poll retries; the window ends with a notification `Drain: {n} evicted pod(s) have replacement(s) that stay Pending` when any remain (and the audit follow-up line below).

## Audit

- Every cordon and accepted or failed eviction commit writes its own line through `checked_write` (0030), with the dialog note.
- **Not audited** (0030 decision 36): a commit refused with 429 (each retry would add a line; nothing changed), a `Blocked` result, dry-runs, polls.
- **One summary line per node** when the node ends (S4, decision 39): action `Drain`, object the Node, fields `evicted` (`deleted` under Skip PodDisruptionBudgets, which deletes directly; L18), `refused`, `failed`, `skipped` (counts as values), outcome `drained`, `stuck`, `cancelled`, `stopped`, or `abandoned` (the app quit: the line of the node the run was on, even before its pods were read; the quit hook is registered when the shell opens, not by the first node shell), the dialog note. Written by the driver with the merged `append_audit` (an `AuditEntry` built by a `drain_summary_entry` helper in `audit_log.rs`; `AuditOutcome` gains `Drained`, `Stuck`, `Cancelled`, `Stopped` beside the merged `Applied`, `Failed`, `Unknown`), after the last commit of that node. Nodes never reached write no line.
- **Follow-up line per node with replacements** (round 3, Q1): action `Drain`, the Node, field `pending_replacements`. When the 60 s window ends with replacements still Pending, one line per node says `pending_replacements = n` with outcome `pending`; when the last of them starts (looked for every 10 s for up to 30 min, e.g. after the Uncordon), one line says `pending_replacements = 0`, outcome `drained`. Each comes with a notification (`Drain: 1 evicted pod has a replacement that stays Pending`, `Drain: the replacement pods are running`). The `drained` summary line itself is written when the node ends, before anyone can know.

## Driver (async)

```rust
// In `cx.spawn` on AppShell, holding a WeakEntity<DrainTab>; one request in flight at a time.
loop {
    let step = tab.read_with(cx, |tab, _| tab.run.next_step(tab.started.elapsed()))?;
    match step {
        Cordon(_) | Evict(_) | DryRun(_) => { let result = checked_write(&shell, write_step(&step), cx).await; tab.update(.. on_write ..) }
        Poll => { let pods = runtime.spawn(connection.drain_pods(node)).await; tab.update(.. on_poll ..) }   // read only
        Sleep(d) => cx.background_executor().timer(d.min(Duration::from_secs(3))).await,
        NodeDone(outcome) => tab.update(.. advance ..),
        Finished => break,
    }
}
```

- `write_step(&step)` builds 0030 `WriteStep { intent, generation, mode, note }`: `intent` = `GuardedIntent { cluster, action: Drain (eviction) or Cordon, label, risk, kind: Write(request), .. }`; `mode` = `CommitMode::DryRun` for `DryRun(_)`, else `CommitMode::Commit { confirmed: run.confirmed }` (`Confirmed` is `Copy`).
- Each `update` calls `cx.notify()` once; at most one request per step and a 3 s poll, so no extra batching.
- `generation` and the confirm state captured at the dialog go into every `WriteStep`; a reconnect stops the run.
- A poll failure is shown on the tab (`Could not refresh pods: {error}`) and retried at the next poll; it does not stop the run.

## Progress tab (dock)

- `DockTab::Drain(Entity<DrainTab>)` (0036 `Dock`), with `cluster()` so the merged `close_tabs_of(cluster)` closes it when its slot is released (decision 45). Title `Drain wk-04` / `Drain 3 nodes`; one running drain per cluster.
- Header: node list with state (`Waiting`, `Cordoning`, `Evicting 12/23`, `Drained`, `Stuck`, `Cancelled`); kit `Progress` (gone / to evict) for the current node; `Timeout in 3:12`.
- Rows per pod: `{ns}/{name}` mono + state text: `Waiting`, `Evicting…`, `Refused by PDB: {message} · retry in 8 s (attempt 3)`, `Terminating`, `Gone`, `recreated · Pending: {reason}`, `Failed: {error}`, `Skipped: {reason}`. Order: failed and refused or blocked rows, then recreated rows (a pod that stays Pending) and the pods still being worked on, then `Gone`, and the pods the drain leaves alone (`Skipped`) last; list order within each group. A stuck header with budget links flows as one line that breaks between words (`Stuck on wk-04: … · blocked by api-pdb · cordoned: wk-04`).
- After the end (any end) open pods stop reading as live work: `Waiting` → `Not evicted`, `Evicting…` → `Eviction sent, still on node` (`Delete sent, still on node` under Skip PDBs), a refusal → `Blocked by PDB {name}` (the name read from the API message). A stuck header appends the blocking budgets: `Stuck on wk-04: Timed out after 20 min: 11 pods left · blocked by api-pdb`; each name is a link that reveals that PodDisruptionBudget in the pod's namespace.
- Buttons: `Cancel` (danger outline) while running; after a stuck, failed, or stopped end `Drain again…` (reopens the Drain dialog on the nodes left undrained, with the timeout, grace period, and toggles of the drain it repeats; Skip PodDisruptionBudgets is never carried over), then `Uncordon {m} node(s)` and `Close`. The tab's ✕ and Ctrl W are disabled while running (`Cancel the drain first`).
- Ends with a notification: `Drain: wk-04 drained`, `Drain stopped: wk-05 stuck ({reason})`, or `Drain cancelled`.

## As built (2026-10-07): counts in semibold

The drain dialog's step strip (`Evict 23 pods · 1 cannot move`), option hints, preview header and dry-run line, and the drain tab's status line (`Stuck on wk-04: Timed out after 20 min: 11 pods left`) and `12 of 23 gone` counter render their counts in semibold through `counted_text` (see 0040 pod-removal).
