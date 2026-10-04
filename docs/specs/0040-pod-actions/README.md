# 0040 — Pod actions

Status: **draft 2026-10-04**, against main `4d99faa` (758f532 + 0029 step 5); amended 2026-10-04 after the opus review (S1–S5, nits). **Mutating.** C3: covered by the one approval of 2026-10-02 for all mutating specs. Debug builds block every write and attach unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only. One cluster at a time (0046). Builds on 0030 (write path, gate, tiers, audit, `fresh_enter`), 0033 (delete flow, uid pin), 0034 (`EvictPod`, 429, drain, `SetNodeLabels`), 0036 (Shell tab, `ConnectIntent`), 0037 (`debug_shell.rs`, `AttachPermit`). Roadmap: gap audit section 1 rows W4 n1, W4b n3, W6 n2, W5 header; plan item 2. Wireframes: **W4** pod menu and note 1, **W4b** note 3, **W5** header `Edit labels` and note 4, **W6** `Skip PodDisruptionBudgets` and note 2, **W8/W8b** shell tab, keyboard map (A).

## Goal

- **Attach** (A, W4 menu, W4b container ⋯ menu) to a running container that has a terminal, in a 0036 Shell tab.
- **Restart pod** (W4: "delete & recreate"): a uid-pinned delete of a controller-owned pod.
- **Evict** (W4): the 0034 `EvictPod` of one pod, PDB-checked, 429 shown.
- **Drain `Skip PodDisruptionBudgets`** (W6): shipped, with the strongest confirm tier.
- **Edit labels of several nodes** (W5 header): a 0032 batch of 0034 `SetNodeLabels`.

## Non-goals

- New `WriteOperation`s, a new clippy exception, a new `#[allow]`, a new connect file ([decisions.md](decisions.md) 1, 3).
- Restart or Evict of several ticked pods (Restart rollout and Drain do that safely); a grace choice for Evict or Restart; Evict retry with backoff (the drain owns it).
- Attach to a container without stdin and TTY (output only: View logs covers it); attach to init or ephemeral containers.
- Bulk Edit taints; bulk labels from the row menu or the selection bar (the W5 header button only).
- A grace period for the deletes of a `Skip PodDisruptionBudgets` drain (pod default; [drain-skip-pdbs.md](drain-skip-pdbs.md)).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster: `ContainerSummary.terminal` (`ContainerTerminal`), `PodSummary.is_finished` (phase `Succeeded` or `Failed`, as `DrainPod.is_finished`), `AttachWait::Container` in `debug_shell.rs`; fixture fields in every `ContainerSummary` and `PodSummary` literal; fake tests. No app caller | 1–4 |
| 2 | App: Attach (A, pod menu, container ⋯ menu, palette), `ConnectOpen::Attach`, `ShellKind::Attach` (no Reconnect), audit, `--screen attach-confirm` | 1, 2, 5, 10–13 |
| 3 | App: Restart pod and Evict through the 0033 start (`Removal`), pod menu in W4 order, `--screen restart-pod-confirm`, `evict-confirm` | 1–3, 6, 7, 10–13 |
| 4 | App: drain `Skip PodDisruptionBudgets` (`BudgetPolicy`), `--screen drain-dialog-skip-pdbs` | 1–3, 8, 10–13 |
| 5 | App: bulk Edit labels (header button, bulk editor, `label_batch`), `--screen node-labels-bulk-editor` | 1–3, 9–13 |
| 6 | Live: UAT denied path, request trace, ui-verifier on the five screens | 14, 15 |

Steps 3, 4, 5 need nothing from steps 1–2 and may run in any order.

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale (connect file, bare pods, Skip PDBs) |
| [safety.md](safety.md) | per-action safety matrix: dry-run, tier, audit, row's cluster, held Enter, clippy ban |
| [attach.md](attach.md) | cluster wait, container terminal, gate, entries, intent, tab, audit |
| [pod-removal.md](pod-removal.md) | Restart pod and Evict: blocks, `Removal`, batch texts, warnings, notices |
| [drain-skip-pdbs.md](drain-skip-pdbs.md) | the option, verdicts, preview, confirm, run, audit |
| [bulk-labels.md](bulk-labels.md) | header button, bulk editor, `label_batch`, batch rules |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step, doc updates; tests, screens, UAT |

## Acceptance criteria

- [ ] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. 0040 adds no change to `object_write.rs`, `clippy.toml`, `Cargo.lock`, `fresh_enter.rs`, or the 0030 exception table; no new `#[allow]` or `#[expect]`.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under its name and passes offline; none talks to a cluster.
- [ ] 3. No new `WriteOperation`: Restart sends `DeleteObject { uid, Background }`, Evict `EvictPod { uid, PodDefault }`, a skip-PDB drain `DeleteObject` per pod, bulk labels `SetNodeLabels` per node; the fake transport pins each shape (0033/0034 bodies).
- [ ] 4. Attach calls only `attach_process` in `debug_shell.rs`, only with an `AttachPermit`; a `Blocked` policy sends zero requests; `AttachWait::Container` attaches after one read of a running container.
- [ ] 5. Attach: A, the pod menu, the container ⋯ menu, and the palette share one gate (`get` and `create pods/attach`, lock) and the container block; the dialog follows the cluster tier and shows the warnings of [attach.md](attach.md); the tab reads `attach · {pod}/{container}` and has no Reconnect (A again attaches anew); one audit line per start.
- [ ] 6. Restart pod is off for a pod with no controller, a static pod, a finished pod (`PodSummary.is_finished`), and a terminating pod (row status, or `deletion_started` from the uid read), with the texts of [pod-removal.md](pod-removal.md); otherwise it reads the uid first and sends one uid-pinned delete after a passed dry-run.
- [ ] 7. Evict is off for a static pod; a 429 dry-run blocks the commit with the server's cause; a 429 commit reads `refused for now: …` and writes no audit line; 404/409 read as gone.
- [ ] 8. `Skip PodDisruptionBudgets` is enabled only when the lazy `delete pods` check allows it and no dry-run runs, starts off on every open, clears and reruns every pod dry-run when toggled, types the name in every tier for `Drain` (`Cordon only` keeps its tier), and sends `DeleteObject` instead of `EvictPod`; the `Drain` summary line records `disable_eviction`.
- [ ] 9. The W5 header `Edit labels` opens the bulk editor for 2–50 ticked nodes of one cluster; kubelet keys are refused; a Remove adds the DaemonSet warning; nodes that already match are skipped; one dry-run and one audit line per node.
- [ ] 10. Every entry takes the row's own cluster (`ClusterObject.cluster`, `RowContext`, `guard_for`); after A → B, or A → B → A, nothing opens or sends ([safety.md](safety.md) tests).
- [ ] 11. A held or repeated Enter never confirms any new dialog; Enter confirms only on a fresh press.
- [ ] 12. Audit lines hold no stream bytes, request bodies, or Secret data; actions `Attach`, `Restart pod`, `Evict`, `Delete` (skip-PDB drain), `Edit labels`.
- [ ] 13. Menus, keys, and palette agree: W4 order; A is bound; Restart pod and Evict have no single key; the shortcut sheet lists A.
- [ ] 14. UAT (debug build): Attach `Not permitted: get and create pods/attach`, Restart `Not permitted: delete pods`, Evict `Not permitted: create pods/eviction`, bulk labels `Not permitted: patch nodes` (or the probe's real answers); a trace shows only GETs and SSAR POSTs, no attach upgrade, no `write finished` line.
- [ ] 15. ui-verifier: the five `--screen`s of [test-plan.md](test-plan.md) match W4, W5, W6 with no high-severity defect.

## Open items

1. R2: no write-capable cluster; commits and attach are proven by fake-transport tests only.
2. `stdinOnce` containers: closing the tab ends their process (stdin EOF). Warned, not prevented; confirm on the first allowed-path run.
3. 0030 open item 5 applies: with scope All, namespace-only rights read as denied.
