# 0001 · Pods and containers (W4/W4b)

[Back to index](README.md) · Module: `src/pod.rs` · Status logic: [pod-status.md](pod-status.md)

## Query

| Method | API call | Conversion (pure, `pub(crate)`) | Order |
|---|---|---|---|
| `list_pods(scope)` | `Api::<Pod>::all` (`All`) or `Api::<Pod>::namespaced` (`Named`), + `list_all` | `pod_summary(&Pod) -> PodSummary` | by (namespace, name) |

`pod_summary` maps the metadata fields itself. It takes `status`, `ready`, and `restarts` from `pod_display` (see [pod-status.md](pod-status.md)).

## Container rules

| Topic | Rule |
|---|---|
| Kind | An init container with `restartPolicy == Some("Always")` is a `Sidecar`. Any other init container is `Init`. `spec.containers` entries are `Main`. |
| Order | All `spec.initContainers` in spec order (Init and Sidecar interleaved as declared), then `spec.containers` in spec order. The UI groups by kind. |
| Status join | By name: `initContainerStatuses` for init and sidecar containers, `containerStatuses` for main containers. |
| No status entry | `state: NotReported`, `is_ready: false`, `restart_count: 0`, `last_termination: None`. |
| State precedence | `terminated` → `Terminated`, then `running` → `Running`, then `waiting` → `Waiting`. If none is set → `NotReported`. |
| Reasons | Parsed with `StatusReason::from_api`. An empty reason is `None`. |
| Last termination | `lastState.terminated`. A signal of `0` or a missing signal is `None`. |
| Counts | API `i32` → `u32` with negatives clamped to 0. Sums use `saturating_add`. |

## Ready column

`ReadyCount.total` counts **main + sidecar** containers, exactly like kubectl. W4 shows `3/4` for 3 main containers plus 1 sidecar. Classic init containers are excluded. W4b's main-only "Containers 2/3" header is computed by the UI from `containers`.

"Readiness failed" (W1/W4) is a UI label. The crate returns the kubectl status `Running` with ready `0/1`.

## Public API

```rust
pub struct PodSummary {
    pub namespace: String,
    pub name: String,
    pub status: PodStatus,
    pub ready: ReadyCount,
    pub restarts: u32,
    pub node_name: Option<String>,
    pub created_at: Option<jiff::Timestamp>,
    pub containers: Vec<ContainerSummary>, // init/sidecar in spec order, then main
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadyCount { pub ready: u32, pub total: u32 } // Display "ready/total"

pub struct ContainerSummary {
    pub name: String,
    pub kind: ContainerKind,
    pub state: ContainerState,
    pub is_ready: bool,
    pub restart_count: u32,
    pub last_termination: Option<Termination>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerKind { Init, Sidecar, Main }

pub enum ContainerState {
    Waiting { reason: Option<StatusReason> },
    Running { started_at: Option<jiff::Timestamp> },
    Terminated(Termination),
    NotReported, // the kubelet has not reported a status yet
}

pub struct Termination {
    pub reason: Option<StatusReason>,
    pub exit_code: i32,
    pub signal: Option<i32>, // None when absent or zero
    pub started_at: Option<jiff::Timestamp>,
    pub finished_at: Option<jiff::Timestamp>,
}

impl ClusterConnection {
    pub async fn list_pods(&self, scope: NamespaceScope) -> Result<Vec<PodSummary>, ClusterError>;
}
```

## Out of scope

Ephemeral containers, images, ports, probes, resources, env and mounts, pod IP, QoS class, owner references, events. Later specs extend these summaries.
