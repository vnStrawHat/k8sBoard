# 0001 · Namespaces and nodes

[Back to index](README.md) · Modules: `src/namespace.rs`, `src/node.rs`

## Queries

| Method | API call | Conversion (pure, `pub(crate)`) | Order |
|---|---|---|---|
| `list_namespaces()` | `Api::<Namespace>::all` + `list_all` | `namespace_summary(&Namespace) -> NamespaceSummary` | by name |
| `list_nodes()` | `Api::<Node>::all` + `list_all` | `node_summary(&Node) -> NodeSummary` | by name |

## Namespace rules

- `status.phase`: `"Active"` → `Active`, `"Terminating"` → `Terminating`, missing or any other value → `Unknown`.
- `created_at` comes from `metadata.creationTimestamp`.
- `NamespaceScope` is passed **by value** everywhere: 0001 queries and 0002 watch streams. The futures and streams that take it own it.

## Node derivation (W5)

| Field | Rule |
|---|---|
| readiness | Condition `type == "Ready"`: `"True"` → `Ready`, `"False"` → `NotReady`. `"Unknown"`, any other value, or a missing condition → `Unknown`. |
| scheduling | `spec.unschedulable == Some(true)` → `Disabled`, otherwise `Enabled`. The UI renders `Ready · SchedulingDisabled`. |
| roles | kubectl `findNodeRoles`. Take the non-empty suffix of each `node-role.kubernetes.io/<role>` label key, plus the non-empty value of the `kubernetes.io/role` label. Deduplicate and sort. If there are none, return an empty `Vec`. The UI shows "—" and does **not** infer `worker`. |
| taints | `spec.taints` in API order. `Display` is `key=value:effect`, or `key:effect` when the value is `None` or empty. |
| kubelet_version | `status.nodeInfo.kubeletVersion`, or `""` if missing. |
| internal_ip | First `status.addresses` entry with `type == "InternalIP"`, else `None`. |
| created_at | `metadata.creationTimestamp`. |

- CPU and memory columns (W5) need metrics values, which are out of scope.

## Public API

```rust
// namespace.rs
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamespaceScope { All, Named(String) }

pub struct NamespaceSummary {
    pub name: String,
    pub phase: NamespacePhase,
    pub created_at: Option<jiff::Timestamp>,
}

pub enum NamespacePhase { Active, Terminating, Unknown }

impl ClusterConnection {
    pub async fn list_namespaces(&self) -> Result<Vec<NamespaceSummary>, ClusterError>;
}

// node.rs
pub struct NodeSummary {
    pub name: String,
    pub status: NodeStatus,
    pub roles: Vec<String>,     // sorted, deduplicated, possibly empty
    pub taints: Vec<NodeTaint>, // API order
    pub kubelet_version: String,
    pub internal_ip: Option<String>,
    pub created_at: Option<jiff::Timestamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeStatus { pub readiness: NodeReadiness, pub scheduling: NodeScheduling }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeReadiness { Ready, NotReady, Unknown }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeScheduling { Enabled, Disabled }

pub struct NodeTaint {
    pub key: String,
    pub value: Option<String>,
    pub effect: String, // "NoSchedule" | "PreferNoSchedule" | "NoExecute"
}
// impl Display for NodeTaint

impl ClusterConnection {
    pub async fn list_nodes(&self) -> Result<Vec<NodeSummary>, ClusterError>;
}
```
