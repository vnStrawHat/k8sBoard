# 0001 · Files to touch and module layout

[Back to index](README.md)

## Cargo changes

| File | Change |
|---|---|
| `Cargo.toml` (root) | add `jiff = { version = "0.2", default-features = false, features = ["std"] }` to `[workspace.dependencies]` |
| `crates/cluster/Cargo.toml` | add `jiff.workspace = true` |

- `kube`, `k8s-openapi`, `tokio`, `thiserror`, `tracing`, and `futures` are already dependencies of the crate.
- `serde` is reached through `k8s_openapi::serde`.
- No other new dependencies, and no new dev-dependencies.
- The `kube` features must **not** include `ws`. This is acceptance criterion 4.
- `crates/app` is unchanged.

## Module layout (`crates/cluster`)

All modules are private. `lib.rs` contains only the crate doc comment, `mod` declarations, and `pub use` re-exports.

| File | Primary concept | Public items | Spec |
|---|---|---|---|
| `src/lib.rs` | declarations and re-exports | — | this file |
| `src/kubeconfig.rs` | kubeconfig file, contexts, resolution | `Kubeconfig`, `ContextSummary`, `ContextOrigin`, `KubeconfigError` | [kubeconfig.md](kubeconfig.md) |
| `src/kubeconfig_tests.rs` | tests | — | [test-plan.md](test-plan.md) |
| `src/connection.rs` | client, deadlines, paging, error mapping, server version | `ClusterConnection`, `ClusterError`, `ServerVersion` | [connection.md](connection.md) |
| `src/namespace.rs` | namespace summaries, namespace scope | `NamespaceSummary`, `NamespacePhase`, `NamespaceScope` | [namespaces-and-nodes.md](namespaces-and-nodes.md) |
| `src/node.rs` | node summaries (W5) | `NodeSummary`, `NodeStatus`, `NodeReadiness`, `NodeScheduling`, `NodeTaint` | [namespaces-and-nodes.md](namespaces-and-nodes.md) |
| `src/node_tests.rs` | tests | — | [test-plan.md](test-plan.md) |
| `src/pod.rs` | pod and container summaries (W4/W4b) | `PodSummary`, `ContainerSummary`, `ContainerKind`, `ContainerState`, `Termination`, `ReadyCount` | [pods.md](pods.md) |
| `src/pod_tests.rs` | tests | — | [test-plan-pods.md](test-plan-pods.md) |
| `src/pod_status.rs` | kubectl 1.32 status, ready, restarts | `PodStatus`, `InitStatus`, `StatusReason` | [pod-status.md](pod-status.md) |
| `src/pod_status_tests.rs` | tests | — | [test-plan-pods.md](test-plan-pods.md) |
| `src/metrics_api.rs` | `metrics.k8s.io` discovery | `MetricsApi` | [access-and-capabilities.md](access-and-capabilities.md) |
| `src/access_review.rs` | SelfSubjectAccessReview | `AccessCheck`, `AccessDecision`, `AccessReview`, `AccessReport` | [access-and-capabilities.md](access-and-capabilities.md) |
| `tests/connection.rs` | integration test, public API, no cluster | — | [test-plan.md](test-plan.md) |
| `tests/fixtures/kubeconfig.yaml` | fake kubeconfig (fake credentials only) | — | [test-plan.md](test-plan.md) |
| `examples/probe.rs` | read-only CLI probe | — | [probe-example.md](probe-example.md) |

Placement change from the previous single-file draft: `server_version.rs` is folded into `connection.rs`. It was a single small query, and its concept, "talk to the API server", is what `connection.rs` already owns.

## Test-file convention

`connection.rs`, `namespace.rs`, `metrics_api.rs`, and `access_review.rs` keep their tests in an inline `#[cfg(test)] mod tests`. Larger test sets go in sibling files, declared with the comment below:

```rust
// `#[path]` keeps the tests a sibling file; without it they would live in src/kubeconfig/.
#[cfg(test)]
#[path = "kubeconfig_tests.rs"]
mod kubeconfig_tests;
```

## `lib.rs`

```rust
//! Cluster access for k8sBoard: kubeconfig loading, context selection, and
//! read-only access to Kubernetes clusters.

mod access_review;
mod connection;
mod kubeconfig;
mod metrics_api;
mod namespace;
mod node;
mod pod;
mod pod_status;

pub use access_review::{AccessCheck, AccessDecision, AccessReport, AccessReview};
pub use connection::{ClusterConnection, ClusterError, ServerVersion};
pub use kubeconfig::{ContextOrigin, ContextSummary, Kubeconfig, KubeconfigError};
pub use metrics_api::MetricsApi;
pub use namespace::{NamespacePhase, NamespaceScope, NamespaceSummary};
pub use node::{NodeReadiness, NodeScheduling, NodeStatus, NodeSummary, NodeTaint};
pub use pod::{ContainerKind, ContainerState, ContainerSummary, PodSummary, ReadyCount, Termination};
pub use pod_status::{InitStatus, PodStatus, StatusReason};
```

## API-wide rules

- Nothing outside this re-export list is `pub`.
- Every public type is `Send + Sync + 'static`.
- Value types derive `Clone, Debug, PartialEq, Eq` unless a topic file says otherwise.
- No `kube` or `k8s_openapi` type appears in a public signature.
- Timestamps are `jiff::Timestamp`. k8s-openapi 0.28 already uses jiff, and depending on it directly keeps the public API off a re-export path.
- Each query is an `async fn(&self, ..)` in an `impl ClusterConnection` block inside its domain module.
- The kube-to-domain conversion is a pure, per-object function in the same module. `namespace_summary`, `node_summary`, and `pod_summary` are `pub(crate)` for reuse by 0002. All other conversions are private.
