# 0014 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; nothing lands before its first user (the probe is the crate's first user in step 1). No Cargo change (`k8s_openapi::api::storage::v1` is in `v1_32`).

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/persistent_volume_claim.rs` (new) + tests in module | summary, `watch_persistent_volume_claims` |
| `src/persistent_volume.rs` (new) + tests in module | summary, `ClaimRef`, `VolumeBackend`, `watch_persistent_volumes` |
| `src/storage_class.rs` (new) + tests in module | summary, `StorageParameter`, `is_secret_parameter`, `watch_storage_classes` |
| `src/object_yaml.rs` (+ `object_yaml_tests.rs`) | 3 `ObjectKind`s, `api_resource` arms, `mask_storage_class_parameters` |
| `src/object_count.rs` | 3 arms |
| `src/access_review.rs` | 3 checks, `ALL`, tests |
| `src/lib.rs` | modules and exports |
| `examples/probe.rs` | 3 watch lines, `USAGE` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/resource_kind.rs` | `PersistentVolumeClaims`, `PersistentVolumes` statics, `ALL`, `watch_rows`; cluster-scoped test |
| 3 | `src/resource_kind.rs` | `StorageClasses` |
| 2 | `src/kind_row.rs` | `KindObject::{PersistentVolumeClaim, PersistentVolume}`, `LiveContent::{ClaimUsage, MountedBy}`, `access_mode_short` |
| 3 | `src/kind_row.rs` | `LiveContent::ClassVolumes` |
| 2 | `src/storage_rows.rs` (new) + `storage_rows_tests.rs` | `persistent_volume_claim_row`, `persistent_volume_row`, `phase_label`, `size_cell`, `modes_text` |
| 3 | `src/storage_rows.rs` | `storage_class_row` |
| 2 | `src/kind_join.rs` (+ tests) | `JoinInputs.kubelet`, PVCs arm, `CLAIM_USED` |
| 3 | `src/kind_join.rs` | StorageClasses arm, `CLASS_VOLUMES` |
| 2 | `src/live_sections.rs` (+ tests) | `ClaimUsage`, `MountedBy`, `claim_pods` |
| 3 | `src/live_sections.rs` | `ClassVolumes` |
| 2 | `src/kind_diagnosis.rs` (+ tests) | VOLUME LOST, RELEASED, RECLAIM FAILED |
| 2 | `src/kubelet_metrics.rs` (+ tests), `src/app_shell.rs` | `KubeletSubject::Claim`, `subject_nodes` arm, `sync_kubelet_demand` row |
| 2 | `src/cluster_session.rs` (+ tests) | join after a kubelet round while PVCs is shown; scope change keeps any cluster-scoped explorer |
| 3 | `src/cluster_session.rs` (+ tests) | `PersistentVolumes` variants of `CompanionLists`, `CompanionUpdate`, `CompanionKind`; `companion_plan` arm |
| 2 | `src/resource_actions.rs` (+ tests) | Go to pod, Go to claim |
| 2, 3 | `src/main.rs` | `mod storage_rows` |

## Docs (with the last step)

- `docs/roadmap/inventory-kinds.md`: PVCs, PVs, StorageClasses → Done (Expand, Set default stay 0032).
- `docs/roadmap/cross-cutting.md` UAT table: storage probe results.
- `docs/specs/0011-metrics-kubelet/README.md` open item 2 → "0014 decision 9: drawer demand only".
- `docs/roadmap/README.md`: status row; 0001 `probe-example.md`: watch line list.
