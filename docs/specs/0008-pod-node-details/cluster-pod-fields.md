# 0008 · Cluster: pod and container fields

[Back to index](README.md) · Step 1 · Modules: `pod.rs`, `container_spec.rs` (new), `workload.rs`, `event.rs`, `lib.rs`. The 0001 API rules apply: no kube or k8s-openapi type in a public signature, no `tracing::` in these modules.

## `PodSummary` and `PodCondition` (`pod.rs`)

```rust
pub struct PodSummary { /* … */
    /// `status.message` (failed, evicted, or rejected pods), cut like event messages.
    pub status_message: Option<String>,
}
pub struct PodCondition { pub name: String, pub is_true: bool,
    pub reason: Option<String>,   // empty -> None
    pub message: Option<String>,  // cut like event messages; e.g. "0/4 nodes are available: …"
}
pub enum ContainerState {
    Waiting { reason: Option<StatusReason>, message: Option<String> }, // message cut like events
    /* Running, Terminated, NotReported unchanged */
}
```

- "Cut like event messages" = `event::truncate_message` (trim, 1 KiB, `…`), made `pub(crate)`; empty text is `None`.
- `status_message` is kept **only when phase is `Failed` or `status.reason` is `Evicted`**; otherwise None (WHY rule P3 relies on it).

## `StatusReason` (`pod_status.rs`)

Add `InvalidImageName`, `ErrImageNeverPull`, `CreateContainerError` to the enum, `from_api`, and `Display` (API text as written). The app adds them to `status_tone::is_bad_reason` (made `pub(crate)`), which the state label and the WHY rules share.
- `lastProbeTime`/`lastTransitionTime` are not kept (decision 6).

## `ContainerSummary` additions

```rust
pub struct ContainerSummary { /* … */
    /// `status.imageID` after its last `@`; the whole id when it starts with `sha256:`; else None.
    pub image_digest: Option<String>,
    pub pull_policy: Option<String>,        // imagePullPolicy as written (the API defaults it)
    pub is_started: Option<bool>,           // status.started; None when unreported
    pub ports: Vec<ContainerPort>,          // workload::ContainerPort, spec order
    pub resources: Vec<ContainerResource>,  // cpu, memory, ephemeral-storage, then others by name
    pub probes: ContainerProbes,
    pub env: Vec<EnvEntry>,                 // spec order
    pub env_from: Vec<EnvFromEntry>,        // spec order
    pub mounts: Vec<MountEntry>,            // spec order
}
```

## New types (`container_spec.rs`, exported from `lib.rs`)

```rust
pub struct ContainerResource { pub name: String, pub request: Option<String>, pub limit: Option<String> }
pub struct ContainerProbes { pub liveness: Option<ProbeSummary>, pub readiness: Option<ProbeSummary>,
    pub startup: Option<ProbeSummary> }
pub struct ProbeSummary { pub action: ProbeAction,
    pub period_seconds: u32,     // default 10
    pub failure_threshold: u32 } // default 3
pub enum ProbeAction {
    HttpGet { scheme: String /* default "HTTP" */, port: String, path: String /* default "/" */ },
    TcpSocket { port: String },
    Grpc { port: u16 },
    Exec { command: Vec<String> }, // the argv as written (N23); shown, never logged
    Unknown,  // no handler set
}
pub struct EnvEntry { pub name: String, pub source: EnvSource }
pub enum EnvSource {
    Literal,                                   // `value`; the value is never read
    ConfigMapKey { name: String, key: String },
    SecretKey { name: String, key: String },
    Field { path: String },                    // fieldRef.fieldPath
    ResourceField { resource: String },        // resourceFieldRef.resource
    Unknown,                                   // valueFrom with no known ref
}
pub struct EnvFromEntry { pub source: EnvFromSource, pub prefix: Option<String> }
pub enum EnvFromSource { ConfigMap { name: String }, Secret { name: String }, Unknown }
pub struct MountEntry { pub path: String, pub volume: String, pub source: VolumeSource,
    pub is_read_only: bool, pub sub_path: Option<String> }
pub enum VolumeSource {
    ConfigMap { name: String }, Secret { name: String }, PersistentVolumeClaim { claim: String },
    EmptyDir, HostPath { path: String }, Projected, DownwardApi,
    Other, // any other type, or a volume name missing from spec.volumes
}
```

All derive `Clone, Debug, PartialEq, Eq` like the other summaries.

## Mapping rules

| Field | Rule |
|---|---|
| `port` texts | `IntOrString` → `int_or_string_text` (`8080` or `http`) |
| HTTP `path` | text before the first `?`; the query string is dropped (decision 9). `httpHeaders` are never read |
| Grpc port | `u16::try_from(port).ok()`; out of range → `Unknown` action |
| `period_seconds`, `failure_threshold` | `optional_count` with the API defaults 10 and 3 when absent |
| resources | union of `requests` and `limits` keys; `Quantity.0` as written; order cpu, memory, ephemeral-storage, then by name |
| `env` | `value` → `Literal` (also when `value` is absent and `valueFrom` is `None`) |
| mounts | `volumeMounts`; the source is looked up by `volume` name in `spec.volumes`; `readOnly` absent → false; empty `subPath` → None |
| `ports` | extract `pub(crate) fn container_ports(&Container) -> Vec<ContainerPort>` from `template_containers` in `workload.rs`; both use it |

- `container_summaries(pod)` passes `spec.volumes` to each `container_summary`. The spec-only fields are filled for containers with and without a status.
- Functions in `container_spec.rs`: `pub(crate) fn container_resources`, `container_probes`, `env_entries`, `env_from_entries`, `mount_entries(&Container, &[Volume])`, `image_digest(&str)`.
- `TemplateContainer` doc comment ("env … volumeMounts can hold plaintext secrets") stays: templates keep ports only.

## `EventSummary.container` (`event.rs`)

```rust
pub struct EventSummary { /* … */
    /// The container named by `involvedObject.fieldPath`: `spec.containers{api}`,
    /// `spec.initContainers{…}`, or `spec.ephemeralContainers{…}`; else None.
    pub container: Option<String>,
}
```

Pure `fn field_path_container(path: &str) -> Option<String>`; the kubelet's `implicitly required container {name}` (and any other text) → None. Used to tie probe failure events to a container ([pod-diagnosis.md](pod-diagnosis.md)).

## Secret rules (C1)

- Env: names and source references only. `EnvVar.value` is never read into any summary.
- Probes: no HTTP headers, no query string; the exec argv is kept (README decision 9).
- Terminated `message` (the termination log, possibly app output) is never kept (decision 8).
- Annotations are never read. Mount paths, volume, ConfigMap, Secret, and PVC names are kept (names, not contents).
