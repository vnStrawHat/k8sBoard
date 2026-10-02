# 0034 · Decisions

[Back to index](README.md). Architect defaults; items marked (user) need confirmation.

| # | Decision | Rationale |
|---|---|---|
| 1 | Eviction is `create_subresource("eviction")` with our own `policy/v1` body; `kube::Api::evict` is not used (stays disallowed) | kube 4.2 serializes `delete_options` in snake case, so grace and the uid precondition would be ignored; `Evict` is implemented only for typed `Pod` |
| 2 | Every eviction carries `deleteOptions.preconditions.uid` | StatefulSet pods reuse names; a retry must never evict the replacement (0030 decision 14) |
| 3 | 429 is a new `WriteError::TooManyRequests { message, retry_after }`, on dry-run and commit | the PDB answer is an expected wait, not a failure and never an unknown outcome |
| 4 | Taint edits are a merge patch of the full list with `metadata.resourceVersion` | `spec.taints` has no merge key; the list is replaced, so a concurrent change must be a 409, not a silent loss |
| 5 | `NodeTaint` keeps `timeAdded` and the editor sends it back unchanged | the node lifecycle controller times `NoExecute` evictions from it |
| 6 | Label edits are a per-key merge patch without `resourceVersion` | keys are independent; only changed keys are sent |
| 7 | System taints (`node.kubernetes.io/`, `node.cloudprovider.kubernetes.io/`) and kubelet labels are read-only in the editors | controllers or the kubelet put them back; cordon owns the unschedulable taint |
| 8 | Drain reads use one-shot lists (`drain_pods` by field selector, all PDBs), not the session watches | the session may be namespace-scoped; the preview must cover every namespace on the node |
| 9 | Bulk cordon reuses the 0032 bulk flow with the 0030 operation | no new write; one-cluster, ≤ 50, dialog, audit rules come for free |
| 10 | Editors open a form dialog, then the 0030 confirm dialog | W5 shows menu items only; the confirm dialog stays the one place for tier, dry-run, typed name, note |
| 11 | Adding a `NoExecute` taint is `Destructive` | it evicts pods at once |
| 12 | The editor re-reads the node on open and on Retry | summaries do not carry `resourceVersion` (it changes with every status update) |
| 13 | Bulk label editing is out of scope | W5 draws one header button and no bulk form (open item 2) |
| 14 | Taint and label values are audited | they are not secret and explain scheduling changes |
| 15 | Drain always opens its dialog; risk `Destructive` | W5 note 5; keyboard rule "destructive actions always open a confirmation" |
| 16 | Unticked options follow kubectl: affected pods block Drain (with a reason), not silently skipped | the options are kubectl flags (W6 note 2); Cordon only remains available |
| 17 | Mirror pods are always skipped; terminating pods are awaited, not evicted | kubectl skips mirror pods; evicting a terminating pod adds nothing |
| 18 | Preview from the 0013 `disruption_state()` plus a per-node rank per PDB | reuses BLOCKS DRAIN logic; explains why the second pod of a 1-allowed budget waits |
| 19 | A pod matched by two or more PDBs blocks Drain | the eviction API refuses such pods permanently (HTTP 500) |
| 20 | Server dry-run of the cordon and of every eviction before Apply, one at a time; 429 counts as passed | 0030 decision 5 per request; the dry-run eviction checks the PDB, so it confirms the preview |
| 21 | Typed name: the node for one node, the cluster for several | W6 note 4; 0030 decision 10 |
| 22 | "Skip PodDisruptionBudgets" is shown disabled (`Comes in a later version`) | it deletes pods (`--disable-eviction`), a 0033 operation |
| 23 | Grace choices `Pod default`, 10, 30, 60, 120 s; timeout 2, 5, 10, 30 min per node, default 5 min | W6 defaults; no infinite timeout, so a drain always ends |
| 24 | Cordon only commits from inside the drain dialog | the dialog already holds the typed name and a passed cordon dry-run |
| 25 | Multi-node: cordon all first, drain in order, stop at the first stuck node | cordon-all-first follows kubectl `drain a b`; the stop rule comes from W5 note 4 (kubectl itself continues to the next node and reports) |
| 26 | One request in flight per run; evictions are sent one by one, then awaited | eviction calls return at once; waiting dominates, so concurrency buys little |
| 27 | Backoff `min(30 s, max(retryAfter, 5 s·2^(n−1)))`, no jitter | honors the server hint (10 s while a PDB is still being processed; the usual "needs N healthy pods" refusal carries none); kubectl uses a fixed 5 s |
| 28 | Pod gone = its uid absent from a 3 s poll of `drain_pods(node)` | one list per tick covers every awaited pod; no per-pod watch; 3 s keeps a long drain light |
| 29 | Uid conflict or 404 on eviction = gone | the pod with that uid no longer exists |
| 30 | An unknown eviction outcome is retried | the uid precondition makes a repeat safe |
| 31 | Cancel stops new requests only; never undoes, never uncordons | an eviction cannot be reverted; uncordoning is an explicit, confirmed action |
| 32 | A lock, switch, or reconnect stops the run like Cancel | `commit_block` before every commit (0030) |
| 33 | 429 refusals are not audited (0030 decision 36) | nothing changed; retries would flood the log |
| 34 | One running drain per cluster; the tab cannot close while running | prevents two runs fighting over one node; closing must not hide a live run |
| 35 | The drain is a dock tab (`DockTab::Drain`) | W6 note 5: "progress per pod in a dock tab, cancellable" |
| 36 | No resume after restart | state lives in memory; nodes stay cordoned and safe (open item 3) |
| 37 | Finished pods are checked before DaemonSet ownership; pending pods skip the PDB row (S3) | kubectl evicts finished DaemonSet pods; the eviction API ignores PDBs for pending (and terminal) pods |
| 38 | A passing eviction dry-run downgrades a local `Blocked`/`Waits` guess to `Dry-run accepted` | the server answer is fresher than the PDB status in the list |
| 39 | One audit summary line per node at its end (action `Drain`; counts `evicted`, `refused`, `failed`, `skipped`; outcome `drained`/`stuck`/`cancelled`/`stopped`), besides the per-commit lines (S4) | the audit shows the drain as one decision with its result, without per-retry noise |
| 40 | `DrainRun.node_started` holds the current node's start; the timeout is per node (S6) | a long first node must not eat the second node's time |
| 41 | The eviction response is decoded as `Status`; a non-`Success` body is an error from its code and message (M2) | the API can answer HTTP 201 with a `Failure` body (two PDBs) |
| 42 | Taint 409 Retry reopens the editor fresh with a notice | re-applying stale rows could resurrect a removed taint |
| 43 | Step 3 splits into 3a (plan, dialog, dry-runs, Cordon only) and 3b (run, evictions, tab) (S5) | the eviction commit path is reviewed and approved on its own |
