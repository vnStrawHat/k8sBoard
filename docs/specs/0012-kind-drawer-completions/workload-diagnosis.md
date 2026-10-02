# 0012 · WHY boxes for kinds

[Back to index](README.md) · Step 3 (Services in step 4a) · Module: `kind_diagnosis.rs` (new, pure; named for every kind because 0013–0015 add non-workload rules) + `kind_diagnosis_tests.rs`; renderer in `kind_drawer.rs`

## API

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KindDiagnosis {
    pub(crate) tone: StatusTone,               // Bad or Warn
    pub(crate) title: String,                  // upper case, e.g. "1 OF 3 NOT READY"
    pub(crate) text: String,
    pub(crate) pod: Option<ResourceKey>,       // the pod the text is about → "Open pod {name} →"
}
pub(crate) struct DiagnosisInputs<'a> {
    pub(crate) pods: &'a [&'a PodSummary],    // owned pods (owns_pod); a Service gets its matching pods
    pub(crate) nodes: &'a [NodeSummary],
    pub(crate) service: Option<ServiceHealth>, // Services only
    pub(crate) now: Timestamp,
}
pub(crate) fn kind_diagnosis(object: &KindObject, inputs: &DiagnosisInputs) -> Option<KindDiagnosis>;
```

- **Unhealthy pod** = the first owned pod (snapshot order) with `pod_diagnosis(pod, None, now)` `Some`; its text is reused as `{diagnosis}`. D1 accepts any such pod.
- **Failing pod** = the first owned pod whose `pod_diagnosis` has tone **Bad**. D3 and S2 use it, so a pod that is only warming up (a running container that is not ready yet, a Warn cause) never raises a box during a normal rollout; a long stall is caught by D1 after the progress deadline.
- Renderer: `Alert::error` for Bad, `Alert::warning` for Warn, `.title(title)`, then a sibling link "Open pod {name} →" that calls `reveal` (the 0008 WHY layout). Placed above the first section, Overview tab only.
- No rule fires while `live.pods` is not Ready (except the condition-only rules marked *).

## Deployments (desired > 0 and not paused; first match wins)

| # | When | Tone · title | Text |
|---|---|---|---|
| D1* | `Progressing` False, reason `ProgressDeadlineExceeded` | Bad · ROLLOUT STALLED | `No progress for {progress_deadline_seconds}s.` + ` Pod {name}: {diagnosis}` when an unhealthy pod exists |
| D2* | `ReplicaFailure` True | Bad · REPLICA FAILURE | `{reason}: {message}` (either part may be missing) |
| D3 | ready < desired and a failing pod exists | Bad when ready 0, else Warn · `{desired − ready} OF {desired} NOT READY` | a container cause: `Pod {name} is {status label}: {diagnosis}`; a pod-level cause (evicted, unschedulable): `Pod {name}: {diagnosis}` |

## DaemonSets (desired > 0)

| # | When | Tone · title | Text |
|---|---|---|---|
| S1 | not-ready owned pods on nodes whose readiness is not Ready (k pods) | Warn · `{k} NODE MISSING` / `{k} NODES MISSING` | `The pod on {node} is not ready because the node is NotReady.`; k > 1: `Pods on {node} and {k − 1} more nodes are not ready because their nodes are NotReady.` |
| S2 | a failing pod on a Ready node, ready < desired | Warn · `{desired − ready} OF {desired} NOT READY` | `Pod {name} on {node}: {diagnosis}` |
| S3* | current < desired | Warn · `{desired − current} NODES WITHOUT A POD` | `{desired} nodes should run a pod; {current} do.` (`1 node` when desired is 1) |
| S4* | misscheduled > 0 | Warn · MISSCHEDULED | `{n} pods run on nodes the DaemonSet no longer targets.` |

## Jobs

| # | When | Tone · title | Text |
|---|---|---|---|
| J1* | status Failed or Failing and the `Failed` (or `FailureTarget`) condition reason is `BackoffLimitExceeded` | Bad · BACKOFF LIMIT REACHED | `{failed} attempts failed.` (`1 attempt`) + ` Last pod exited with code {code} ({reason}).` from the newest failed pod's first main container termination, when known |
| J2* | same, reason `DeadlineExceeded` | Bad · DEADLINE EXCEEDED | `The job ran longer than its active deadline of {active_deadline_seconds}s.` |
| J3* | Failed or Failing with another reason | Bad · JOB FAILED | `{reason}: {message}` |
| J4* | Running and failed > 0 | Warn · `{failed} FAILED ATTEMPTS` | `Retrying; the job fails after {backoff_limit + 1} failed attempts.` (`backoff_limit` default 6) |

## Services (step 4a)

| # | When | Tone · title | Text |
|---|---|---|---|
| V1 | `service_health.matching_pods == Some(0)` | Bad · NO MATCHING PODS | `No pod in {namespace} has the labels {selector joined ", "}.` |
| V2 | endpoints loaded, total > 0, ready 0 | Bad · NO READY ENDPOINTS | `{total} endpoints, none ready.` + ` Pod {name}: {diagnosis}` for the first unhealthy matching pod |

ExternalName and selector-less services never get V1.

## Not covered (stay without a box)

StatefulSets, ReplicaSets, CronJobs, Ingresses (CERTIFICATE is 0016), ConfigMaps, Namespaces (STUCK is 0018). Pod events are not read here, so probe-failure causes appear only in the pod drawer.
