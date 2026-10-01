# 0001 · Pod status (kubectl 1.32 parity)

[Back to index](README.md) · Module: `src/pod_status.rs`

The private function `pod_display(pod: &Pod) -> PodDisplay { status, ready, restarts }` ports **kubectl 1.32 `printPod`** and computes all three values in one pass.

- The UAT cluster runs 1.29, but this status text is computed client-side, so 1.32 semantics apply.
- Example: the `Terminating` guard for terminal phases has been in kubectl since 1.30.
- **One deliberate divergence:** a missing phase becomes `Unknown`; kubectl prints an empty string.

## Algorithm

```text
phase  = status.phase (missing -> "Unknown"  [deliberate divergence])
reason = status.reason if non-empty, else phase
if a condition has type "PodScheduled" and reason "SchedulingGated": reason = "SchedulingGated"
total  = len(spec.containers) + count(spec.initContainers where restartPolicy == "Always")
ready = restarts = sidecar_restarts = 0; initializing = false

for (i, c) in status.initContainerStatuses:
    restarts += c.restartCount
    is_sidecar = spec init container named c.name has restartPolicy "Always"
    if is_sidecar: sidecar_restarts += c.restartCount
    if c.state.terminated and exitCode == 0: continue
    if is_sidecar and c.started == Some(true): (if c.ready: ready += 1); continue
    if c.state.terminated:
        reason = "Init:"+reason if non-empty, else "Init:Signal:<n>" if signal != 0, else "Init:ExitCode:<n>"
    elif c.state.waiting and reason non-empty and != "PodInitializing":
        reason = "Init:" + waiting.reason
    else:
        reason = "Init:<i>/<len(spec.initContainers)>"
    initializing = true; break

if not initializing or condition "Initialized" == "True":
    restarts = sidecar_restarts; has_running = false
    for c in status.containerStatuses REVERSED:      # first container in spec order wins
        restarts += c.restartCount
        if   c.state.waiting and reason non-empty:    reason = waiting.reason
        elif c.state.terminated and reason non-empty: reason = terminated.reason
        elif c.state.terminated:                      reason = "Signal:<n>" if signal != 0 else "ExitCode:<n>"
        elif c.ready and c.state.running:             has_running = true; ready += 1
    if reason == "Completed" and has_running:
        reason = "Running" if condition "Ready" == "True" else "NotReady"

if metadata.deletionTimestamp is set:
    if status.reason == "NodeLost":            reason = "Unknown"
    elif phase not in {"Failed", "Succeeded"}: reason = "Terminating"
```

Build the typed value directly at each assignment. The text above defines the semantics only; do not format a string and parse it back.

## Mapping to `PodStatus`

| kubectl text | Typed value |
|---|---|
| `Init:<i>/<n>` | `Init(InitStatus::Progress { first_incomplete: i, total: n })` |
| `Init:Signal:<n>` | `Init(InitStatus::Reason(StatusReason::Signal(n)))` |
| `Init:ExitCode:<n>` | `Init(InitStatus::Reason(StatusReason::ExitCode(n)))` |
| `Init:<other>` | `Init(InitStatus::Reason(StatusReason::from_api(other)))` |
| `Signal:<n>` / `ExitCode:<n>` | `Reason(StatusReason::Signal(n))` / `Reason(StatusReason::ExitCode(n))` |
| `NotReady` | `NotReady` |
| `Terminating` | `Terminating` |
| anything else | `Reason(StatusReason::from_api(text))` |

`first_incomplete` is the index in `initContainerStatuses` of the first init container that is not done. It is a position, not a count of completed classic init containers: started sidecars and init containers that exited 0 before it are passed over. `total` is `len(spec.initContainers)`, sidecars included.

## Public API

```rust
pub enum PodStatus {        // Display = kubectl STATUS text
    Reason(StatusReason),   // phase, pod reason, or the first failing main container's reason
    Init(InitStatus),       // "Init:..."
    NotReady,               // a completed container plus a running one, pod not Ready
    Terminating,            // deletion requested, phase not terminal
}

pub enum InitStatus {
    Progress { first_incomplete: u32, total: u32 }, // "Init:<first_incomplete>/<total>"
    Reason(StatusReason),                           // "Init:<reason>"
}

pub enum StatusReason {
    Running, Pending, Succeeded, Failed, Unknown, Completed,
    ContainerCreating, PodInitializing, CrashLoopBackOff, ImagePullBackOff, ErrImagePull,
    CreateContainerConfigError,
    OomKilled, // API text "OOMKilled"
    Error, ContainerCannotRun, Evicted, SchedulingGated,
    Signal(i32),   // "Signal:<n>"
    ExitCode(i32), // "ExitCode:<n>"
    Other(String), // any other text, verbatim
}

impl StatusReason {
    /// Exact, case-sensitive match; unknown text -> Other. Never yields Signal/ExitCode.
    pub(crate) fn from_api(text: &str) -> Self;
}
```
