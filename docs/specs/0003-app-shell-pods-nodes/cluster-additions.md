# 0003 · Cluster crate additions

[Back to index](README.md) · Crate: `crates/cluster`. These are the only cluster changes in 0003, limited to fields the drawer displays. The 0001 API-wide rules apply.

## `PodSummary`: new fields (`src/pod.rs`)

```rust
pub struct PodSummary {
    // ...existing 0001 fields...
    pub pod_ip: Option<String>,             // status.podIP, empty -> None
    pub qos_class: Option<String>,          // status.qosClass ("Guaranteed" | "Burstable" | "BestEffort")
    pub service_account: Option<String>,    // spec.serviceAccountName, empty -> None
    pub controller: Option<PodController>,  // the ownerReference with controller == true
    pub conditions: Vec<PodCondition>,      // status.conditions, API order
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodController { pub kind: String, pub name: String }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodCondition {
    pub name: String,   // condition type, e.g. "Ready"
    pub is_true: bool,  // status == "True"; "False"/"Unknown" -> false
}
```

- `qos_class` stays a `String`: the UI only displays it, and an enum adds nothing yet.
- 0002 snapshot diffing compares summaries with `!=`, so condition changes (e.g. `Ready` flipping) now produce snapshots. That is intended.

## `ContainerSummary`: new field

```rust
pub image: String, // spec container image, as written in the spec (no digest resolution)
```


## `lib.rs`

```rust
pub use pod::{ /* existing */, PodCondition, PodController };
```

## Tests (in the existing test files)

| File | Test |
|---|---|
| `pod_tests.rs` | `pod_summary_reads_ip_qos_service_account` |
| `pod_tests.rs` | `pod_summary_controller_is_owner_with_controller_true` (the non-controller owner is ignored) |
| `pod_tests.rs` | `pod_summary_conditions_keep_api_order_and_truth` |
| `pod_tests.rs` | `container_summary_reads_image` |

No new dependencies. The probe example does not change. Each field above is shown by the drawer (Overview: IP, QoS, service account, controller, conditions; Containers: image) and nothing else is added.

Spec 0008 adds the pod, container, event, and node detail fields to these summaries; see `docs/specs/0008-pod-node-details/cluster-pod-fields.md` and `cluster-node-fields.md`.
