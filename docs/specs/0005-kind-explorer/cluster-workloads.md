# 0005 · Cluster crate: workload summaries

[Back to index](README.md) · Modules: `workload.rs` (shared), `deployment.rs`, `stateful_set.rs`, `daemon_set.rs`, `replica_set.rs`, `job.rs`, `cron_job.rs`. The 0001 and 0002 API rules apply: no kube types in public signatures, and summaries only.

## Shared pieces (`src/workload.rs`)

```rust
#[derive(Clone, Debug, PartialEq, Eq)] pub struct ControllerRef { pub kind: String, pub name: String } // was pod::PodController
#[derive(Clone, Debug, PartialEq, Eq)] pub struct WorkloadCondition { pub name: String, pub is_true: bool, pub reason: Option<String> }
#[derive(Clone, Debug, PartialEq, Eq)] pub struct TemplateContainer { pub name: String, pub image: String, pub ports: Vec<ContainerPort> }
#[derive(Clone, Debug, PartialEq, Eq)] pub struct ContainerPort { pub name: Option<String>, pub port: u16, pub protocol: String }

pub(crate) fn controller_ref(metadata: &ObjectMeta) -> Option<ControllerRef>; // owner with controller == true
pub(crate) fn label_terms(metadata: &ObjectMeta) -> Vec<String>;             // "key=value", key order
pub(crate) fn selector_terms(selector: &LabelSelector) -> Vec<String>;
pub(crate) fn template_containers(template: &PodTemplateSpec) -> Vec<TemplateContainer>; // main containers, spec order
pub(crate) fn condition(name: &str, status: &str, reason: Option<&str>) -> WorkloadCondition;
pub(crate) fn int_or_string_text(value: &IntOrString) -> String;            // 25% | 1
```

- `pod.rs` uses `controller_ref(&pod.metadata)`, and `PodSummary.controller` becomes `Option<ControllerRef>`.
- `selector_terms` uses kubectl's selector syntax: `matchLabels` gives `k=v`, then the expressions in order: `In` gives `k in (a,b)`, `NotIn` gives `k notin (a,b)`, `Exists` gives `k`, `DoesNotExist` gives `!k`.
- **Template containers keep `name`, `image`, and `ports` only.** `env`, `envFrom`, `command`, `args`, and `volumeMounts` are never copied, because they can hold plaintext secrets.
- `ContainerPort.protocol` defaults to `TCP` when absent. A port outside `u16` is dropped (the API rejects such ports anyway).
- An empty `reason` becomes `None`, using the existing `non_empty` rule. All counts go through `pod_status::non_negative`, with `None` read as 0.
- Every summary below also has `namespace: String`, `name: String`, `created_at: Option<jiff::Timestamp>`, and `labels: Vec<String>`. These four are not repeated.

## Deployment (`deployment.rs`, `apps/v1`)

```rust
pub struct DeploymentSummary { /* common */ pub desired: u32, pub ready: u32, pub up_to_date: u32, pub available: u32,
    pub strategy: String, pub max_surge: Option<String>, pub max_unavailable: Option<String>, pub is_paused: bool,
    pub revision: Option<String>, pub selector: Vec<String>, pub containers: Vec<TemplateContainer>,
    pub conditions: Vec<WorkloadCondition> }
```

- `desired = spec.replicas`, defaulting to 1, as the API server does. `ready`, `up_to_date`, and `available` come from `status.{readyReplicas, updatedReplicas, availableReplicas}`.
- `strategy` is `spec.strategy.type`, or "" when absent. `max_surge` and `max_unavailable` come from `rollingUpdate`.
- `revision` is the annotation `deployment.kubernetes.io/revision`. It is read once in the summarizer; other annotations are never kept.

## StatefulSet (`stateful_set.rs`)

```rust
pub struct StatefulSetSummary { /* common */ pub desired: u32, pub ready: u32, pub current: u32, pub updated: u32,
    pub service_name: Option<String>, pub update_strategy: String, pub pod_management_policy: String,
    pub selector: Vec<String>, pub containers: Vec<TemplateContainer>, pub claim_templates: Vec<ClaimTemplate> }
pub struct ClaimTemplate { pub name: String, pub storage: Option<String>, pub storage_class: Option<String>, pub access_modes: Vec<String> }
```

- `service_name` is `spec.serviceName` (optional in v1_32), with "" read as `None`. `storage` is `spec.resources.requests["storage"]`, as the quantity text.

## DaemonSet (`daemon_set.rs`)

```rust
pub struct DaemonSetSummary { /* common */ pub desired: u32, pub current: u32, pub ready: u32, pub up_to_date: u32,
    pub available: u32, pub misscheduled: u32, pub node_selector: Vec<String>, pub update_strategy: String,
    pub selector: Vec<String>, pub containers: Vec<TemplateContainer> } // no conditions: the DaemonSet controller never writes any
```

- The counts are `desiredNumberScheduled`, `currentNumberScheduled`, `numberReady`, `updatedNumberScheduled`, `numberAvailable`, and `numberMisscheduled`.
- `node_selector` is `template.spec.nodeSelector` as `k=v` terms.

## ReplicaSet (`replica_set.rs`)

```rust
pub struct ReplicaSetSummary { /* common */ pub desired: u32, pub current: u32, pub ready: u32,
    pub owner: Option<ControllerRef>, pub revision: Option<String>, pub selector: Vec<String>,
    pub containers: Vec<TemplateContainer> }
```

`desired = spec.replicas`, defaulting to 1. `current = status.replicas`. `revision` uses the same annotation as Deployment.

## Job (`job.rs`) and CronJob (`cron_job.rs`), `batch/v1`

```rust
pub struct JobSummary { /* common */ pub status: JobStatus, pub completions: Option<u32>, pub parallelism: Option<u32>,
    pub succeeded: u32, pub failed: u32, pub active: u32, pub backoff_limit: Option<u32>,
    pub started_at: Option<jiff::Timestamp>, pub finished_at: Option<jiff::Timestamp>,
    pub owner: Option<ControllerRef>, pub conditions: Vec<WorkloadCondition>, pub containers: Vec<TemplateContainer> }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus { Running, Complete, Failed, Failing, Suspended } // + Display: "Running" … "Failing"

pub struct CronJobSummary { /* common */ pub schedule: String, pub time_zone: Option<String>, pub is_suspended: bool,
    pub concurrency_policy: String, pub starting_deadline_seconds: Option<i64>,
    pub successful_history_limit: Option<u32>, pub failed_history_limit: Option<u32>,
    pub active_jobs: Vec<String>, pub last_schedule_at: Option<jiff::Timestamp>,
    pub last_success_at: Option<jiff::Timestamp>, pub containers: Vec<TemplateContainer> }
```

- `JobStatus` (the private `fn job_status`) is the first true condition, in kubectl 1.32 `printJob` order (`Terminating` is not modelled):
  1. `Complete` gives Complete;
  2. `Failed` gives Failed;
  3. `Suspended` gives Suspended;
  4. `FailureTarget` gives Failing;
  5. otherwise, Running.
- `finished_at` is `status.completionTime`, else the `lastTransitionTime` of the true `Failed` condition. This keeps a failed job's duration from growing forever.
- `CronJobSummary.active_jobs` holds the names in `status.active[]`. `containers` come from `spec.jobTemplate.spec.template`.

## Watch methods (one per kind, in its module)

```rust
impl ClusterConnection {
    pub fn watch_deployments(&self, scope: NamespaceScope)  -> impl Stream<Item = WatchUpdate<DeploymentSummary>> + Send + 'static;
    // same shape: watch_stateful_sets, watch_daemon_sets, watch_replica_sets, watch_jobs, watch_cron_jobs
}
// connection.rs, replaces pod.rs `pods_api`
pub(crate) fn scoped_api<K>(&self, scope: NamespaceScope) -> Api<K>
where K: kube::Resource<Scope = NamespaceResourceScope>, K::DynamicType: Default;
```

Each method is `summary_watch(self, self.scoped_api(scope), "watching deployments", deployment_summary)`, with `stateful sets`, `daemon sets`, `replica sets`, `jobs`, or `cron jobs` in the action text. Nothing else in `resource_watch.rs` changes.
