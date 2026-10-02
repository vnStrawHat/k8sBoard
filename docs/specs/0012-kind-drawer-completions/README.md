# 0012 — Kind drawer completions (read-only)

Status: **implemented** (steps 1a to 5); amended after advisor review (HEAD `013ab1f`), then at HEAD `81497ba` so 0013–0015 add no renames: `DetailRow::Bar`, `KindCell::Quantity.tone`, `kind_diagnosis.rs`, `cluster::Selector` (one label matcher), and `KindList.companion` are general from the start. Crates: `crates/cluster` (steps 1a–1b), `crates/app` (steps 2–5). Requires 0009 (`TableRow`, filters, `NamespaceScope::Several`, `PodSummary.labels`) and 0010 (`CpuAmount`, `ByteAmount`, `Measure`) merged; independent of 0011. Wireframes: W7 drawers and columns of the 12 live kinds. Applies C1, C6, C7, C11.

## Goal

Finish the W7 tables and drawers of the live kinds: Deployment **Revisions** and WHY box; StatefulSet **pods by ordinal with claims**; DaemonSet **rollout bars, Not ready, WHY**; Job **Attempts, BACKOFF LIMIT** box; CronJob **Next run** column, **Next runs**, **Recent jobs**; Service **Endpoints** column and list, "matches no pods"; Ingress **Open URL** and backend links; ConfigMap **Used by** column and drawer, **value previews**; Namespace **Pods / CPU req / Memory req**; ReplicaSet and Job **Go to owner**; **sidebar counts for every kind** (C11).

## Non-goals

Any mutation (Roll back, Trigger, Restart stay disabled); ControllerRevisions; PVC status (0014); TLS expiry (0016); STUCK namespaces and Quota (0013, 0018); workload logs (0019); Topology (0022); ConfigMap "changed by" (needs managedFields, 0031); core/v1 Endpoints fallback.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1a | `crates/cluster`: `cron_schedule.rs`, `CronJobSummary.timetable`, jiff tz feature, probe next run | 1, 2, 3, 4 |
| 1b | `crates/cluster`: other summary fields, `selector.rs`, `endpoint_slice.rs`, selected watches, `object_count.rs`, `ListEndpointSlices`, probe | 1, 2, 3, 5, 6 |
| 2 | App row model (`KindObject`, live rows, `NextRun`), related watch (`follow_drawer_subjects`), Deployment Revisions, CronJob Next run and Recent jobs, owner links, reveal clears filter, `open_watch_count` | 1, 2, 3, 7, 8, 10 |
| 3 | `kind_diagnosis.rs` WHY boxes; DaemonSet Rollout by node and Not ready; StatefulSet claims; Job Attempts; ReplicaSet Template and Go to owner | 1, 2, 7, 10 |
| 4a | `kind_join.rs` and join triggers; Services: endpoint-slice companion, Endpoints column, status, list, WHY | 1, 2, 7, 8, 10 |
| 4b | ConfigMaps Used by and value previews; Namespaces columns; Ingress Open URL and backend links | 1, 2, 5, 7, 9, 10 |
| 5 | Sidebar counts (C11); full ui-verifier run | 1, 2, 8, 9, 10 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale, ceilings |
| [cluster-api.md](cluster-api.md) | step 1b: new fields, `Selector`, endpoint slices, selected watches, counts, access check, probe |
| [cron-schedule.md](cron-schedule.md) | step 1a: grammar, day rule, robfig `Next` port, `@every`, time zones, display |
| [row-model.md](row-model.md) | steps 2–4b: `KindObject`, `DetailRow::Live`, new cells and columns, joins |
| [drawer-sections.md](drawer-sections.md) | steps 2–4b: per-kind section order and live content, links, Open URL |
| [workload-diagnosis.md](workload-diagnosis.md) | step 3 (+4a for Services): WHY rules |
| [session-async.md](session-async.md) | steps 2, 4a, 5: drawer subjects, endpoint companion, join triggers, counts, watch count |
| [files-to-touch.md](files-to-touch.md) | modules and Cargo changes per step, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [ ] 3. No kube or k8s-openapi type in a public signature; the crate spawns no task; the app gains no kube dependency. The 0001 read-only grep still finds only the SSAR `create`; new requests are `list`/`watch` only.
- [ ] 4. `Cargo.lock` gains exactly one package, `jiff-tzdb` (jiff feature `tzdb-bundle-always`); no cron crate.
- [ ] 5. Secret safety: ConfigMap value previews exist only in the related watch of the open drawer, are cut at 120 chars, and `config_map.rs`, `config_map_rows.rs`, `live_sections.rs` contain no `tracing::` call. List summaries stay value-free.
- [ ] 6. On UAT, the probe prints `list endpointslices` allowed or denied, an `endpointslices` watch line, and 13 `count` lines (every `ObjectKind`); results are copied into [decisions.md](decisions.md) "UAT probe".
- [ ] 7. On UAT: a Deployment drawer lists revisions newest first with the current one marked; a CronJob shows Next run and three next runs; a DaemonSet shows two bars; a Service shows endpoints with ready state; a ConfigMap shows Used by and value previews; Namespaces show Pods and request columns when the scope is All.
- [ ] 8. Watches per session stay at most `3N + 4` for N picked namespaces (`open_watch_count` test plus review of `set_explorer_kind`, `set_related_subject`).
- [ ] 9. The 0003 AC4 color-literal grep is clean; bars use the kit `Progress` and `tone_color`.
- [ ] 10. The step's screenshots exist; the ui-verifier reports no high-severity defect against W7.

## Open items

1. ConfigMap Used by misses workloads that run no pod right now (scaled to zero, CronJob history 0); a template scan needs five more list calls.
2. Deployment WHY "since rev 38" needs the pod → ReplicaSet revision join; add if users ask.
