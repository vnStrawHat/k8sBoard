# 0039 — Drawer and menu polish (read-only items)

Status: draft, 2026-10-03, against main `a50264c`. **Read-only: no new mutating calls.** New cluster calls are one GET helper (`pod_template_yaml`) and one list/watch (`watch_limit_ranges`) plus its access check (`ListLimitRanges`: one more SSAR at connect per namespace of the scope, as for every namespaced check); the 0030 `WriteOperation` allow-list, the connect files, and `disallowed-methods` are unchanged. Crates: `crates/app`, `crates/cluster` (steps 3, 4). Audit gap 9 (rows W4 n2, W4b n3, W7 Deployments, CronJobs, ConfigMaps, ResourceQuotas, Namespaces). Prerequisites: 0012, 0019, 0028, 0031, 0032, 0036 (all merged). Single cluster: the app shows one cluster at a time; every item reads the session of the cluster in view.

## Goal

- **View logs ▸ container submenu** (W4 n2): `name · MAIN/SIDECAR/INIT` entries, direct item for one container.
- **Container ⋯ menu** (W4b n3): View logs, Open shell, Copy image for the shown container.
- **CronJob View logs of last job** (W7 CronJobs, key L); L also opens workload logs on Deployments, StatefulSets, DaemonSets, ReplicaSets, and Jobs (W7 shows `View logs (all pods)` with L).
- **Deployment revision diff** (W7 Deployments "history with diff"): a `Diff` button per revision opens a read-only diff of its pod template against the current one.
- **Namespace LimitRange row** (W7 Namespaces, Quota section `LimitRange`).
- **ConfigMap restart hint** (W7 ConfigMaps note): Used by says which workloads need a restart to read changed env values.
- **ResourceQuotas**: drop the stale disabled `Edit` (Edit YAML already covers it).

## Non-goals

- Attach (A), Restart pod, Evict (audit 0040, mutating); Secret value editing; ConfigMap "Compare with previous" (audit 0041, needs snapshots).
- Namespaces "Show remaining resources" and PDBs "Show selected pods" menu items (audit row 1 lists them; not in this task's list, left to a later pass).
- A restart button in the ConfigMap hint (writes; the workload's own Restart rollout, key R, stays the way).
- Logs of manually triggered jobs in "last job" (decision 6).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Quota `Edit` removal; ConfigMap `restart_hint`; `LogsMenu` (View logs submenu); container ⋯ menu | 1–5, 10, 11 |
| 2 | `last_job_owner`, CronJob `View logs of last job`, L on workload kinds through `subject_action` / `key_availability` / `selected_log_target` | 1–3, 6 |
| 3 | `ClusterConnection::pod_template_yaml`; `RevisionDiffView` dialog; `Diff` buttons in Revisions; `diff_row_element` shared with Edit YAML; `--screen revision-diff` | 1–3, 7, 8 |
| 4 | `limit_range.rs`, `AccessCheck::ListLimitRanges`, Namespace related stream with limit ranges, LimitRange rows; ui-verifier for all steps | 1–3, 9, 12 |

## Files

| File | Contents |
|---|---|
| [logs-and-menus.md](logs-and-menus.md) | View logs submenu, container ⋯ menu, CronJob last job, L on workload kinds |
| [revision-diff.md](revision-diff.md) | template GET, masking, dialog view, async, buttons |
| [namespace-and-config.md](namespace-and-config.md) | LimitRange (cluster and app), ConfigMap restart hint, ResourceQuotas `Edit` |
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [files-to-touch.md](files-to-touch.md) | files per step, docs |
| [test-plan.md](test-plan.md) | unit, fake transport, headless, live, ui-verifier |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`; `Cargo.lock` gains no package.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [ ] 3. Read-only: no new `WriteOperation`, connect file, or `disallowed-methods` exception; new requests are one GET per diff side, one limitranges list/watch while a Namespace drawer is open, and one `ListLimitRanges` SSAR per namespace of the scope at connect.
- [ ] 4. (W4 n2) A pod with several containers shows `View logs ▸` with one entry per container (`name · MAIN|SIDECAR|INIT`), each opening a dock tab on that container; one container opens directly; the reason shows when logs are not permitted.
- [ ] 5. (W4b n3) The container detail header has a ⋯ menu: View logs (this container), Open shell (this container, through the existing guarded shell flow; disabled with the reason when not running or not permitted), Copy image.
- [ ] 6. (W7 CronJobs, keyboard L) `View logs of last job` (hint L) opens the workload logs of the Job named after `last_schedule_at`; disabled with `No job has run yet` / `Job {name} has no pods left`; L does the same on the cursor row, and L on a Deployment, StatefulSet, DaemonSet, ReplicaSet, or Job opens its workload logs.
- [ ] 7. (W7 Deployments) Each non-current revision row has `Diff`; it opens a dialog `rev {old} → rev {new}` with the line diff of the two pod templates (the `pod-template-hash` label left out); equal texts say `The pod templates of the two revisions are the same.`, or, when env values are hidden, `No visible difference; env values are hidden` next to the toggle.
- [ ] 8. (W7 Deployments, W10 masking rules) The diff masks like the YAML tab: env literals hidden by default with a `Show env values` toggle, secret-looking annotations hidden, also under `spec.template.metadata`; nothing is logged.
- [ ] 9. (W7 Namespaces) The Quota section lists each LimitRange (`{type}: default …, request …, max …, min …`) or `No LimitRange`; quotas and LimitRanges are gated each by its own check (`ListResourceQuotas`, `ListLimitRanges`): a denied one reads `Not permitted: list …` and starts no watch, the allowed one still lists (namespace-and-config.md table).
- [ ] 10. (W7 ConfigMaps) Used by ends with the restart hint naming the env readers (not CronJobs, Jobs, or bare `pod/{name}` owners) when any reads the ConfigMap through env; none otherwise.
- [ ] 11. (W7 ResourceQuotas) The menu has `Edit YAML` and no disabled `Edit`.
- [ ] 12. ui-verifier: `pod-containers`, `cronjobs-menu`, `resourcequotas-menu`, `configmaps-drawer`, `deployments-drawer`, `revision-diff`, `namespaces-drawer` (light, dark) have no high-severity defect against W4, W4b, W7.

## Open items

1. The same restart hint fits Secrets read through env; not asked, one call in `secret_used_by_rows` when wanted.
2. "Last job" ignores manual `Trigger now` jobs (decision 6); the CronJob drawer's Recent jobs list reveals any Job, whose own menu has View logs.
