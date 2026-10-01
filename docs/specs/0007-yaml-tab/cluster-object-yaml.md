# 0007 · Cluster crate: object YAML

[Back to index](README.md) · Step 1 · Modules: `src/object_yaml.rs` (new) + `src/object_yaml_tests.rs`, `src/lib.rs`, `examples/probe.rs`

## Public API (`object_yaml.rs`)

```rust
/// A built-in kind whose objects can be read one at a time. 0016 adds `Secret`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKind { Pod, Node, Namespace, Event, Deployment, StatefulSet, DaemonSet,
    ReplicaSet, Job, CronJob, Service, Ingress, ConfigMap }
impl ObjectKind {
    pub fn name(self) -> &'static str;      // "Pod", "Deployment", … (Kubernetes `kind`)
    pub fn is_namespaced(self) -> bool;     // false for Node and Namespace
}

/// One object. Always valid: a namespace exactly when the kind is namespaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRef { kind: ObjectKind, namespace: Option<String>, name: String }
impl ObjectRef {
    /// `None` when `namespace.is_some() != kind.is_namespaced()`.
    pub fn new(kind: ObjectKind, namespace: Option<String>, name: String) -> Option<Self>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EnvValues { #[default] Hidden, Shown }

/// Masked YAML of one object. No `Debug`: the text must never reach a log.
pub struct ObjectYaml {
    pub text: String,
    /// Env literals replaced by `<hidden>` (0 when `EnvValues::Shown`).
    pub hidden_env_values: usize,
}

impl ClusterConnection {
    /// One GET of `object`, masked and serialized on the calling (tokio) task.
    /// action: "reading the object YAML".
    pub async fn object_yaml(&self, object: &ObjectRef, env: EnvValues)
        -> Result<ObjectYaml, ClusterError>;
}
```

## Fetch

| Step | Rule |
|---|---|
| resource | private `fn api_resource(kind) -> ApiResource`: `ApiResource::erase::<K>(&())` with the k8s-openapi type (core v1 `Pod`, `Node`, `Namespace`, `Event`, `Service`, `ConfigMap`; apps v1; batch v1 `Job`, `CronJob`; networking v1 `Ingress`) |
| api | `Api::<DynamicObject>::namespaced_with(client, ns, &resource)` or `all_with` for cluster-scoped kinds |
| request | `self.run(ACTION, api.get(&object.name))`: timeout and `classify_error` as every request (403 → `Forbidden`, 404 → `Api { code: 404 }`) |
| convert | `serde_json::to_value(&found)`, then `to_masked_yaml(value, env)` |
| errors | conversion or serializer failure → `UnexpectedResponse { source: "the object could not be converted to YAML" }`. The library error is dropped: it could quote content |

No `tracing::` call in `object_yaml.rs`. The raw `DynamicObject` and `Value` are dropped before the function returns.

## Masking (`fn to_masked_yaml(object: Value, env: EnvValues) -> Result<ObjectYaml, BoxError>`, pure)

Order: mask, then sort, then serialize. `HIDDEN: &str = "<hidden>"`.

```rust
/// Annotations that embed a whole applied manifest, so they can carry Secret data and env literals.
const MASKED_ANNOTATIONS: [&str; 3] = ["kubectl.kubernetes.io/last-applied-configuration",
    "kapp.k14s.io/original", "kapp.k14s.io/original-diff"];
```

| # | Rule | Counted as |
|---|---|---|
| 1 | remove `metadata.managedFields` | — |
| 2 | walk the whole object; in every `metadata.annotations` map at any depth (top level, pod templates, job templates), each `MASKED_ANNOTATIONS` key's value → `HIDDEN`; other annotations kept | other |
| 3 | top-level `kind == "Secret"` (any `apiVersion`): each value of the `data` and `stringData` maps → `HIDDEN`; keys kept | other |
| 4 | `EnvValues::Hidden`: walk `spec` recursively; for every key `containers`, `initContainers`, `ephemeralContainers` whose value is an array, each item's `env` array, each entry's `value` (when a string) → `HIDDEN` | env |
| 5 | `value.sort_all_objects()` | — |

- Private helpers: `fn mask_secret_data(object: &mut Value) -> usize`, `fn mask_env_values(spec: &mut Value) -> usize`, `fn mask_manifest_annotations(object: &mut Value) -> usize` (recursive).
- ConfigMap `data` and `binaryData` are not masked (decision 8).
- Header: when `env + other > 0`, the text starts with `# k8sBoard hid {n} values as <hidden>.\n` (`1 value` singular).
- `status` and every other field are untouched.

## Serialization

```rust
let options = SerializerOptions { folded_wrap_chars: usize::MAX, ..SerializerOptions::default() };
serde_saphyr::to_string_with_options(&value, options)
```

- Defaults kept: 2-space indent, compact list indent (`- name:` at the parent key's column, like kubectl), `{}`/`[]` for empty collections, YAML 1.1 quoting (`"yes"`, `"1.0"` stay strings).
- Multi-line strings use literal blocks (`|`). `folded_wrap_chars: usize::MAX` keeps long single-line strings on one line (no `>` folding), like kubectl. If the option does not behave so, the coder records it and picks the nearest kubectl-like setting.

## `lib.rs`

`mod object_yaml;` and `pub use object_yaml::{EnvValues, ObjectKind, ObjectRef, ObjectYaml};`.

## Probe (`examples/probe.rs`)

- New flag `--yaml`: after the pod list, read the YAML of the first listed pod and the first node with `EnvValues::Hidden`, and print one line each:
  `yaml pod {ns}/{name}: {lines} lines, {hidden_env_values} env values hidden, managedFields {absent|PRESENT}, last-applied {hidden|absent|VISIBLE}`.
- Flags come from `text.contains(..)` checks; the probe never prints YAML text. A failure prints the error `Display` like other probe lines. Update `USAGE` and the module doc.
