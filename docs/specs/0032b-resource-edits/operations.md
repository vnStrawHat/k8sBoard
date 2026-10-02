# 0032b · Operations (cluster crate)

[Back to index](README.md) · Step 1 · Modules: `object_write.rs` (+ `object_write_tests.rs`), `access_review.rs`. Decisions 1–6. Contract: 0030 write-path.md as amended; 0032 write-operations.md style.

## New operations

```rust
pub enum WriteOperation {
    // 0030, 0031, 0032, 0033, 0034 variants …
    SetHpaReplicaRange { min: u32, max: u32 },          // HorizontalPodAutoscaler
    ExpandClaim { storage: String },                    // PersistentVolumeClaim; a Kubernetes quantity, stored trimmed
    SetDefaultStorageClass { is_default: bool },        // StorageClass
}
```

| `WriteRequest::new` refuses (`None`) | Why |
|---|---|
| any other kind | 0030 decision 3 |
| `min == 0` or `min > max` | `minReplicas: 0` needs the alpha `HPAScaleToZero` gate; the API rejects `min > max` |
| `storage` that, after `trim()`, `cluster::ByteAmount::parse` (`quantity.rs`, already public) rejects, or zero | a bad quantity must never reach the dry-run |

All three are JSON merge patches with `fieldManager=k8sboard` (dry-run adds `dryRun=All`), no `resourceVersion` (0030 decision 14), `effect: Patched`.

## Wire format (allow-list rows)

| Operation | HTTP | Path | Body |
|---|---|---|---|
| `SetHpaReplicaRange` | PATCH | `/apis/autoscaling/v2/namespaces/{ns}/horizontalpodautoscalers/{name}` | `{"spec":{"minReplicas":3,"maxReplicas":20}}` |
| `ExpandClaim` | PATCH | `/api/v1/namespaces/{ns}/persistentvolumeclaims/{name}` | `{"spec":{"resources":{"requests":{"storage":"150Gi"}}}}` |
| `SetDefaultStorageClass { true }` | PATCH | `/apis/storage.k8s.io/v1/storageclasses/{name}` | `{"metadata":{"annotations":{"storageclass.kubernetes.io/is-default-class":"true"}}}` |
| `SetDefaultStorageClass { false }` | PATCH | same | `{"metadata":{"annotations":{"storageclass.kubernetes.io/is-default-class":"false","storageclass.beta.kubernetes.io/is-default-class":null}}}` |

- The HPA patch uses `autoscaling/v2`, the version 0013 watches.
- `new` stores `storage.trim()` and that exact text is sent (`150Gi`), not re-formatted.
- Unset clears the beta key too: 0014 reads either key as "default" (decision 4 there), so leaving a beta `true` would keep the class default. Removing an absent key is a no-op in a merge patch.

## RBAC, dry-run, risk, audit

| Operation | `AccessCheck` (new) | SSAR | Dry-run | Risk | `changed_fields()` |
|---|---|---|---|---|---|
| `SetHpaReplicaRange` | `PatchHorizontalPodAutoscalers` | `patch autoscaling horizontalpodautoscalers`, namespaced | yes | Change | `spec.minReplicas` = `"3"`, `spec.maxReplicas` = `"20"` |
| `ExpandClaim` | `PatchPersistentVolumeClaims` | `patch "" persistentvolumeclaims`, namespaced | yes | Change (irreversible; the warning `A volume cannot shrink; this cannot be undone` carries it, as 0032 decision 13 does for scale-down) | `spec.resources.requests.storage` = `"150Gi"` |
| `SetDefaultStorageClass` | `PatchStorageClasses` | `patch storage.k8s.io storageclasses`, cluster-scoped | yes | Change | `metadata.annotations[storageclass.kubernetes.io/is-default-class]` = `"true"`/`"false"`; on unset also `metadata.annotations[storageclass.beta.kubernetes.io/is-default-class]` with value `None` (removed) |

`ALL` grows by 3. Display: `patch horizontalpodautoscalers`, `patch persistentvolumeclaims`, `patch storageclasses`.

## Errors (0030 mapping, plus)

| Case | Server answer | Mapping and text |
|---|---|---|
| Expand on a class without `allowVolumeExpansion`, or a statically provisioned claim | 403 from the `PersistentVolumeClaimResize` admission plugin (no `is forbidden: User`) | 0030 maps a non-RBAC 403 to `Invalid` (decided here with 0030): `the change is invalid: only dynamically provisioned pvc can be resized …`, never "not permitted" |
| Expand on a cluster with `PersistentVolumeClaimResize` disabled | the dry-run and the commit pass | the resize then fails asynchronously: the claim gets a `Resizing` failure condition or event, shown by the existing PVC row and events (no extra surface) |
| Expand to a smaller size | 422 `spec.resources.requests.storage: Forbidden: field can not be less than previous value` | `Invalid { fields }` |
| HPA `min > max` sent anyway (stale validation) | 422 | `Invalid { fields }` |

No Secret, template, or env value is involved, so messages are shown as the server sends them (0030 redaction applies to Secret targets only).
