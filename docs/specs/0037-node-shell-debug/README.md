# 0037 — Node shell and debug containers

Status: draft; **refreshed 2026-10-03 against main `2c7dc08`**. **Mutating and privileged. C3: Approved by the user on 2026-10-02 (one approval for all mutating specs).** Debug builds still block every create, patch, delete, and attach unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only; node shell still always types the node name. Prerequisites: merged 0024, 0025 (Clusters › Safety), 0027, 0028, 0030 steps 1/2a/3 (`object_write.rs` with `WriteOutcome.uid` / `created_name`, `WritePolicy`, `confirm_step`, `ActionRisk`, `audit_log.rs`); 0030 steps 2b + 4 (in flight); 0036 (terminal, `Dock`, Shell tabs, `drive`, `ws`, multi-check gate, `leaving_work`). 0035 is **not** required (`CreateThenAttach` has its own `open`); only the `run_raw` move is shared with it. Lane W2, after 0036 (and 0035). Wireframes: W2 Safety "Allow node shell" (hint "Creates a privileged debug pod on the node. Off by default for production."), W4 menu + note 2 "Debug container… · ephemeral", W5 menu "Open node shell" (S), W5 note 7 and dock tab `›_ node shell · wk-03 (debug pod)`.

## Goal

- **Debug container…**: add an ephemeral container (`pods/ephemeralcontainers`, strategic merge patch) to a running pod, sharing the target container's process namespace, and attach a Shell tab to it. The answer for distroless pods (0036 "No shell in this container").
- **Open node shell**: create a privileged pod on the node (`hostPID`, `privileged`, `nsenter -t 1` into the host), attach a Shell tab, and delete the pod when the session ends, the tab closes, the cluster switches, or the main window closes (best effort at other quits).
- Strongest confirmation for node shell (always type the node name), an image choice, a per-cluster **Allow node shell** setting (off for PROD), orphan guards, and exact pod specs and RBAC.
- Every create, patch, and delete goes through 0030 `ClusterConnection::write` (dry-run first, audited); the attach requires an `AttachPermit`.

## Non-goals

Removing an ephemeral container (the API cannot); kubectl debug profiles (`general`, `netadmin`, …), pod copies (`--copy-to`), custom commands; Windows nodes; Attach to existing containers (W4 "Attach", a later item); a node shell image allow-list; automatic leftover sweeps without the user.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster crate: `AccessCheck`s, `AttachPermit`, `WriteOperation::{AddDebugContainer, CreateNodeShellPod, DeleteNodeShellPod}`, `WriteOutcome.uid`, `debug_shell.rs` (wait + attach + 0036 `drive`); fake tests. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1–5 |
| 2 | App: `ActionRisk::Privileged`, settings keys and the W2 toggle, gate rows, `GuardedKind::CreateThenAttach`, cleanup path, Debug container… end to end; UAT denied path. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 6–9, 12 |
| 3 | Node shell end to end: options dialog, typed node name, tab, cleanup on every end, quit hook, leftover sweep; `--screen node-shell-confirm`. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 6–13 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [pod-specs.md](pod-specs.md) | exact ephemeral container patch, node shell pod, delete; RBAC; admission |
| [session-flow.md](session-flow.md) | cluster API, guarded create-then-attach, wait, attach, cleanup, orphans, lifecycle |
| [ui.md](ui.md) | entry points, options dialogs, confirm, settings, Shell tab variants |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step; tests and checks |

## Acceptance criteria

- [ ] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. `Cargo.lock` unchanged.
- [ ] 2. Every test in [test-plan.md](test-plan.md) exists under its name and passes offline; none talks to a cluster.
- [ ] 3. The fake transport pins method, path, query (`dryRun=All` then none, `fieldManager=k8sboard`), content type, and body of the three new operations, byte for byte against [pod-specs.md](pod-specs.md).
- [ ] 4. `attach` is called only in `debug_shell.rs` and only with an `AttachPermit` (only non-test constructor: `AccessReport::attach_permit`, both attach verbs allowed); upgrade refusals 401/403/404 map to typed errors.
- [ ] 5. Debug builds without `K8SBOARD_ALLOW_WRITES=1` send zero requests for both actions.
- [ ] 6. Node shell always asks for the typed **node name**, whatever the cluster tier; Debug container follows the cluster tier after its options dialog.
- [ ] 7. "Allow node shell" defaults on only for DEV/LOCAL and explicitly set STG (off for PROD, guessed STG, unknown); off → `Node shell is off for {cluster}`; Windows nodes → `Node shell needs a Linux node`.
- [ ] 8. The attach permit is taken **before** the commit: no permit → nothing is created.
- [ ] 9. Each create, patch, and delete appends one audit line; no line holds session bytes, a pod body, or an env value.
- [ ] 10. A node shell pod is deleted (uid precondition, commit only) when its shell exits, its tab closes, the cluster switches, the start fails after creation, and before the main window closes (the close waits). Other quit paths: best effort within GPUI's 200 ms; `stdinOnce` exit plus the sweep cover the rest.
- [ ] 11. The node shell pod carries `activeDeadlineSeconds: 14400`, `stdinOnce: true`, and the k8sBoard labels including `k8sboard.io/instance`; at session start a read-only list reports leftovers of other instances (any phase) in a notice; nothing is deleted without the user's click.
- [ ] 12. On UAT the SSAR answers of every new check are recorded; both menu items show their `Not permitted: …` reason; **no create, patch, delete, or attach request is sent** (trace).
- [ ] 13. ui-verifier: `--screen node-shell-confirm` and the W2 Safety toggle match the wireframes with no high-severity defect.

## Open items

1. R2: no write-capable cluster; a privileged pod must never be tried on UAT. The allowed path waits for a disposable cluster.
2. `stdinOnce` ending the shell (and its children) when the attach closes is an assumption to confirm in the first allowed-path run (busybox `nsenter` is verified present).
3. Pod Security Admission `baseline`/`restricted` namespaces reject the node shell pod (the default `kube-system` is usually exempt); the dry-run shows the reason and the user picks another namespace. No automatic choice.
4. A pod with `runAsNonRoot` rejects the root busybox debug container at start; profiles (kubectl `--profile`) are out of scope.
5. Session-start SSARs grow by five checks (C13 budget); measure with 0036's.
6. The default image digest (`busybox:1.36.1@sha256:<DIGEST>`) could not be verified offline; the coder fills it from the registry in step 1 and records the lookup.
7. A debug container whose attach never happened (crash between patch and attach) keeps `sh` waiting until the pod is deleted; the API cannot remove it.
9. (0030 dependency) 0030 AC 9 says `ClusterConnection::write` is called only from `checked_write`. `run_cleanup` (commit-only `DeleteNodeShellPod`, no gate: decision 13) is a second caller in `write_flow.rs`, because `checked_write` re-resolves `guard_for` and would refuse after a slot release. The 0030 AC needs this named exception when 0037 lands.
