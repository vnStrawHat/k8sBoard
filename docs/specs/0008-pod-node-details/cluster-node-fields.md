# 0008 · Cluster: node fields

[Back to index](README.md) · Step 1 · Module: `node.rs` (+ `node_tests.rs`), `lib.rs`.

## `NodeSummary` additions

```rust
pub struct NodeSummary { /* … existing: name, status, roles, taints, kubelet_version, internal_ip, created_at */
    pub conditions: Vec<NodeCondition>, // API order
    pub addresses: Vec<NodeAddress>,    // API order
    pub system: NodeSystemInfo,
    pub resources: Vec<NodeResource>,   // see ordering below
    pub labels: Vec<String>,            // `key=value`, key order (workload::label_terms)
}

pub struct NodeCondition {
    pub name: String,                   // condition type, e.g. `MemoryPressure`
    pub status: ConditionStatus,
    pub reason: Option<String>,         // empty -> None
    pub message: Option<String>,        // cut like event messages
    pub changed_at: Option<jiff::Timestamp>, // lastTransitionTime
}
pub enum ConditionStatus { True, False, Unknown } // any other text -> Unknown
pub struct NodeAddress { pub kind: String /* InternalIP, ExternalIP, Hostname, … */, pub address: String }
pub struct NodeSystemInfo {             // nodeInfo; absent -> empty strings
    pub operating_system: String, pub architecture: String, pub os_image: String,
    pub kernel_version: String, pub container_runtime: String,
}
pub struct NodeResource { pub name: String, pub capacity: Option<String>, pub allocatable: Option<String> }
```

`ConditionStatus`, `NodeAddress`, `NodeCondition`, `NodeResource`, `NodeSystemInfo` are exported from `lib.rs`. All derive `Clone, Debug, PartialEq, Eq` (`ConditionStatus` also `Copy`).

## Rules

| Field | Rule |
|---|---|
| `conditions` | `lastHeartbeatTime` is **never kept**: the kubelet refreshes it on every status update, and keeping it would send a node snapshot each time (decision 6) |
| `resources` | union of `capacity` and `allocatable` keys, `Quantity.0` as written; order `cpu`, `memory`, `pods`, `ephemeral-storage`, then by name; an entry whose capacity and allocatable are both `"0"` (unused `hugepages-*`) is dropped |
| `system` | `kubelet_version` stays the existing top-level field (the table uses it); `kubeProxyVersion` (deprecated), `machineID`, `systemUUID`, `bootID` are not kept |
| `labels` | node labels are not secret (kubectl shows them); **annotations are never read** |
| `internal_ip` | unchanged (the table column); `addresses` is the full list for the drawer |

Not kept: `spec.podCIDR(s)`, `providerID`, `status.images` (large), `volumesInUse`, `config`, `daemonEndpoints`. The YAML tab (0007) shows them.

## Memory

Nodes are few (UAT 4, large clusters ~1,000); the additions are ~2 KiB per node with labels. No limit is needed.
