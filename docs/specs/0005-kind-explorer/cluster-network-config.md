# 0005 · Cluster crate: Service, Ingress, ConfigMap, access, probe

[Back to index](README.md) · Modules: `service.rs`, `ingress.rs`, `config_map.rs`, `access_review.rs`, `lib.rs`, `examples/probe.rs`. The common fields (`namespace`, `name`, `created_at`, `labels`) are as described in [cluster-workloads.md](cluster-workloads.md).

## Service (`service.rs`, core/v1)

```rust
pub struct ServiceSummary { /* common */ pub service_type: String, pub cluster_ips: Vec<String>, pub is_headless: bool,
    pub external_addresses: Vec<String>, pub ports: Vec<ServicePortSummary>, pub selector: Vec<String> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServicePortSummary { pub name: Option<String>, pub port: u16, pub target_port: Option<String>,
    pub node_port: Option<u16>, pub protocol: String }
impl fmt::Display for ServicePortSummary // kubectl PORT(S): "80/TCP", "80:30080/TCP"
```

- `service_type` is `spec.type`, defaulting to `ClusterIP`.
- `is_headless` is true when `spec.clusterIP == "None"`. `cluster_ips` is `spec.clusterIPs`, else `[clusterIP]`. It is empty when the service is headless or the value is empty.
- `external_addresses`, in order:
  - `status.loadBalancer.ingress[]`, using `ip`, else `hostname`;
  - then `spec.externalIPs`;
  - for an `ExternalName` service, `[spec.externalName]`.
- `target_port` is the `IntOrString` text. The protocol defaults to `TCP`. `selector` holds `k=v` terms in key order.

## Ingress (`ingress.rs`, networking.k8s.io/v1)

```rust
pub struct IngressSummary { /* common */ pub class: Option<String>, pub hosts: Vec<String>, pub addresses: Vec<String>,
    pub rules: Vec<IngressPath>, pub default_backend: Option<String>, pub tls: Vec<IngressTls> }
pub struct IngressPath { pub host: Option<String>, pub path: Option<String>, pub backend: String }
pub struct IngressTls { pub hosts: Vec<String>, pub secret_name: Option<String> }
```

- `class` is `spec.ingressClassName`, else the annotation `kubernetes.io/ingress.class`. That annotation is read once and not kept.
- `hosts` are the rule hosts in order, with duplicates removed. A rule without a host adds nothing. The app shows `*` when the list is empty.
- `rules` has one entry per HTTP path. `backend` reads:
  - a service backend as `name:port`, using the port number or name;
  - a resource backend as `Kind/name`.

  `default_backend` uses the same text.
- `addresses` come from `status.loadBalancer.ingress[]`, using `ip`, else `hostname`. TLS hosts and secret names are names only. The secret is never read.

## ConfigMap (`config_map.rs`, core/v1)

```rust
pub struct ConfigMapSummary { /* common */ pub keys: Vec<ConfigMapKey>, pub is_immutable: bool }
pub struct ConfigMapKey { pub name: String, pub size_bytes: usize, pub is_binary: bool }
```

- `keys` merges `data` (size = UTF-8 length) and `binaryData` (size = decoded length, `is_binary = true`), sorted by name.
- **Values are never copied.** The summarizer only reads `.len()`. The raw object is dropped with the watcher event, as for every kind (0002 decision 3).

## Access checks (`access_review.rs`)

`CheckTarget` gains `group: &'static str`, which `resource_attributes` uses instead of the fixed `""`. Display stays `{verb} {resource}`, so the reason reads "Not permitted: list deployments". `ALL` grows to 19, appending in this order:

| Variant | verb | group | resource | namespaced |
|---|---|---|---|---|
| `ListNamespaces` | list | "" | namespaces | no |
| `ListDeployments` | list | apps | deployments | yes |
| `ListStatefulSets` | list | apps | statefulsets | yes |
| `ListDaemonSets` | list | apps | daemonsets | yes |
| `ListReplicaSets` | list | apps | replicasets | yes |
| `ListJobs` | list | batch | jobs | yes |
| `ListCronJobs` | list | batch | cronjobs | yes |
| `ListServices` | list | "" | services | yes |
| `ListIngresses` | list | networking.k8s.io | ingresses | yes |
| `ListConfigMaps` | list | "" | configmaps | yes |

The review stays all-or-nothing, run concurrently (19 small POSTs). It is still the only POST in the crate.

## Watch methods

`watch_services`, `watch_ingresses`, and `watch_config_maps` take a `scope` and have the same shape as in [cluster-workloads.md](cluster-workloads.md). `watch_namespaces` is unchanged.

## `lib.rs`

```rust
mod config_map; mod cron_job; mod daemon_set; mod deployment; mod ingress; mod job; mod replica_set;
mod service; mod stateful_set; mod workload;
pub use config_map::{ConfigMapKey, ConfigMapSummary};
pub use daemon_set::DaemonSetSummary;
pub use deployment::DeploymentSummary;
pub use ingress::{IngressPath, IngressSummary, IngressTls};
pub use cron_job::CronJobSummary;
pub use job::{JobStatus, JobSummary};
pub use replica_set::ReplicaSetSummary;
pub use service::{ServicePortSummary, ServiceSummary};
pub use stateful_set::{ClaimTemplate, StatefulSetSummary};
pub use workload::{ContainerPort, ControllerRef, TemplateContainer, WorkloadCondition};
// pod: PodController removed from the export list
```

## Probe (`examples/probe.rs`)

- `--watch-seconds <n>` now runs all 12 watches together: pods, nodes, namespaces, plus the 9 new kinds in the scope. It prints one line per kind, in that order.
- Implementation: each stream maps to `(kind_name, Tally)`, where `Tally` is `Snapshot(len)` or `Failed(text)`. The streams merge with `futures::stream::select_all`, so `watch_for` has no 12-arm `select!`.
- A denied kind prints `0 snapshots … N failures; last error: … 403 …`. That is information, not a probe failure: `all_succeeded` stays false only for a kind that was silent (as in 0002).
- The access section already prints every `AccessCheck`, so the 10 new lines appear with no change.
- Update `USAGE`, the doc comment, and one row in [0001 probe-example.md](../0001-cluster-read-only/probe-example.md).
