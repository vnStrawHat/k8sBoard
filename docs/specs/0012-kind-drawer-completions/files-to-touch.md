# 0012 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; nothing lands before its first user (the probe is the crate's first user in steps 1a and 1b).

## Cargo

| S | File | Change |
|---|---|---|
| 1a | root `Cargo.toml` | `jiff` features `["std", "tzdb-bundle-always"]`; `Cargo.lock` gains `jiff-tzdb` only |

No other dependency in any step: `thiserror` and `futures` (`buffer_unordered`) are already dependencies; the kit has `Progress`, `Alert`, `Button`.

## `crates/cluster`

| S | File | Change |
|---|---|---|
| 1a | `src/cron_schedule.rs` (new) + `cron_schedule_tests.rs` | `CronSchedule`, `ScheduleError`, grammar, day rule, robfig `Next` port, `@every` |
| 1a | `src/cron_job.rs`, `src/lib.rs`, `examples/probe.rs` | `timetable`; exports; next run on the `cronjobs` line |
| 1b | `src/selector.rs` (new) + tests in module | `Selector` ([cluster-api.md](cluster-api.md)); `workload::selector_terms` delegates to it |
| 1b | `src/endpoint_slice.rs` (new) + tests in module | summaries, `watch_endpoint_slices` |
| 1b | `src/object_count.rs` (new) + tests in module | `count_objects`, `count_params`, pure `count_of(items, remaining, has_continue)` |
| 1b | `src/resource_watch.rs` | `selected_summary_watch` |
| 1b | `src/workload.rs` (+ `workload_tests.rs`) | `WorkloadCondition.message`, `ContainerPort.host_port` |
| 1b | `src/deployment.rs`, `src/job.rs`, `src/stateful_set.rs`, `src/ingress.rs` | new fields; `watch_namespace_jobs` in `job.rs` |
| 1b | `src/replica_set.rs` | `watch_selected_replica_sets` |
| 1b | `src/config_map.rs` | `ConfigMapValues`, `ValuePreview`, `watch_config_map_values`; no `tracing::` |
| 1b | `src/container_spec.rs` (+ tests) | `VolumeSource::Projected { config_maps }` |
| 1b | `src/access_review.rs`, `src/lib.rs` | `ListEndpointSlices`; exports |
| 1b | `examples/probe.rs` | `endpointslices` watch line, `--counts` |
| 1a, 1b | app compile arms | fixtures with the new fields; `container_detail.rs` matches `Projected { .. }` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/kind_row.rs` | `KindObject` (Deployment, CronJob), `DetailRow::Live`, `LiveContent` (Revisions, NextRuns, RecentJobs), `KindCell::NextRun` |
| 2 | `src/workload_rows.rs`, `src/batch_rows.rs` (+ tests) | `object`; Revisions placeholder; ReplicaSet and Job owner Links; Job deadline and TTL rows; CronJob Next run cell, Next runs and Recent jobs placeholders, Time zone text |
| 2 | `src/network_rows.rs`, `src/config_map_rows.rs`, `src/namespace_rows.rs`, `src/event_rows.rs` | `object: KindObject::Plain` |
| 2 | `src/resource_kind.rs`, `src/kind_table.rs`, `src/kind_drawer.rs` | Next run column; `NextRun` cell and value; `Live` rows; Deployment subtitle `· rev {n}` |
| 2 | `src/live_sections.rs` (new) + `live_sections_tests.rs` | Revisions, NextRuns, RecentJobs; pure `revision_rows`, `image_tag`, `recent_jobs`, `run_label` |
| 2 | `src/related_objects.rs` (new), `src/object_events.rs` | `RelatedSubject`, `related_subject`; generic `subject_change` |
| 2 | `src/cluster_session.rs` (+ tests) | `RelatedObjects`, `RelatedList`, `set_related_subject`, `related_of`; `OpenWatches` and `open_watch_count` (Several multiplicity, related; replaces the two-argument form and its test) |
| 2 | `src/table_selection.rs`, `src/table_view.rs` (+ tests) | `ResourceKey::of_owner`; `TableView::reveal(item)` clears text, chips, preset when they hide `item` |
| 2 | `src/app_shell.rs`, `src/screenshot.rs` | `follow_drawer_subjects` and `pending_subjects` (replace the events-only pair); settle; `reveal` calls `TableView::reveal` |
| 3 | `src/kind_diagnosis.rs` (new) + `kind_diagnosis_tests.rs` | rules D, S, J |
| 3 | `src/kind_row.rs` | `DetailRow::Bar`, `percent`, `LiveContent::NotReadyPods`, `KindObject::{StatefulSet, DaemonSet, ReplicaSet, Job}` |
| 3 | `src/workload_rows.rs` (+ tests) | DaemonSet "Rollout by node", Not ready placeholder, host port suffix; StatefulSet Retention; ReplicaSet Template section |
| 3 | `src/related_pods.rs` (+ tests), `src/node_drawer.rs` | `PodRowDetail::{StatusAndClaims, Attempt}`, `pod_row_detail` |
| 3 | `src/kind_drawer.rs`, `src/live_sections.rs`, `src/resource_actions.rs` (+ tests) | WHY box, `Bar` renderer, NotReadyPods; ReplicaSet Go to owner |
| 4a | `src/kind_join.rs` (new) + `kind_join_tests.rs` | `join_rows` (Services), `service_health` (matching through `cluster::Selector`), pods index |
| 4a | `src/resource_kind.rs`, `src/network_rows.rs`, `src/kind_row.rs` | Endpoints column and placeholder; `KindObject::Service`, `LiveContent::Endpoints` |
| 4a | `src/cluster_session.rs` (+ tests) | `Companion`, `CompanionLists::EndpointSlices`, `CompanionUpdate`, `companion_plan`, `companion()`, `join_explorer`, `items_mut`, `OpenWatches.companion` |
| 4a | `src/kind_diagnosis.rs`, `src/live_sections.rs` | rules V1–V2; Endpoints rows |
| 4b | `src/kind_join.rs` | `config_map_users`, `namespace_load`, their `join_rows` arms |
| 4b | `src/resource_kind.rs`, `src/config_map_rows.rs`, `src/namespace_rows.rs`, `src/network_rows.rs` (+ tests) | ConfigMaps and Namespaces columns, placeholders, `Live` sections; Ingress backend Links, `ingress_urls` |
| 4b | `src/kind_row.rs`, `src/kind_table.rs`, `src/kind_drawer.rs` | `KindCell::Quantity` (with `tone`); `KindObject::{Ingress, ConfigMap}`; `LiveContent::{UsedBy, ConfigMapData}` |
| 4b | `src/related_objects.rs`, `src/cluster_session.rs`, `src/live_sections.rs`, `src/resource_actions.rs` | `ConfigMapValues` subject; UsedBy and ConfigMapData rows; Open URL item and submenu |
| 5 | `src/cluster_session.rs` (+ tests), `src/navigation.rs`, `src/app_shell.rs` | `KindCounts`, run from `finish_access_review` and `show_screen`, `NavigationCounts.kinds` |

## Doc updates (with the last step)

- `docs/roadmap/inventory-kinds.md`: every 0012 item → Done; CronJobs, Services, ConfigMaps, Namespaces table → Done (Namespaces Partial until 0013/0018).
- `docs/roadmap/inventory-shell.md`: N3 → Done.
- `docs/roadmap/cross-cutting.md`: C6 cron row → "own robfig port + jiff `tzdb-bundle-always` (0012)"; C11 → Done (0012).
- `docs/roadmap/README.md`: status rows; `docs/specs/0005-kind-explorer/README.md` open items 1–4 → "done in 0012".
