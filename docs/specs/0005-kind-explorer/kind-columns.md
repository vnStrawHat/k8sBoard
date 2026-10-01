# 0005 · Columns, cells, and status per kind

[Back to index](README.md) · Modules: `resource_kind.rs` (columns), `*_rows.rs` (cells and status), `kind_table.rs`

## Common rules

- **Name** is always column 0 and flexible: `flexible_width(table_width, sum(fixed), px(200.))`. For namespaced kinds it is rendered like the pod Name cell, with `{namespace}/` muted and the name mono, ellipsized, with a tooltip.
- **Owner format** everywhere (cells and drawer fields): `{kind lowercased}/{name}`, e.g. `deployment/api`, `cronjob/reconcile`.
- **Age** is always last: 70 px, right-aligned, `KindCell::Age { at: created_at, tone: None }`.
- Numbers use `Text`, right-aligned through `Align::Right`. IPs, ports, schedules, and images use `Mono`. Absent values use `Absent`, a muted "—".
- **Replica tone** `fn replica_tone(ready: u32, desired: u32) -> StatusTone`:

  | Case | Tone |
  |---|---|
  | desired 0 | `Done` |
  | ready ≥ desired | `Ok` |
  | ready 0 | `Bad` |
  | otherwise | `Warn` |

  A "Ready" cell is `Toned(StatusLabel { text: "{ready}/{desired}", tone })`.

## Columns (widths in px; `r` = right-aligned)

| Kind | Columns after Name | Deviation from W7 |
|---|---|---|
| Namespaces | Status 140 · Age | no Pods or requests columns (need metrics) |
| Deployments | Ready 80 · Up-to-date 100 r · Available 90 r · Strategy 130 · Age | — |
| StatefulSets | Ready 80 · Service 200 · Update strategy 140 · Age | — |
| DaemonSets | Desired 80 r · Current 80 r · Ready 80 r · Up-to-date 100 r · Available 90 r · Node selector 200 · Age | adds kubectl's Available |
| ReplicaSets | Desired 80 r · Current 80 r · Ready 80 r · Owner 220 · Age | — |
| Jobs | Status 120 · Completions 110 · Duration 90 r · Age | follows kubectl 1.31+ order (Status first) rather than W7 (Completions, Duration, Status) |
| CronJobs | Schedule 140 · Suspend 80 · Active 70 r · Last schedule 120 r · Age | kubectl's Last schedule instead of Last run and Next run |
| Services | Type 130 · Cluster IP 140 · External IP 200 · Ports 180 · Age | kubectl columns; no Endpoints (needs EndpointSlices) |
| Ingresses | Class 100 · Hosts 260 · Address 180 · Ports 80 · Age | kubectl columns; no TLS expiry (needs the Secret) |
| ConfigMaps | Data 70 r · Age | kubectl `DATA`; no Used by |

## Cells

| Kind | Cell rules |
|---|---|
| Namespaces | Status toned: Active is Ok, Terminating is Info, Unknown is Warn |
| Deployments | Ready toned `{ready}/{desired}`. Strategy is `Text`, or `Absent` when empty |
| StatefulSets | Ready toned. Service `Mono`, or `Absent`. Update strategy `Text` |
| DaemonSets | Ready is a toned number, `replica_tone(ready, desired)`. Node selector terms joined with `, `, or `Absent` |
| ReplicaSets | Ready toned. Owner `{kind lowercased}/{name}` (kubectl style, e.g. `deployment/api`; the same format in every drawer Owner field), or `Absent` |
| Jobs | Status toned (below). Completions are `{succeeded}/{completions}`. When `completions` is `None`: `{succeeded}/1 of {parallelism}` if parallelism > 1, else `{succeeded}/1` (kubectl). Duration is `KindCell::Duration` |
| CronJobs | Schedule `Mono`; Suspend `Yes` toned `Done`, or `No`; Active = `active_jobs.len()`; Last schedule = `Age { at: last_schedule_at, tone: last_run_tone }` |
| Services | Type as-is. Cluster IP: `None` (muted `Text`) when headless, else `cluster_ips` joined `,`, or `Absent`. External IP: `<pending>` (`Toned` Warn) for a LoadBalancer with no address, else the addresses joined `,`, or `Absent`. Ports: `Display` joined `,` |
| Ingresses | Class or `Absent`. Hosts joined `,`, or `*`. Address joined `,`, or `Absent`. Ports `80`, or `80, 443` when `tls` is non-empty |
| ConfigMaps | Data = `keys.len()` |

## Status label (drawer subtitle, `KindRow.status`)

| Kind | Rule, first match wins |
|---|---|
| Deployments | desired 0: Done "Scaled to zero". Condition `Progressing` false with reason `ProgressDeadlineExceeded`: Bad "Progress deadline exceeded". Paused: Info "Paused". Otherwise `replica_tone(available, desired)` with "{available}/{desired} available" |
| StatefulSets, ReplicaSets | `replica_tone(ready, desired)` with "{ready}/{desired} ready"; desired 0 reads "Scaled to zero" |
| DaemonSets | desired 0: Done "No nodes scheduled"; else `replica_tone(ready, desired)` with "{ready}/{desired} ready" |
| Jobs | Complete is Ok, Running is Info, Failed and Failing are Bad, Suspended is Done; the text is the `JobStatus` Display |
| CronJobs | suspended: Done "Suspended". Otherwise by `last_run`: Running is Info "Running", Failed is Warn "Last run failed", Succeeded is Ok "Last run succeeded", NeverRun is Info "Not run yet" |
| Services | a LoadBalancer with no address: Warn "Address pending"; else Ok with `service_type` |
| Ingresses | no address: Warn "No address yet"; else Ok "Address assigned" |
| ConfigMaps | Ok "{n} keys" ("1 key") |
| Namespaces | the Status cell label |

### CronJob last run (pure, `workload_rows.rs`)

```rust
enum LastRun { NeverRun, Running, Succeeded, Failed }
fn last_run(cron_job: &CronJobSummary) -> LastRun;
fn last_run_tone(cron_job: &CronJobSummary) -> Option<StatusTone>; // Running: Info, Succeeded: Ok, Failed: Bad, NeverRun: None
```

| Case (in order) | `last_run` |
|---|---|
| `last_schedule_at` is `None` | NeverRun |
| `active_jobs` is non-empty | Running |
| `last_success_at >= last_schedule_at` | Succeeded |
| otherwise | Failed |

A Job that succeeds sets `lastSuccessfulTime` after its schedule time, so a schedule newer than the last success, with no active job, means that run did not succeed.
