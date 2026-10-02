# 0032 · Write operations (cluster crate)

[Back to index](README.md) · Step 1 · Modules: `object_write.rs` (+ `object_write_tests.rs`), `access_review.rs`, `lib.rs`, root `Cargo.toml`, `clippy.toml`. Decisions 1–9, 27–29. Contract: 0030 write-path.md as amended (`WriteOutcome { effect, created_name }`, `WriteEffect`, no SSA).

## Verified APIs (kube 4.2, `.cargo-home/registry`)

| API | Where | Use |
|---|---|---|
| `Api::patch_scale(name, &PatchParams, &Patch)` → `Scale` | `kube-client-4.2.0/src/api/subresource.rs:38` | Scale (already in the 0030 clippy list) |
| `Api::create(&PostParams, &data)`; `PostParams { dry_run, field_manager }` | `core_methods.rs`; `kube-core-4.2.0/src/params.rs:535` | Trigger now, Re-run |
| `Patch::Json(json_patch::Patch)`, feature `jsonpatch` | `kube-core-4.2.0/src/params.rs:605–634`; `kube-4.2.0/Cargo.toml:98` | Roll back |
| `Api::restart` writes `kube.kubernetes.io/restartedAt` with sub-second time | `kube-core-4.2.0/src/util.rs:21–37` | **not used**: wrong annotation for kubectl parity; added to clippy |
| `Api::get` (typed `Deployment`, `ReplicaSet`, `CronJob`, `Job`) | `core_methods.rs` | reads inside the operation (not disallowed) |

## New operations

```rust
pub enum WriteOperation {                      // 0030 variant kept; Debug stays manual (name only)
    SetNodeSchedulable { schedulable: bool },
    ScaleWorkload { replicas: u32 },                          // Deployment, StatefulSet
    RestartRollout { restarted_at: jiff::Timestamp },         // Deployment, StatefulSet, DaemonSet
    SetRolloutPaused { paused: bool },                        // Deployment
    RollBackDeployment { replica_set: String, revision: u64 },// Deployment
    SetCronJobSuspended { suspended: bool },                  // CronJob
    TriggerCronJob,                                           // CronJob
    RerunJob,                                                 // Job
}
pub enum WriteEffect { Patched, Created /* new: Trigger now, Re-run */ /* 0031, 0033 variants */ }
```

`WriteRequest::new` returns `None` for any other kind. `restarted_at` is rounded to whole seconds when the intent is built, so the dry-run and the commit send the same body. Patches return `effect: Patched`; the two creates return `Created` with `created_name` = the response `metadata.name`.

## Wire format (allow-list rows)

Paths are namespaced: `/apis/{group}/v1/namespaces/{ns}/{resource}/{name}`. Every request carries `fieldManager=k8sboard`; a dry-run adds `dryRun=All`. No `resourceVersion` (0030 decision 14).

| Operation | HTTP | Path suffix | Content type | Body |
|---|---|---|---|---|
| `ScaleWorkload` | PATCH | `apps/v1 … deployments\|statefulsets/{name}/scale` | `application/merge-patch+json` | `{"spec":{"replicas":5}}` |
| `RestartRollout` | PATCH | `apps/v1 … deployments\|statefulsets\|daemonsets/{name}` | `application/merge-patch+json` | `{"spec":{"template":{"metadata":{"annotations":{"kubectl.kubernetes.io/restartedAt":"2026-10-02T09:12:03Z"}}}}}` |
| `SetRolloutPaused` | PATCH | `apps/v1 … deployments/{name}` | `application/merge-patch+json` | `{"spec":{"paused":true}}` / `false` |
| `RollBackDeployment` | GET ×2, PATCH | `apps/v1 … deployments/{name}` | `application/json-patch+json` | see Roll back |
| `SetCronJobSuspended` | PATCH | `batch/v1 … cronjobs/{name}` | `application/merge-patch+json` | `{"spec":{"suspend":true}}` / `false` |
| `TriggerCronJob` | GET, POST | `batch/v1 … jobs` | `application/json` | see Trigger now |
| `RerunJob` | GET, POST | `batch/v1 … jobs` | `application/json` | see Re-run |

## RBAC, dry-run, risk, audit

| Operation | `AccessCheck` (new) | SSAR | Dry-run | Risk | `changed_fields()` (path = value) |
|---|---|---|---|---|---|
| Scale Deployment | `PatchDeploymentScale` | `patch apps deployments/scale` | yes | Change; **Destructive at 0** | `spec.replicas` = `"5"` |
| Scale StatefulSet | `PatchStatefulSetScale` | `patch apps statefulsets/scale` | yes | same | same |
| Restart | `PatchDeployments` / `PatchStatefulSets` / `PatchDaemonSets` | `patch apps {resource}` | yes | Change | `spec.template.metadata.annotations[kubectl.kubernetes.io/restartedAt]` = timestamp |
| Pause / Resume | `PatchDeployments` | `patch apps deployments` | yes | Change | `spec.paused` = `"true"`/`"false"` |
| Roll back | `PatchDeployments` | `patch apps deployments` | yes | Change | `spec.template` = `"rev 37 (api-6c1e2a)"` |
| Suspend / Resume | `PatchCronJobs` | `patch batch cronjobs` | yes | Change | `spec.suspend` = `"true"`/`"false"` |
| Trigger now | `CreateJobs` | `create batch jobs` | yes | Change | `metadata.generateName` = `"reconcile-manual-"`; audit adds `metadata.name` (0030 amendment) |
| Re-run | `CreateJobs` | `create batch jobs` | yes | Change | `metadata.generateName` = `"etl-nightly-29312400-rerun-"`; same |

All checks are namespaced. `ALL` grows by 7. Display: `patch deployments/scale`, `create jobs`.

## Roll back (kubectl `rollout undo --to-revision` parity)

1. GET the Deployment and the ReplicaSet `replica_set` (typed).
2. The ReplicaSet's controller owner reference must carry the Deployment's `uid`, and its `deployment.kubernetes.io/revision` must equal `revision`; otherwise `NotFound` and **no PATCH**.
3. Template = the ReplicaSet `spec.template` with `metadata.labels["pod-template-hash"]` removed. Equal to the Deployment's current template (hash removed from both) → `Invalid { message: "revision {n} has the same template as the current one", fields: [] }`, no PATCH (so the dry-run disables Apply).
4. Body: `[{"op":"test","path":"/metadata/uid","value":"<deployment uid>"},{"op":"replace","path":"/spec/template","value":<template>}]`.
5. **422 on this PATCH is a failed `test` op** (the Deployment was replaced): `Conflict { message: "the deployment was replaced since it was read", managers: [] }`.

JSON Patch, not merge patch: a merge patch would keep map keys the old revision lacks. The template (env literals) lives only inside the request build; it is never traced, returned, or audited.

## Trigger now (kubectl `create job --from=cronjob/…` parity)

```json
{"apiVersion":"batch/v1","kind":"Job",
 "metadata":{"generateName":"<cronjob, first 50 chars>-manual-","namespace":"<ns>",
  "labels":<jobTemplate.metadata.labels>,
  "annotations":<jobTemplate.metadata.annotations + "cronjob.kubernetes.io/instantiate":"manual">,
  "ownerReferences":[{"apiVersion":"batch/v1","kind":"CronJob","name":"<cronjob>","uid":"<uid>","controller":true}]},
 "spec":<jobTemplate.spec>}
```

- `controller: true`, **no `blockOwnerDeletion`**: setting it needs `update` on `cronjobs/finalizers` (OwnerReferencesPermissionEnforcement), which `create jobs` alone does not grant.
- 50 + `-manual-` (8) + 5 generated chars = 63, the Job name limit. The dry-run and the commit differ only in the server-picked suffix.

## Re-run (W7 note "Re-run creates a new Job from the template")

GET the Job, then POST a Job with `generateName` `<job, first 51 chars>-rerun-`, the Job's labels, and its `spec`, minus `spec.selector`, `spec.manualSelector`, and the labels `controller-uid`, `job-name`, `batch.kubernetes.io/controller-uid`, `batch.kubernetes.io/job-name` (in `metadata.labels` and `spec.template.metadata.labels`); `spec.suspend` is set to `false`. No owner references, no annotations copied.

## Errors

0030 mapping, plus:

- A GET failure inside an operation maps like a dry-run error (`Cluster(..)`, never `OutcomeUnknown`: nothing was sent).
- **422 on Trigger now and Re-run** → `Invalid { message: "the server rejected the generated object", fields }`: field paths only; the server message is dropped (it can quote template values such as env literals). **422 on Roll back** → the fixed-text `Conflict` of step 5, so no server text reaches the UI there either.
