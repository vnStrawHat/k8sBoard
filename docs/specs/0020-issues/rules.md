# 0020 · Rules: pods, nodes, volume usage, events

[Back to index](README.md) · Step 1a · Module: `issue_rules.rs` (+ `issue_rules_tests.rs`). `IssueRule` variants follow the order in [kind-rules.md](kind-rules.md#order-inside-evaluate). Grace is applied by the board (decision 20), never inside a rule.

## `pod_diagnosis` change (`pod_diagnosis.rs`)

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DiagnosisCause {
    Unschedulable { since: Option<Timestamp> }, SchedulingGated, PodFailed,          // P1, P2, P3
    ImagePull(StatusReason), CrashLoop, Waiting(StatusReason),                         // C1, C2–C4, C5
    Exited { reason: Option<StatusReason> }, StartupPending, NotReady,                 // C6, C7, C8
}
pub(crate) struct PodDiagnosis { /* tone, container, text */ pub(crate) cause: DiagnosisCause }
```

`Unschedulable.since` = the `PodScheduled` condition's new `changed_at`. Box rendering ignores `cause`. `ready_since` below = the pod `Ready` condition's `changed_at` when it is false.

## Pods (per pod, first match; `events` = the pod's slice of the Warning feed)

Skipped: pods `pod_diagnosis` P0 skips; Job-owned pods for PodExited and PodFailed **only while the Jobs feed is Live** (`is_job_feed_live`; the Job rule then owns them).

| Rule | When | Severity | Reason (pill) | Cause | Onset | Grace | Action |
|---|---|---|---|---|---|---|---|
| PodImage | cause `ImagePull(r)` | Critical | `r` | diagnosis text | — | — | Open |
| PodCrash | `CrashLoop` | Critical | `CrashLoopBackOff` | diagnosis text | `ready_since` | — | ViewLogs(container) |
| PodWaiting | `Waiting(r)` | Critical | `r` | diagnosis text | — | — | Open |
| PodUnschedulable | `Unschedulable` | Warning | `Pending` | diagnosis text | `since`, else `created_at` | `UNSCHEDULABLE_GRACE` | Open |
| PodFailed | `PodFailed`; when the status is `Evicted`, only if `ready_since` is within `EVICTION_WINDOW` | Warning | pod status text (`Evicted`) | diagnosis text | `ready_since` | — | Open |
| PodExited | `Exited` | Warning | reason or `Error` | diagnosis text | `ready_since` | — | ViewLogs(container) |
| PodStartup | `StartupPending` | Warning | `Startup probe` | diagnosis text | container `started_at` | startup probe `period_seconds × failure_threshold`, else `STARTUP_FALLBACK_GRACE` | Open |
| PodNotReady | `NotReady` | Warning | `Not ready` | diagnosis text | `ready_since` | `NOT_READY_GRACE` | Open |
| PodStuck | no diagnosis; status `ContainerCreating`, `PodInitializing`, `Pending`, or `Init:` progress | Warning | status text | `Stuck in {status} for {age}.` + ` {reason}: {message line}` of the pod's newest Warning event | `created_at` | `STUCK_STARTING_AFTER` | Open |
| PodRestarts | a Running container whose `last_termination.finished_at` is within `RESTART_WINDOW`: reason OOMKilled, or `restart_count ≥ RESTART_WARN` | Warning | `OOMKilled` / `Restarting` | OOM: `OOMKilled {age} ago; memory limit {limit}.` (no limit: `No memory limit is set.`); else `Restarted {n} times in total; last exit {reason or "code {code}"} {age} ago.` | `finished_at` | — | ViewLogs(container) |
| PodMemory | a container's memory ≥ `USAGE_WARN_RATIO` × its parsed memory limit in **both** of the last 2 samples | Warning | `Near memory limit` | `Container {c} uses {used} of its {limit} memory limit.` (`Measure::format_pair`, newest sample) | — | — | Open |
| PodCpu | the newest CPU sample ≥ `USAGE_WARN_RATIO` × its CPU limit | Warning | `At CPU limit` | `Container {c} uses {used} of its {limit} CPU limit.` | — | — | Open |

- `diagnosis` = `pod_diagnosis(pod, events, now)` with `events` = `WarningEvents::of(Pod, ns, name)` (`Some(&[])` when Ready and empty; `None` when not Ready).
- A held finding (grace not reached) still is the pod's first match: no other pod rule fires for it meanwhile.
- `workload` = `kind_row::pod_workload(namespace, controller)`: Deployment for a ReplicaSet named `{d}-{hash}` (0005 alphabet), else the controller kind and name, else `None`.
- `container` = the diagnosis container, or the matched container for PodRestarts, PodMemory, PodCpu.
- Usage: `PodUsageHistory::latest_container_pair` (new: the last two fine ticks, `None` unless both exist) and `latest_container`; limits parse with `ByteAmount::parse` / `CpuAmount::parse`; no limit → no finding. PodCpu does not claim throttling: CFS throttling is not measured.
- `ponytail:` PodRestarts looks back 1 h from the last exit only, because `restart_count` is a lifetime total and there is no restart history; upgrade path: count `last_termination` changes per tick in the metrics history.

## Nodes (per node, first match; action Open)

| Rule | When | Severity | Reason | Cause | Onset | Grace |
|---|---|---|---|---|---|---|
| NodeNotReady | `Ready` False, or Unknown | Critical | `NotReady` / `Unknown` | `Ready is {status} for {age}.` + ` {message}` | `changed_at` | Unknown: `NODE_UNKNOWN_GRACE` |
| NodeNetwork | `NetworkUnavailable` True | Critical | `NetworkUnavailable` | message, or `The node network is not configured.` | `changed_at` | — |
| NodePressure | `MemoryPressure`, `DiskPressure`, or `PIDPressure` True (first in that order) | Warning | condition name | message + ` Also {others}.` when more are True | `changed_at` | — |
| NodeCondition | any other condition with `node_condition_tone` Bad (node-problem-detector) | Warning | condition name | `{reason}: {message}` | `changed_at` | — |
| NodeMemory / NodeCpu | `node_usage` memory / cpu ratio ≥ `USAGE_WARN_RATIO` | Warning | `High memory` / `High CPU` | `Uses {pct} of allocatable {resource}.` | — | — |

Cordoned nodes are not issues (decision 23).

## Volume usage (kubelet, 0011)

| Rule | When | Severity | Reason | Cause |
|---|---|---|---|---|
| VolumeFull | a `PvcUsage` in scope with `used / capacity ≥ USAGE_WARN_RATIO` | Warning | `Volume almost full` | `{used} of {capacity} used ({pct}).` |

Object = the PVC. Needs `KubeletHistory::pvc_usages()` (new). The 0011 node cap makes this feed **Limited**, not partial (decision 6).

## Events (Warning feed; `last_seen` within `EVENT_WINDOW`; one finding per involved object, the highest `count`)

| Rule | When | Severity | Reason | Cause |
|---|---|---|---|---|
| EventFailedCreate | reason `FailedCreate` on ReplicaSet, StatefulSet, DaemonSet, Job | Critical | `FailedCreate` | message line |
| EventJobFailed | `BackoffLimitExceeded`, `DeadlineExceeded` on Job | Warning | reason | message line |
| EventBurst | any other reason except `FailedScheduling`, `count ≥ WARNING_BURST`, **and** `first_seen` within `EVENT_WINDOW` | Warning | reason | `{message line} (×{count} in {age}).` |

Object normalization: ReplicaSet `{d}-{hash}` → Deployment `d`. Onset = `first_seen`. `cause` messages are cut at 200 chars. EventBurst's `first_seen` bound keeps a series that fired 10 times over days from looking like a burst.
