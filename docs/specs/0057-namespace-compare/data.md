# 0057 data — cluster side

Module `crates/cluster/src/namespace_compare.rs` (tests in `namespace_compare_tests.rs`). Everything here is read-only; the text of an object never reaches a log.

## Kinds

`COMPARE_KINDS`, in the order the dialog shows them: Deployment, StatefulSet, DaemonSet, CronJob, Service, ConfigMap, Secret, Ingress, PersistentVolumeClaim, HorizontalPodAutoscaler, ResourceQuota. A Helm release Secret (`helm.sh/release.v1`) is left out: it is a record, not an object of the namespace.

## Reading

`ClusterConnection::compare_namespaces(left, right, env) -> NamespaceComparison`:

1. For each kind and each namespace, `list_all` of a `DynamicObject` API (the existing paged, timeout-bound helper): 22 requests at once with `futures::future::join_all`.
2. A kind whose list fails becomes `KindOutcome::Unreadable(message)` for the pair (`ClusterError` text, no object content). The call itself never fails.
3. Each item gets its `kind` (list items carry none), then is reduced by `reduce_object` (below) into a `serde_json::Value`.
4. `compare` (pure) matches the two maps by name.

## Reducing one object

1. `managedFields` removed; manifest annotations masked (`mask_manifest_annotations`); env literals masked unless `EnvValues::Shown` (`mask_object`, which also counts `hidden_env_values`).
2. **Secret**: `data` and `stringData` values become `<hash:{16 hex}>` tokens from one `RandomState` per call (equal values give equal tokens; the salt is random per call and dropped with it). `mask_secret_data` is not used for them.
3. `clean_value` (the `clean_yaml` rules, split out of it): drop `status`, `creationTimestamp`, `generation`, `resourceVersion`, `uid`, `managedFields`, `selfLink`, and every `<hidden>` entry.
4. Per-namespace noise dropped: `metadata.namespace`, `metadata.ownerReferences`; annotations `deployment.kubernetes.io/revision`, `meta.helm.sh/release-namespace`, `pv.kubernetes.io/bind-completed`, `pv.kubernetes.io/bound-by-controller`, `volume.kubernetes.io/selected-node`, `volume.kubernetes.io/storage-provisioner`, `volume.beta.kubernetes.io/storage-provisioner`; Service `spec.clusterIP` and `spec.clusterIPs`; PVC `spec.volumeName`. An annotations map left empty is removed.
5. Keys sorted (`sort_all_objects`).

## Matching and result types

```rust
pub const COMPARE_KINDS: [ObjectKind; 11];
pub struct NamespaceComparison { pub left: String, pub right: String,
    pub kinds: Vec<KindComparison>, pub hidden_env_values: usize }
pub struct KindComparison { pub kind: ObjectKind, pub outcome: KindOutcome }
pub enum KindOutcome { Unreadable(String),
    Compared { only_left: Vec<String>, only_right: Vec<String>,
               differs: Vec<ObjectDifference>, same: usize } }
pub struct ObjectDifference { pub name: String, pub changes: Vec<FieldChange>,
    pub more_changes: usize, pub left_text: String, pub right_text: String }
pub struct CompareCounts { pub differ: usize, pub only_left: usize, pub only_right: usize, pub same: usize }
impl NamespaceComparison { pub fn counts(&self) -> CompareCounts }
```

Names are sorted. `changes` is `field_changes(left, right, field_paths(left, right))` (at most 200, `more_changes` is the rest); `left_text` and `right_text` are the YAML of the reduced values (without a header), built only for a differing object. A Secret change prints tokens, never a value. No `Debug` derive on the types that hold texts.
