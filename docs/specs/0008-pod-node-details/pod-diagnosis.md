# 0008 · App: WHY rules, probe results, next retry

[Back to index](README.md) · Step 2 · Module: `pod_diagnosis.rs` (new, pure, no GPUI types except `StatusTone`), tests in `pod_diagnosis_tests.rs`.

`events` is the open pod's object events list (0006, newest first): `Some(items)` when Ready, `None` while loading or failed. The 0006 watch starts 250 ms after the selection rests, so WHY text and probe results that depend on events appear after that debounce and the first list; until then they use the flag-only rows below.

```rust
pub(crate) struct PodDiagnosis { pub(crate) tone: StatusTone /* Bad | Warn */,
    pub(crate) container: Option<String> /* None = pod-level */, pub(crate) text: String }
pub(crate) fn pod_diagnosis(pod: &PodSummary, events: Option<&[EventSummary]>, now: Timestamp) -> Option<PodDiagnosis>;

#[derive(Clone, Copy)] pub(crate) enum ProbeKind { Liveness, Readiness, Startup }
pub(crate) enum ProbeResult { NotSet, Inactive, WaitingForStartup, Pending, Passed, Passing,
    Failing { failures: u32 }, NoData }
pub(crate) fn probe_result(pod: &PodSummary, container: &ContainerSummary, kind: ProbeKind,
    events: Option<&[EventSummary]>) -> ProbeResult;
/// The newest probe failure event of `kind` for this container since it last started.
fn probe_failure<'a>(container: &ContainerSummary, kind: ProbeKind, events: &'a [EventSummary]) -> Option<&'a EventSummary>;
/// CrashLoopBackOff only: time left until the kubelet retries.
pub(crate) fn next_retry(container: &ContainerSummary, now: Timestamp) -> Option<jiff::SignedDuration>;
```

## Probe failure events

An event matches when `reason == "Unhealthy"`, `container == Some(name)`, the message starts with `"{Kind} probe"` (`Liveness`/`Readiness`/`Startup`; covers "failed" and "errored"), and `last_seen >= started_at` of the Running state (no `started_at` → every match). `failures` = the **newest** match's `count` only (one event series; never summed across events).

## `probe_result` (first matching row)

| Probe | Pod / container | Result (text, tone) |
|---|---|---|
| None | any | `NotSet` ("not set", muted) |
| set | pod `Terminating`, or container not Running | `Inactive` ("—", muted) |
| Liveness, Readiness | a startup probe is set and `is_started == Some(false)` | `WaitingForStartup` ("Waiting for startup", muted): the kubelet runs them only after startup passes |
| Readiness | `is_ready` | `Passing` (Ok) |
| Readiness | not ready | `Failing { failures }` ("Failing", or "Failing · ×N" when a match exists; Bad) |
| Startup | `is_started == Some(true)` | `Passed` (Ok) |
| Startup | otherwise | `Failing` when a match exists (Bad), else `Pending` ("Not passed yet", Warn) |
| Liveness | `events` is None | `NoData` ("No data", muted) |
| Liveness | a match exists | `Failing { failures }` (Bad) |
| Liveness | no match | `Passing` (Ok) |

Deviation: W4b shows "Failing · 3/3" (consecutive failures / threshold). The API has no consecutive count, so the closest honest shape is "Failing · ×N" (failures in the newest event series).

## `pod_diagnosis` rules (first match; `None` = no WHY box)

Pod-level, in order:

| # | When | Tone | Text |
|---|---|---|---|
| P0 | status `Terminating`, or Reason `Succeeded`/`Completed` | — | `None` |
| P1 | condition `PodScheduled` not true, reason `Unschedulable` | Bad | `Cannot be scheduled: {message}` (no message: `Cannot be scheduled.`) |
| P2 | status Reason `SchedulingGated` | Warn | `Waiting for its scheduling gates to be removed.` |
| P3 | `status_message` is Some (the crate keeps it only for phase Failed or reason Evicted) | Bad | `{pod status}: {status_message}` (e.g. `Evicted: The node was low on resource: memory.`) |

Otherwise every container gets `container_problem` (below). The box shows the first **Bad** problem in container order (init and sidecars, then main), else the first **Warn** one. Suffix (W4 shows it; joined to the text with a full stop added when the text does not already end in `.`, `!`, or `?`): ` Other containers are healthy.` when the pod has 2+ containers and one problem; ` {n} other containers also have problems.` (`1 other container also has a problem.`) when more.

C1 and C5 decide on `status_tone::is_bad_reason` (now `pub(crate)`, with the new `InvalidImageName`, `ErrImageNeverPull`, `CreateContainerError` variants), so the WHY box and the state label never disagree. No rule string-matches `StatusReason::Other`.

| # | Container | Tone | Text |
|---|---|---|---|
| C1 | Waiting `ImagePullBackOff`, `ErrImagePull`, `InvalidImageName`, or `ErrImageNeverPull` | Bad | `Cannot pull image {image}: {message}` (no message: `: {reason}`) |
| C2 | Waiting `CrashLoopBackOff`, last termination `OOMKilled` | Bad | `OOMKilled (exit {code}) about {run} after each start: memory hits the {limit} limit.` (W4 wording); no limit: `… after each start. No memory limit is set.` |
| C3 | Waiting `CrashLoopBackOff`, other last termination | Bad | `Exits with {reason} (exit {code}) about {run} after each start. Restarted {n} times.` (no reason: `Exits with code {code} …`) |
| C4 | Waiting `CrashLoopBackOff`, no last termination | Bad | `CrashLoopBackOff: {message}` |
| C5 | Waiting, any other reason where `is_bad_reason` is true | Bad | `{reason}: {message}` or `{reason}.` (CreateContainerConfigError, CreateContainerError, …) |
| C6 | Terminated, exit ≠ 0 | Bad | `Exited with {reason} (exit {code}).` + ` Memory limit {limit}.` when OOMKilled and a limit is set |
| C7 | Running, a startup probe is set, `is_started == Some(false)` | Warn | `Startup probe has not passed yet.` + ` {event message} (×N, {age} ago)` for a startup match |
| C8 | Running, not ready, kind Main or Sidecar | Warn | `Running but not ready.` + ` {event message} (×N, {age} ago)` for a readiness match, else ` The readiness probe ({probe text}) has not passed.` when one is set |

- `{run}` = `format_age(started_at, finished_at)` of the termination; when either is missing, ` about {run} after each start` becomes ` on each start`.
- `{limit}` = the `memory` limit from `resources`. `{n}` = `restart_count`; `N` = newest match's `count`; `{age}` = `format_age(last_seen, now)`; `{probe text}` is from [pod-drawer.md](pod-drawer.md).
- Never problems: exit-0 terminated init containers, Waiting with a reason that is not bad (`ContainerCreating`, `PodInitializing`, unknown text), `NotReported` (ContainerCreating stuck on a mount: open item 2).
- The termination message is not kept (decision 8), so C2/C3/C6 name the reason and exit code, not the app's last words.

## `next_retry` (W4b header "next retry in 3m20s")

The kubelet's CrashLoopBackOff waiting message is `back-off %s restarting failed container=…` (verified), and the back-off is measured from `lastState.terminated.finishedAt`. So: Waiting `CrashLoopBackOff`, message contains `back-off {go duration}`, and a last termination `finished_at` → `finished_at + duration - now` when positive, else None. The parser accepts integer `h`, `m`, `s` parts (`5m0s`, `40s`, `1h0m0s`); anything else → None.

## Pull secrets (walk H11)

For a failed pull (ImagePullBackOff or ErrImagePull) the drawer appends `Pull secrets: x (missing), y` to the WHY text (`PodDiagnosis::with_pull_secrets`) and links each existing secret under the box. A name is `(missing)` only when the Secrets screen has the namespace's list loaded and lacks it; otherwise nothing is called missing. The Overview Pod section lists `Image pull secrets` as links when the pod has any. The Issues and kind diagnoses keep the short text.
