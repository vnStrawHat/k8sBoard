# 0012 · Row model, columns, joins

[Back to index](README.md) · Steps 2–4b · Modules: `kind_row.rs`, `resource_kind.rs`, `kind_table.rs`, `kind_join.rs` (new, steps 4a–4b) + `kind_join_tests.rs`, every `*_rows.rs`

## `KindRow` additions (`kind_row.rs`, step 2)

```rust
pub(crate) struct KindRow { /* 0005–0009 fields */ pub(crate) object: KindObject }
/// The summary a row was built from, for content computed at paint time (decision 16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KindObject {
    Plain,                              // Namespaces, Events
    Deployment(DeploymentSummary), StatefulSet(StatefulSetSummary), DaemonSet(DaemonSetSummary),
    ReplicaSet(ReplicaSetSummary), Job(JobSummary), CronJob(CronJobSummary),
    Service(ServiceSummary), Ingress(IngressSummary), ConfigMap(ConfigMapSummary),
}
pub(crate) enum DetailRow { /* … */
    /// Content computed at paint time from `KindObject` and the session's live lists.
    Live(LiveContent),
    /// A labelled bar. `percent` is 0–100 (builders clamp with `percent`); `tone: None` uses the
    /// kit default color. Step 3 (DaemonSets); 0013–0015 reuse it for quantities.
    Bar { label: SharedString, percent: u8, text: SharedString, tone: Option<StatusTone> },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LiveContent {
    Revisions, NextRuns, RecentJobs,          // step 2
    NotReadyPods,                             // step 3
    Endpoints, UsedBy, ConfigMapData,         // Endpoints 4a; the others 4b
}
pub(crate) enum KindCell { /* … */
    /// The next run, painted relative to now (cron-schedule.md). Step 2.
    NextRun(CronSchedule),
    /// Right-aligned mono text that sorts by `value`: request sums. `tone` colors the text
    /// (`None` here; 0013–0015 tone usage cells). Step 4b.
    Quantity { text: SharedString, value: u64, tone: Option<StatusTone> },
}
/// Rounds a ratio to a bar percent, clamped to 0–100. Step 3.
pub(crate) fn percent(ratio: f64) -> u8;
```

- Builders clone their summary into `object` (`rows` stays `fn(&T) -> KindRow`). Variants land with their first reader (an unread variant field fails clippy): Deployment, CronJob in step 2; StatefulSet, DaemonSet, ReplicaSet, Job in step 3; Service in step 4a; Ingress, ConfigMap in step 4b. Until then those builders use `Plain`.
- `kind_table.rs` `value`: `NextRun` → `Number(next second)` or `Absent`; `Quantity` → `Number(value)`. `cell_element`: `NextRun` as plain text `in 11m`; `Quantity` mono, colored by `tone_color` when it has a tone. The `Bar` renderer: label column, `Progress::new(id).value(percent)` (`.color(tone_color(t))` only with a tone), then `text` mono.
- `kind_drawer.rs` `field_value` handles both like the table.
- `ResourceKey::of_owner(namespace: &str, owner: &ControllerRef) -> Option<ResourceKey>` wraps `of_object` (step 2) for links and Go to owner.

## Columns (`resource_kind.rs`; widths px, `r` right-aligned)

| Kind | Columns after Name | Step |
|---|---|---|
| CronJobs | Schedule 140 · Suspend 80 · Active 70 r · Last schedule 120 r · **Next run 100 r** · Age | 2 |
| Services | Type 130 · Cluster IP 140 · External IP 200 · Ports 180 · **Endpoints 100** · Age | 4a |
| ConfigMaps | Data 70 r · **Used by 220** · Age | 4b |
| Namespaces | Status 140 · **Pods 70 r · CPU req 90 r · Memory req 100 r** · Age | 4b |

Joined columns are built as `KindCell::Absent` and filled by the join. `kind_join.rs` names their indices (`SERVICE_ENDPOINTS`, `CONFIG_MAP_USED_BY`, `NAMESPACE_PODS`, `NAMESPACE_CPU`, `NAMESPACE_MEMORY`), and a test asserts each index names the right column.

## Joins (`kind_join.rs`, pure; Services in step 4a, ConfigMaps and Namespaces in step 4b)

```rust
pub(crate) struct JoinInputs<'a> {
    pub(crate) pods: &'a LiveList<PodSummary>,
    pub(crate) companion: Option<&'a CompanionLists>,   // EndpointSlices for Services (session-async.md)
    pub(crate) scope: &'a NamespaceScope,
}
/// Rewrites the joined cells (and the Service status) of `rows`. Other kinds: no-op.
pub(crate) fn join_rows(kind: ResourceKind, rows: &mut [KindRow], inputs: &JoinInputs);
pub(crate) fn service_health(service: &ServiceSummary, pods: &LiveList<PodSummary>, slices: Option<&LiveList<EndpointSliceSummary>>) -> ServiceHealth;
pub(crate) fn config_map_users(pods: &[PodSummary]) -> ConfigMapUsers; // namespace → name → Vec<UsedBy>
pub(crate) fn namespace_load(namespace: &str, pods: &[PodSummary]) -> NamespaceLoad;
```

Each index is built once per join call, then each row is a lookup. Pods are indexed as `HashMap<&str (namespace), Vec<&PodSummary>>` in one pass; slices with `service: None` or `address_type == "FQDN"` are skipped while indexing them by `(namespace, service)`.

### Services

`ServiceHealth { matching_pods: Option<usize>, endpoints: Option<EndpointCounts> }` (`None` = not loaded or denied). First match wins:

| Case | Status | Endpoints cell |
|---|---|---|
| `ExternalName` | builder status | `Absent` |
| LoadBalancer without address | builder "Address pending" | from slices as below |
| selector set, pods ready, 0 matching pods | Bad "Matches no pods" | Bad `0` |
| slices ready, total 0, selector set | Bad "No endpoints" | Bad `0` |
| slices ready, total 0, selector-less | Warn "No endpoints" (slices are managed by hand) | Warn `0` |
| all ready | Ok "{n} endpoints ready" | Ok `{n}` |
| ready 0 | Bad "0 of {n} endpoints ready" | Bad `0 of {n}` |
| some ready | Warn "{r} of {n} endpoints ready" | Warn `{r} of {n}` |
| slices not loaded or denied | builder status | `Absent` |

Matching: only the pods of the service's namespace (one index lookup); `cluster::Selector::of_labels(&service.selector)` (`None` for a selector-less Service, which matches no pod) is built once per service, then `matches(&pod.labels)` per pod ([cluster-api.md](cluster-api.md) "Selector"; the one label matcher in the project). Counting: decision 7.

### ConfigMaps

`UsedBy { owner: String /* deployment/api */, target: Option<ResourceKey>, ways: BTreeSet<&'static str> /* "env", "env from", "volume" */ }`. Pod refs: `env[].source = ConfigMapKey` → `env`; `env_from[].source = ConfigMap` → `env from`; mounts with `VolumeSource::ConfigMap` or a `Projected` naming it → `volume`. All container kinds count. Owner mapping: decision 9; users sorted by owner text, deduped. Cell: none → `Absent`; else the first owner, plus ` +{n}`. Pods not ready → `Absent`.

### Namespaces

`NamespaceLoad { pods: usize, cpu: CpuAmount, memory: ByteAmount }` (decision 28). Cells: `Text` count, `Quantity { Measure::Cpu.format(cores), nanocores, None }`, `Quantity { Measure::Bytes.format(bytes), bytes, None }`. A namespace outside `scope` (`Named`/`Several` not containing it) or pods not ready → three `Absent`.
