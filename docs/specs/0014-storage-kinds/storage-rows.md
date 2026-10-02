# 0014 · Storage rows: columns, status, sections, live content

[Back to index](README.md) · Module: `storage_rows.rs` (new) + `storage_rows_tests.rs`; `live_sections.rs`, `kind_diagnosis.rs`. Common rules (Name first, Age last, `Absent` = "—") are 0005's. Byte text uses 0010 `Measure::Bytes`.

## Shared (pure, `storage_rows.rs`)

```rust
fn phase_label(phase: &str, is_terminating: bool) -> StatusLabel; // decision 16
fn size_cell(text: Option<&str>) -> KindCell; // Quantity { text as written, ByteAmount bytes, None }; unparsable → Text; None → Absent
fn modes_text(modes: &[String]) -> String;    // short names joined ","
```

`phase_label`: terminating → Info "Terminating"; Bound, Available → Ok; Pending → Warn; Released → Done; Lost, Failed → Bad; other text → Warn.

## PVCs (step 2)

| Column | Width | Cell |
|---|---|---|
| Status | 110 | `Toned(phase_label)` |
| Capacity | 90 r | `size_cell(capacity)`; an unbound claim shows `requested` muted (`Done` tone) |
| Used | 80 r | joined: `Quantity { format_percent(ratio), permille, usage_tone(ratio) }`; no stats → `Absent` |
| Access | 90 | `modes_text`, or `Absent` |
| Class | 150 | `Text` or `Absent` |
| Age | 70 r | |

Status: `phase_label`; for a Bound claim, condition `Resizing` true → Info "Resizing", else `FileSystemResizePending` true → Info "Resize pending"; the drawer Status row reads "Resize pending; restart the pod" (decision 20). In Conditions, a true `Resizing` or `FileSystemResizePending` reads Info. The join leaves the Used cell `Absent` and the status unchanged for a shared filesystem (see Known ceilings in decisions.md). Otherwise it raises a Bound claim to Warn/Bad "{pct} used" when `usage_tone` gives a tone.

WHY **VOLUME LOST** (Bad), phase `Lost`: `The bound volume {volume} no longer exists. The data on it is gone or unreachable.`

| Section | Rows |
|---|---|
| WHY | above |
| Claim | Status (toned), Volume (`Link` → PV when `volume` is set, else `Absent`), Class (`Link` → StorageClass when the kind has a screen, else `Text`), Requested, Capacity, Access modes (`ReadWriteOnce (RWO)`, joined), Volume mode |
| Usage | `Live(ClaimUsage)` |
| Mounted by | `Live(MountedBy)` |
| Conditions | 0005 rows, only when any |
| Labels | |

### `ClaimUsage` (Live; filesystem mode only)

From `live.metrics.kubelet.history.pvc_usage(ns, name)`:

- `Bar { "Used", percent(used/capacity), Measure::Bytes.format_pair(used, capacity, " of "), usage_tone }`; for a shared filesystem (kubelet capacity larger than the claim) the label is "Node filesystem" and a note "Shared with the node: the claim has no quota of its own" follows the bars;
- `Bar { "Inodes", percent(inodes_used/inodes), format_percent(..), usage_tone }` when both are known;
- then a muted note `Sampled {age} ago`.

No sample: `volume_mode` is `Block` → "Block volumes report no usage"; claim not Bound → "No usage until the claim is bound"; feed `Unavailable(reason)` → "Usage unavailable: {reason}"; else "No usage data yet: kubelet stats appear once a running pod mounts the claim".

### `MountedBy` (Live; filesystem mode only)

```rust
pub(crate) fn claim_pods<'a>(namespace: &str, claim: &str, pods: &'a [PodSummary]) -> Vec<(&'a PodSummary, &'a str /* first mount path */)>;
```

Pods of the namespace with any container mount whose source is `PersistentVolumeClaim { claim }`, sorted by name, one entry per pod. Block-mode claims are attached as `volumeDevices`, which pod summaries do not keep, so they read "Not mounted by any pod" with the note "Block volumes are not listed" (decision ceiling). Row: pod name (mono, click → `reveal`) · muted `on {node} · {path}` (`unscheduled` without a node). At most 50, then `+{n} more`. Empty / loading / failed: "Not mounted by any pod" / "Loading pods…" / "Pods are unavailable".

## PVs (step 2)

| Column | Width | Cell |
|---|---|---|
| Capacity | 90 r | `size_cell(capacity)` |
| Access | 90 | `modes_text` |
| Reclaim | 90 | `Text(reclaim_policy)` |
| Status | 110 | `Toned(phase_label)` |
| Claim | 240 | `Qualified { prefix: namespace, text: name }`, or `Absent` |
| Class | 150 | `Text` or `Absent` |
| Age | 70 r | |

Status: `phase_label`.

| Box | When | Text |
|---|---|---|
| RELEASED (Warn) | Released, Retain | `Claim {ns}/{name} was deleted. Reclaim policy Retain keeps the data on the volume. Delete the PV and its backing volume to free the space, or clear claimRef to bind it again.` |
| RELEASED (Warn) | Released, other policy | `Claim {ns}/{name} was deleted and reclaim policy is {policy}, but the volume still exists. Check the Events tab for reclaim errors.` |
| RECLAIM FAILED (Bad) | Failed | `{reason}: {message}` (either part may be missing) |

| Section | Rows |
|---|---|
| WHY | above |
| Volume | Status (toned), Claim (`Link` → PVC, else `Absent`), Reclaim policy, Class (`Link`/`Text`), Capacity, Access modes, Volume mode, Mount options (joined, only when any) |
| Source | CSI: Driver, Volume handle (Mono), FS type · NFS: Server, Path · HostPath / Local: Type, Path · Other: Type `{kind}` |
| Node affinity | `Chips(node_affinity)`; omitted when empty |
| Labels | |

## StorageClasses (step 3)

| Column | Width | Cell |
|---|---|---|
| Provisioner | 200 | `Mono` |
| Reclaim | 90 | `Text` |
| Binding mode | 170 | `Text` |
| Expansion | 90 | `Yes` / `No` |
| Default | 70 | `Text("★")` or `Absent` |
| PVs | 70 r | joined: `Quantity { n, n, None }`; companion not ready or denied → `Absent` |
| Age | 70 r | |

Status: default → Ok "Default"; else Done "Not default".

| Section | Rows |
|---|---|
| Class | Provisioner (Mono), Default (Yes/No), Reclaim policy, Binding mode, Volume expansion, Mount options (joined, only when any) |
| Parameters | `{key}` → Mono value, or muted `hidden` when `value` is `None`; none → `Note("No parameters")` |
| Volumes | `Live(ClassVolumes)` |
| Labels | |

`ClassVolumes`: companion PVs with this class: `Field { "Persistent volumes", "{n} · {bound} bound · {released} released" }` (zero parts skipped), then up to 20 PV `Link`s by name (label = phase). Loading / failed / denied: "Loading volumes…" / "Volumes are unavailable" / "Not permitted: list persistentvolumes".
