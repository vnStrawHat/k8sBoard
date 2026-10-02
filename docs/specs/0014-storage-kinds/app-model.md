# 0014 · App: kind specs, joins, PV companion, kubelet demand, menus

[Back to index](README.md) · Steps 2–3 · Modules: `resource_kind.rs`, `kind_row.rs`, `kind_join.rs`, `live_sections.rs`, `kind_diagnosis.rs`, `cluster_session.rs`, `kubelet_metrics.rs`, `app_shell.rs`, `resource_actions.rs`. Anything not listed works unchanged for a new `ResourceKind` (0013 [app-model.md](../0013-policy-kinds/app-model.md)).

## `KindSpec` statics

| Variant (S) | label | object | singular / plural | badge | ns | access | read-only actions | delete label |
|---|---|---|---|---|---|---|---|---|
| `PersistentVolumeClaims` (2) | PVCs | PersistentVolumeClaim | persistentvolumeclaim / persistentvolumeclaims | Pc | yes | `ListPersistentVolumeClaims` | Expand… | Delete PVC… |
| `PersistentVolumes` (2) | PVs | PersistentVolume | persistentvolume / persistentvolumes | Pv | no | `ListPersistentVolumes` | — | Delete PV… |
| `StorageClasses` (3) | StorageClasses | StorageClass | storageclass / storageclasses | Sc | no | `ListStorageClasses` | Set as default | Delete storage class… |

`watch_rows`: PVs and StorageClasses ignore `scope` (like Namespaces). `only_namespaces_is_cluster_scoped` becomes `cluster_scoped_kinds_are_listed` (Namespaces, PersistentVolumes, StorageClasses; 0015 extends it). A scope change does not restart a cluster-scoped explorer (0005 decision 14 generalized from Namespaces to `!kind.is_namespaced()`).

W7 parts not rendered: the drawer `meta` line (e.g. "data · gp3 · pvc-1f2e9a7c", "ebs.csi.aws.com · 58 volumes") stays 0005's subtitle (`status · namespace · created`), its facts appear as section fields; no list-level top buttons (Expand, Set default are 0032).

## Row model (`kind_row.rs`)

- `KindObject::{PersistentVolumeClaim, PersistentVolume}` (step 2). StorageClasses stay `Plain` (their joins and live content read only the row name).
- `LiveContent::{ClaimUsage, MountedBy}` (step 2), `ClassVolumes` (step 3).
- Pure `fn access_mode_short(mode: &str) -> &str`: `ReadWriteOnce` RWO, `ReadOnlyMany` ROX, `ReadWriteMany` RWX, `ReadWriteOncePod` RWOP, else as is.

## Joins (`kind_join.rs`)

```rust
pub(crate) struct JoinInputs<'a> { pub(crate) pods: &'a LiveList<PodSummary>,
    pub(crate) companion: Option<&'a CompanionLists>,      // 0012
    pub(crate) kubelet: Option<&'a KubeletHistory>,        // step 2; None without a metrics feed
    pub(crate) scope: &'a NamespaceScope }
```

| Kind (S) | Column | Source | Triggers |
|---|---|---|---|
| PVCs (2) | `CLAIM_USED` | `kubelet.pvc_usage(ns, name)`: used / capacity, both present and capacity > 0 | explorer snapshot; each kubelet `Snapshot` round while PVCs is the explorer kind |
| StorageClasses (3) | `CLASS_VOLUMES` | companion PVs with `storage_class == name` | explorer snapshot; companion snapshot or failure |

Cell and status rules are in [storage-rows.md](storage-rows.md). The Services arm reads `CompanionLists::EndpointSlices` from `companion`.

## PV companion (step 3, `cluster_session.rs`)

0012 already has `KindList.companion: Option<Companion>`, `CompanionLists`, `CompanionUpdate`, `CompanionKind`, `companion_plan`, `companion()`, and `OpenWatches.companion: usize`. This spec only adds variants and arms; nothing is renamed:

```rust
pub(crate) enum CompanionLists { /* EndpointSlices (0012) */ PersistentVolumes(LiveList<PersistentVolumeSummary>) }
enum CompanionUpdate { /* … */ PersistentVolumes(WatchUpdate<PersistentVolumeSummary>) }
pub(crate) enum CompanionKind { /* EndpointSlices */ PersistentVolumes }
```

- `companion_plan(StorageClasses, ..)`: `Denied(ListPersistentVolumes)` for a `Known` denial (the drawer says "Not permitted: list persistentvolumes"), else `Start(PersistentVolumes)` with `watch_persistent_volumes()`.
- `CompanionLists::apply` ignores a variant that does not match (0012). Started and dropped with the explorer; a scope change does not restart a cluster-scoped explorer or its companion.
- `OpenWatches.companion` = 1 for this cluster-scoped companion. StorageClasses: `2 + N + 1 + 1 + 1` ≤ `3N + 4`.

## Kubelet demand (step 2, 0011 `kubelet_metrics.rs`, `app_shell.rs`)

- `KubeletSubject::Claim { namespace: String, claim: String }`.
- `subject_nodes(Claim)`: nodes of the namespace's non-Done pods with a container mount `VolumeSource::PersistentVolumeClaim { claim }`, most pods first, then name.
- `sync_kubelet_demand`: open drawer of a PVCs row → `Claim { namespace, name }`, `wants_disk_io: false`.

## Diagnosis (`kind_diagnosis.rs`, step 2)

Arms for PVC (VOLUME LOST) and PV (RELEASED, RECLAIM FAILED); object-only rules ([storage-rows.md](storage-rows.md)).

## Menus (`resource_actions.rs`, step 2)

| Kind | Item after View YAML | Target / disabled reason |
|---|---|---|
| PVCs | Go to pod | first pod by name from `claim_pods` (the MountedBy helper); none → "Not mounted by any pod" |
| PVs | Go to claim | `of_object("PersistentVolumeClaim", ns, name)`; no claim → "No claim" |

Both use 0013 `go_to_item`. `kind_menu` reads `live.pods` for PVCs (already reachable at both call sites, as 0009 View pods on node).
