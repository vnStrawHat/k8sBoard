# 0041 — Edit YAML part 2: revision history, quota check, timeline

Status: draft, 2026-10-04, against main `4d99faa`. **Read-only: no new `WriteOperation`, connect file, or `disallowed-methods` exception.** New requests: one ReplicaSet LIST (namespace, filtered to one Deployment) per history open or timeline click, and the two 0039 template GETs per diff. No new SSAR (`ListReplicaSets`, `ListResourceQuotas` are in `AccessCheck::ALL`). Crates: `crates/cluster`, `crates/app`. One cluster at a time (0046). Wireframes: **W10** (tabs, side panel Checks), **W3 n4** (Recent changes "who", click opens diff). Roadmap: gap audit rows W10, W10 n2, W3 n4; 0031 non-goals; 0021 open item 1.

## Goal

- **Revision history tab** in Edit YAML for Deployments (W10 tabs): the Deployment's revisions, newest first; a past revision opens a read-only diff of its pod template against the current one (the 0039 `RevisionDiffView`, embedded).
- **Quota check** in the Edit YAML Checks (W10 n2, "Namespace quota OK (22Gi left)"): the namespace ResourceQuota headroom against the steady-state change in pods, CPU, and memory. Advisory; never blocks Apply.
- **Overview timeline**: a "who" for Deployment rollouts from `managedFields` (manager and time), and a click on a rollout row opens the diff of the latest revision against the previous one.

## Non-goals

- **Pre-apply snapshot and one-step rollback** (W10 n5, footer "Rev 38 snapshot will be kept"): dropped by the user on 2026-10-03.
- **ConfigMap "Compare with previous"** (W7 ConfigMaps): dropped with snapshots (user, 2026-10-03); Kubernetes keeps no ConfigMap history.
- **Timeline ConfigMap rows** ("changed 2 keys · an.nguyen", W3): counting changed keys needs the previous data (stored history), and a cluster-wide ConfigMap watch. Dropped for the same reason.
- History for StatefulSets and DaemonSets (W10 draws a Deployment only; they need a ControllerRevision kind). Roll back from the tab (0032 Roll back stays in the drawer). Copying a past template into the editor.
- W10 `Hide managedFields` toggle: the edit body strips them (0031 decision 12), so showing them would invite edits that are discarded.
- Quota of surge pods during a rolling update, scoped quotas, LimitRange defaults, `spec.overhead` (decision 4).
- A user identity: Kubernetes stores none on objects; "who" is the field manager name (decision 6).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster: `deployment_revisions`, `DeploymentSummary.template_change` (managedFields), `quota_demand.rs` (`WorkloadDemand`, `DemandChange`, `quota_check`), `EditPreview.demand`. No app caller | 1–3, 10, 11 |
| 2 | App: `EditTab::History`, `revision_history.rs` (list, selection, embedded `RevisionDiffView`), `--screen edit-yaml-history` | 1–5, 12 |
| 3 | App: quota line in the side panel and dialog warning, read from the session's ResourceQuotas condition feed; `--screen edit-yaml-diff` shows it | 1–3, 6, 7, 12 |
| 4 | App: timeline "who" and click-to-diff (`open_latest_revision_diff`, `Go to deployment` in the dialog); UAT trace; ui-verifier for all steps | 1–3, 8–10, 12 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [revision-history.md](revision-history.md) | cluster LIST helper, the History tab, async |
| [quota-check.md](quota-check.md) | demand model, headroom rule, texts |
| [overview-timeline.md](overview-timeline.md) | managedFields "who", click-to-diff |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step; tests and checks |

## Acceptance criteria

- [ ] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`; `Cargo.lock` unchanged.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [ ] 3. Read-only: no new `WriteOperation`, connect file, or clippy exception; the only new request is the ReplicaSet LIST of `deployment_revisions`; no new `AccessCheck`.
- [ ] 4. (W10) Edit YAML on a Deployment shows `Editor`, `Diff vs cluster`, `Revision history`; other kinds show two tabs. The tab lists `rev {n} · {tag}`, age, and `current`, newest first; the previous revision is selected on open.
- [ ] 5. Selecting a past revision shows its pod-template diff against the current one with the 0039 masking and env toggle; the current row reads `This is the current revision.`; a denied list reads `Not permitted: list replicasets`; one revision reads `No earlier revision kept (revisionHistoryLimit)`. The tab never changes the editor text.
- [ ] 6. (W10 n2) After a passed dry-run of a Deployment, StatefulSet, or DaemonSet change that adds pods, CPU, or memory, Checks shows `Namespace quota OK ({left} {resource} left)` or, per exceeded item, `Quota {name}: {resource} needs {needed} more, {left} left` (warning tone, also a confirm-dialog warning). No line when nothing grows or the namespace has no unscoped quota.
- [ ] 7. With the ResourceQuotas feed off or loading, Checks reads `Quota not checked: {reason}`; Apply is never blocked by the quota check.
- [ ] 8. (W3 n4) A Deployment rollout row shows the field manager of the latest template change when it falls in the rule window (decision 7); otherwise the event source, as today. HPA, node, and namespace rows are unchanged.
- [ ] 9. Clicking a Deployment row opens the revision diff of the newest revision against the one before; the dialog has `Go to deployment` (reveals the row, closes the dialog). Other rows keep reveal. A failed list pushes `Could not load revisions: {error}`.
- [ ] 10. Nothing logs a template, a manager name, or a quota value; `WorkloadDemand` and `DemandChange` derive `Debug` over numbers only.
- [ ] 11. `quota_check` and `workload_demand` follow [quota-check.md](quota-check.md) on every fixture (replicas default 1, init and sidecar rule, DaemonSet node count, unparsable quantity skipped).
- [ ] 12. ui-verifier: `edit-yaml-history`, `edit-yaml-diff` (with the quota line), `overview` (who column), `revision-diff` (Go to deployment), light and dark, no high-severity defect against W10 and W3.

## Open items

1. R2 is not touched (read-only). UAT has Deployments with ReplicaSets, so step 4's live check covers the LIST; quota fixtures only (UAT has no ResourceQuotas).
2. 0021 open item 1 closes with step 4 (pointer added by the coder); ConfigMap rows stay out (non-goal).
