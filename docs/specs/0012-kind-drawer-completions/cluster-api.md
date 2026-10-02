# 0012 · Cluster crate additions

[Back to index](README.md) · Step 1b (the `timetable` row is step 1a) · The 0001/0002 API rules apply: summaries only, no kube types in public signatures, no spawned task. Cron parsing is in [cron-schedule.md](cron-schedule.md).

## New summary fields

| Type (module) | Field | Source |
|---|---|---|
| `WorkloadCondition` (`workload.rs`) | `message: Option<String>` | `conditions[].message`, cut with `event::optional_message` (1 KiB) |
| `ContainerPort` (`workload.rs`) | `host_port: Option<u16>` | `hostPort`; out of `u16` → `None` |
| `DeploymentSummary` | `progress_deadline_seconds: u32` | `spec.progressDeadlineSeconds`, default 600 |
| `JobSummary` | `active_deadline_seconds: Option<u64>`, `ttl_seconds_after_finished: Option<u32>` | `spec.*` |
| `StatefulSetSummary` | `claim_retention: Option<String>` | `persistentVolumeClaimRetentionPolicy` as `whenDeleted Retain · whenScaled Delete`; `None` when unset |
| `CronJobSummary` | `timetable: Result<CronSchedule, ScheduleError>` | `CronSchedule::parse(schedule, time_zone)`, then `anchored_at(last_schedule_at.or(created_at))` (step 1a) |
| `IngressPath` | `service: Option<String>` | the service backend's name; `None` for resource backends |
| `IngressSummary` | `default_service: Option<String>` | same, for `defaultBackend` |
| `VolumeSource::Projected` | `{ config_maps: Vec<String> }` | `projected.sources[].configMap.name`, in order; other sources ignored |

Every fixture that builds these types gains the field (crate and app tests).

## EndpointSlices (`endpoint_slice.rs`, discovery.k8s.io/v1, new)

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointSliceSummary { pub namespace: String, pub name: String,
    pub service: Option<String>,          // label kubernetes.io/service-name
    pub address_type: String,             // IPv4 | IPv6 | FQDN
    pub ports: Vec<EndpointPort>, pub endpoints: Vec<EndpointSummary> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointPort { pub name: Option<String>, pub port: Option<u16>, pub protocol: String } // TCP default
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointSummary { pub address: String,  // the first address; endpoints without one are dropped
    pub is_ready: bool,                            // conditions.ready; None reads true (API doc)
    pub is_terminating: bool,                      // conditions.terminating; None reads false
    pub pod: Option<String>,                       // targetRef.name when targetRef.kind == "Pod"
    pub node: Option<String> }
impl ClusterConnection {
    pub fn watch_endpoint_slices(&self, scope: NamespaceScope) -> impl Stream<Item = WatchUpdate<EndpointSliceSummary>> + Send + 'static;
}
```

No labels other than the service name, no annotations, hints, hostnames, or zones are kept. `conditions.serving` is intentionally not read: only `ready` decides whether a non-terminating endpoint takes traffic. Slices without the service label are kept (`service: None`) and ignored by the app. Action text: `watching endpoint slices`.

## Selected watches (drawer-scoped, one namespace)

```rust
impl ClusterConnection {
    /// ReplicaSets matching `selector` (kubectl selector syntax, e.g. `app=api,tier in (a,b)`).
    pub fn watch_selected_replica_sets(&self, namespace: &str, selector: &str) -> impl Stream<Item = WatchUpdate<ReplicaSetSummary>> + Send + 'static;
    /// Every Job of the namespace; the app keeps those owned by the CronJob.
    pub fn watch_namespace_jobs(&self, namespace: &str) -> impl Stream<Item = WatchUpdate<JobSummary>> + Send + 'static;
    /// One ConfigMap (field selector `metadata.name`), as value previews.
    pub fn watch_config_map_values(&self, namespace: &str, name: &str) -> impl Stream<Item = WatchUpdate<ConfigMapValues>> + Send + 'static;
}
// resource_watch.rs: summary_watch with a server-side config and no store limit
pub(crate) fn selected_summary_watch<K, T>(connection, apis, config: watcher::Config, action, summarize) -> impl Stream<..>;
```

- An empty selector would select every ReplicaSet: `watch_selected_replica_sets` with `""` returns a stream that emits `Snapshot(vec![])` once and ends.
- `ConfigMapValues` (`config_map.rs`):

```rust
#[derive(Clone, PartialEq, Eq)] // no Debug: values must never reach a log
pub struct ConfigMapValues { pub namespace: String, pub name: String, pub entries: Vec<ConfigMapValue> }
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigMapValue { pub key: String, pub preview: ValuePreview }
#[derive(Clone, PartialEq, Eq)]
pub enum ValuePreview { Line(String), Json { size_bytes: usize }, Text { size_bytes: usize, lines: usize }, Binary { size_bytes: usize } }
```

| Value | Preview |
|---|---|
| `binaryData` key | `Binary` (decoded length) |
| no `\n` (ignoring one trailing), ≤ 120 chars | `Line(value)` |
| trimmed starts with `{` or `[` | `Json` |
| otherwise | `Text` (`lines` = line count) |

Keys sorted by name. `LiveList<T>` needs no `Debug`; if a `Debug` bound appears, implement it by hand printing key names only.

## Object counts (`object_count.rs`, new)

```rust
impl ClusterConnection {
    /// The object count of `kind` in `scope`: one `list` with `limit=1` per namespace (or one for All
    /// and cluster-scoped kinds), `items.len() + remainingItemCount`. `None` when a list has a
    /// continue token but no remaining count.
    pub async fn count_objects(&self, kind: ObjectKind, scope: &NamespaceScope) -> Result<Option<u64>, ClusterError>;
}
```

`list_metadata` (metadata only), one `match` over `ObjectKind` to pick `K`, with `ListParams` from one private helper `fn count_params() -> ListParams { ListParams::default().limit(1) }`: `resource_version` stays `None`, because `resourceVersion=0` is answered from the watch cache, which ignores `limit` and omits `remainingItemCount`. `Several`: requests run one after another; any `None` gives `None`. Action text `counting {kind}`.

## Access check

`AccessCheck::ListEndpointSlices` → `("list", "discovery.k8s.io", "endpointslices", None, true)`, appended to `ALL`. Display reads "list endpointslices". With `Several` it is a namespaced check per namespace (0009).

## `lib.rs`

Export `CronSchedule`, `ScheduleError`, `EndpointPort`, `EndpointSliceSummary`, `EndpointSummary`, `ConfigMapValue`, `ConfigMapValues`, `ValuePreview`.

## Probe (`examples/probe.rs`)

- `--watch-seconds`: one more line, `endpointslices`, after `configmaps`.
- `--counts`: after the access section, one line per `ObjectKind` (13), in sidebar order: `count {plural} {n}` or `count {plural} unknown`; denied kinds print `count {plural} denied` without a request.
- Step 1a: the `cronjobs` watch line appends `next {time in its zone}` of the first cron job when one exists (shows tz bundling works on UAT).
- Update `USAGE` and [0001 probe-example.md](../0001-cluster-read-only/probe-example.md).
