# 0014 · Cluster crate: storage summaries, access, YAML masking, probe

[Back to index](README.md) · Step 1 · The 0001/0002 rules apply. Common fields as 0005 (`namespace` only on PVCs). `is_terminating` = `metadata.deletionTimestamp` is set. Access modes keep API names (`ReadWriteOnce`); the app abbreviates.

## PVC (`persistent_volume_claim.rs`, core/v1)

```rust
pub struct PersistentVolumeClaimSummary { /* common */ pub phase: String /* "Pending" when status is absent */,
    pub is_terminating: bool, pub volume: Option<String> /* spec.volumeName */,
    pub capacity: Option<String> /* status.capacity["storage"] */, pub requested: Option<String> /* spec.resources.requests["storage"] */,
    pub access_modes: Vec<String> /* status.accessModes, else spec.accessModes */, pub storage_class: Option<String>,
    pub volume_mode: Option<String>, pub conditions: Vec<WorkloadCondition> /* status.conditions, with message */ }
impl ClusterConnection { pub fn watch_persistent_volume_claims(&self, scope: NamespaceScope) -> impl Stream<..>; }
```

Empty strings read as `None` (`non_empty`). Action text `watching persistent volume claims`.

## PV (`persistent_volume.rs`, core/v1, cluster-scoped)

```rust
pub struct PersistentVolumeSummary { pub name: String, pub created_at: Option<jiff::Timestamp>, pub labels: Vec<String>,
    pub capacity: Option<String>, pub access_modes: Vec<String>, pub reclaim_policy: String /* default "Retain" */,
    pub phase: String /* default "Pending" */, pub is_terminating: bool, pub claim: Option<ClaimRef>,
    pub storage_class: Option<String>, pub volume_mode: Option<String>, pub backend: VolumeBackend,
    pub node_affinity: Vec<String>, pub mount_options: Vec<String>, pub reason: Option<String>, pub message: Option<String> }
pub struct ClaimRef { pub namespace: String, pub name: String }
pub enum VolumeBackend {
    Csi { driver: String, volume_handle: String, fs_type: Option<String> },
    Nfs { server: String, path: String },
    HostPath { path: String },
    Local { path: String },
    /// An in-tree source by its field name (`awsElasticBlockStore`, `rbd`, …), or `unknown`.
    Other { kind: &'static str },
}
impl ClusterConnection { pub fn watch_persistent_volumes(&self) -> impl Stream<..>; }
```

- `claim` from `spec.claimRef` when both namespace and name are set.
- `node_affinity`: each `spec.nodeAffinity.required.nodeSelectorTerms[]` as one string, its `matchExpressions` then its `matchFields` (e.g. `metadata.name in (node-1)`) in kubectl selector syntax joined with `, ` (e.g. `topology.kubernetes.io/zone in (eu-central-1a)`).
- `message` is cut with `event::optional_message` (1 KiB).
- **Never copied**: `csi.volumeAttributes`, `csi.*SecretRef`, every other `secretRef`, `flexVolume.options` (decision 3). `spec.mountOptions` is kept as `mount_options` (as for StorageClasses).

## StorageClass (`storage_class.rs`, storage.k8s.io/v1, cluster-scoped)

```rust
pub struct StorageClassSummary { pub name: String, pub created_at: Option<jiff::Timestamp>, pub labels: Vec<String>,
    pub provisioner: String, pub reclaim_policy: String /* default "Delete" */, pub binding_mode: String /* default "Immediate" */,
    pub allows_expansion: bool, pub is_default: bool, pub parameters: Vec<StorageParameter> /* key order */,
    pub mount_options: Vec<String> }
pub struct StorageParameter { pub key: String, pub value: Option<String> /* None = hidden */ }
impl ClusterConnection { pub fn watch_storage_classes(&self) -> impl Stream<..>; }
pub(crate) fn is_secret_parameter(key: &str) -> bool; // decision 5
```

- `is_default`: annotation `storageclass.kubernetes.io/is-default-class` or `storageclass.beta.kubernetes.io/is-default-class` equals `true` (ASCII case-insensitive). The summarizer reads only these two keys (decision 4).
- `is_secret_parameter(key)`: lowercase the key and drop `-`, `_`, `.`; keys ending in `secret-name` or `secret-namespace` (references, e.g. `csi.storage.k8s.io/provisioner-secret-name`) are never secret; otherwise secret when it contains `password`, `passwd`, `token`, `credential`, `secretkey`, `accesskey`, `userkey`, `privatekey`, or `restuserkey`. So `restuserkey`, `adminPassword`, `access-key` are hidden; `kmsKeyId`, `type`, `fsType` are shown.
- Values of secret parameters are dropped before the summary is built.

## YAML masking (`object_yaml.rs`)

`to_masked_yaml` gains `mask_storage_class_parameters(&mut object)`: when `kind == "StorageClass"`, every `parameters` value whose key passes `is_secret_parameter` becomes `HIDDEN` and counts toward the "k8sBoard hid N values" header. Same shape as `mask_secret_data`.

## Other additions

| Item | Change |
|---|---|
| `access_review.rs` | `ListPersistentVolumeClaims` ("", namespaced), `ListPersistentVolumes` ("", cluster), `ListStorageClasses` (storage.k8s.io, cluster); appended to `ALL` |
| `object_yaml.rs` | `ObjectKind::{PersistentVolumeClaim, PersistentVolume, StorageClass}`; `is_namespaced` is false for the last two; `api_resource` arms |
| `object_count.rs` (0012) | three arms |
| `lib.rs` | modules; export the three summaries, `ClaimRef`, `VolumeBackend`, `StorageParameter` |
| `examples/probe.rs` | `--watch-seconds`: `persistent volume claims`, `persistent volumes`, `storage classes` after the 0013 lines; `USAGE`; 0001 `probe-example.md` |

Access and count lines grow with `ALL` and `ObjectKind`. PVC usage is not probed here (0011 `--kubelet-seconds` covers it).
