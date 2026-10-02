# 0014 — Storage kinds: PVCs, PVs, StorageClasses (read-only)

Status: **implemented** (steps 1 to 3); amended after advisor review (HEAD `81497ba`); architect defaults, the user asked not to stop for questions. Crates: `crates/cluster` (step 1), `crates/app` (steps 2–3). Requires the amended 0012 (`KindObject`, `Live`, `Bar`, joins, the general `KindList.companion`, counts), 0011 (`KubeletHistory::pvc_usage`, `KubeletSubject`, `sync_kubelet_demand`), and 0013 (`go_to_item`, `quantity_ratio`) merged. Wireframes: W7 `k("PVCs")`, `k("PVs")`, `k("StorageClasses")`. Applies C1, C11.

## Goal

Three new explorer kinds on the 0005 explorer (one `KindSpec`, row builder, lazy watch, list SSAR, Events/YAML tabs, 0009 toolkit, 0012 counts):

- **PVCs**: status, capacity, **Used %** from kubelet stats (0011; "—" without it), access, class; drawer Usage bars, **Mounted by** pods, links to its PV and StorageClass, Go to pod;
- **PVs**: capacity, access, reclaim, status, claim link, class, source (CSI driver and handle, NFS, path), node affinity, **RELEASED** hint;
- **StorageClasses**: provisioner, reclaim, binding mode, expansion, **default ★**, **PV count** (companion watch), parameters with password-like values hidden.

## Non-goals

Expand, Set default, Delete, Clean up (0032/0033; menu items disabled); VolumeSnapshots, VolumeAttachments, CSIDrivers; VolumeAttributesClass; demanding kubelet stats for every node just for the PVC table (decision 9); a "two default classes" warning (open item 2).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `crates/cluster`: three summaries and watches, 3 `AccessCheck`s, 3 `ObjectKind`s, StorageClass parameter masking in the YAML view, probe. **Run the UAT probe first** and fill [decisions.md](decisions.md) "UAT probe" | 1, 2, 3, 4 |
| 2 | App: **PVCs** and **PVs** (rows, Used join, ClaimUsage and MountedBy live content, kubelet `Claim` demand, RELEASED box, Go to pod / claim) | 1, 2, 3, 5, 6, 7 |
| 3 | App: **StorageClasses**, the `PersistentVolumes` companion variant, PVs column, ClassVolumes; full ui-verifier run | 1, 2, 3, 5, 6, 7, 8 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale, ceilings, UAT probe table |
| [cluster-api.md](cluster-api.md) | step 1: summaries, watches, access checks, YAML masking, probe |
| [app-model.md](app-model.md) | steps 2–3: kind specs, W7 parts not rendered, `KindObject`, joins, PV companion, kubelet demand, menus |
| [storage-rows.md](storage-rows.md) | steps 2–3: columns, cells, status, sections, live content, boxes |
| [files-to-touch.md](files-to-touch.md) | modules per step, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [x] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`. No `Cargo.lock` change.
- [x] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [x] 3. No kube or k8s-openapi type in a public signature; the crate spawns no task; the 0001 read-only grep finds only the SSAR `create`. Summaries keep no annotation except the default-class flag (decision 4); no CSI `volumeAttributes` or secret references are copied.
- [ ] 4. On UAT the probe prints 3 new watch, access, and count lines; results are in [decisions.md](decisions.md) "UAT probe". The AC7 credential script reports 0. Not ticked: the AC7 credential script was not run; the substitute is in decisions.md "UAT probe".
- [x] 5. On UAT each allowed kind shows live rows and a drawer with Overview, YAML, Events tabs; a denied kind is disabled with "Not permitted: list …".
- [ ] 6. On UAT, if a mounted PVC exists: its Used cell and Usage bars match the kubelet `stats/summary` volume numbers (read through `nodes/proxy`) within 1 % (spot check), and Mounted by lists its pod. Not ticked: `kubectl` was not available for the independent spot check; see decisions.md "Known ceilings" (shared filesystems) and the 0014 step 2 report.
- [x] 7. The 0003 AC4 color-literal grep is clean; the step's screenshots exist; the ui-verifier reports no high-severity defect against W7.
- [x] 8. Watches per session stay at most `3N + 4` (`open_watch_count` test covers the PV companion).

## Open items

1. PVC Pending reasons (WaitForFirstConsumer, ProvisioningFailed) show only in the Events tab; a WHY box needs object events in `DiagnosisInputs`.
2. Two default StorageClasses (new claims get the newest) are not flagged.
3. Large clusters (> 10 Ready nodes) show Used only for claims on polled nodes; a slow all-node cadence would fill the column (0011 open item 2).
